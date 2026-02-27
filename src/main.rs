use std::{
    collections::BTreeSet,
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use clap::Parser;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::Mutex;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Serve all JSON files in a folder as a REST API"
)]
struct Cli {
    /// Folder containing .json files
    #[arg(short, long, default_value = "./data")]
    folder: PathBuf,

    /// Bind address, e.g. 127.0.0.1:3000
    #[arg(short, long, default_value = "127.0.0.1:3000")]
    bind: SocketAddr,

    /// Enable read-only mode (only GET endpoints are exposed)
    #[arg(long)]
    readonly: bool,
}

#[derive(Clone)]
struct AppState {
    folder: Arc<PathBuf>,
    resources: Arc<RwLock<BTreeSet<String>>>,
    io_lock: Arc<Mutex<()>>,
}

#[derive(Debug)]
struct AppError {
    status: StatusCode,
    message: String,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl AppError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    if let Err(err) = fs::create_dir_all(&cli.folder) {
        eprintln!(
            "Failed to create data folder {}: {err}",
            cli.folder.display()
        );
        std::process::exit(1);
    }

    let initial_resources = scan_resources(&cli.folder).unwrap_or_else(|err| {
        eprintln!("Failed to scan data folder {}: {err}", cli.folder.display());
        BTreeSet::new()
    });

    let state = AppState {
        folder: Arc::new(cli.folder),
        resources: Arc::new(RwLock::new(initial_resources)),
        io_lock: Arc::new(Mutex::new(())),
    };

    start_resource_watcher(state.folder.clone(), state.resources.clone());

    let app = if cli.readonly {
        Router::new()
            .route("/", get(list_resources))
            .route("/{resource}", get(get_collection))
            .route("/{resource}/{id}", get(get_item))
            .with_state(state)
    } else {
        Router::new()
            .route("/", get(list_resources))
            .route("/{resource}", get(get_collection).post(create_item))
            .route(
                "/{resource}/{id}",
                get(get_item)
                    .put(replace_item)
                    .patch(patch_item)
                    .delete(delete_item),
            )
            .with_state(state)
    };

    tracing::info!(readonly = cli.readonly, "Readonly mode");
    tracing::info!("Listening on http://{}", cli.bind);
    let listener = tokio::net::TcpListener::bind(cli.bind)
        .await
        .expect("binding server listener");
    axum::serve(listener, app).await.expect("running server");
}

async fn list_resources(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let resources = state
        .resources
        .read()
        .map_err(|_| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Resource cache lock poisoned",
            )
        })?
        .iter()
        .cloned()
        .collect::<Vec<_>>();

    Ok(Json(serde_json::json!({ "resources": resources })))
}

async fn get_collection(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.io_lock.lock().await;
    let data = load_resource(&state.folder, &resource)?;
    Ok(Json(data))
}

async fn create_item(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Json(mut payload): Json<Value>,
) -> Result<impl IntoResponse, AppError> {
    let _guard = state.io_lock.lock().await;

    let mut data = load_resource(&state.folder, &resource)?;
    let array = data
        .as_array_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    let item = payload
        .as_object_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Payload must be a JSON object"))?;

    if !item.contains_key("id") {
        let next_id = next_numeric_id(array);
        item.insert("id".to_string(), Value::from(next_id));
    }

    let created = Value::Object(item.clone());
    array.push(created.clone());
    write_resource(&state.folder, &resource, &data)?;

    Ok((StatusCode::CREATED, Json(created)))
}

async fn get_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.io_lock.lock().await;
    let data = load_resource(&state.folder, &resource)?;
    let array = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    let item = find_item(array, &id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?;

    Ok(Json(item.clone()))
}

async fn replace_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
    Json(mut payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.io_lock.lock().await;
    let mut data = load_resource(&state.folder, &resource)?;
    let array = data
        .as_array_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    let object = payload
        .as_object_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Payload must be a JSON object"))?;
    object.insert("id".to_string(), coerce_id_value(&id));

    let replacement = Value::Object(object.clone());
    let position = find_item_index(array, &id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?;
    array[position] = replacement.clone();

    write_resource(&state.folder, &resource, &data)?;
    Ok(Json(replacement))
}

async fn patch_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.io_lock.lock().await;
    let mut data = load_resource(&state.folder, &resource)?;
    let array = data
        .as_array_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    let patch = payload
        .as_object()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Payload must be a JSON object"))?;

    let index = find_item_index(array, &id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?;
    let current = array[index].as_object_mut().ok_or_else(|| {
        AppError::new(StatusCode::BAD_REQUEST, "Array item must be a JSON object")
    })?;

    for (key, value) in patch {
        if key != "id" {
            current.insert(key.clone(), value.clone());
        }
    }

    let updated = Value::Object(current.clone());
    write_resource(&state.folder, &resource, &data)?;
    Ok(Json(updated))
}

async fn delete_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
) -> Result<StatusCode, AppError> {
    let _guard = state.io_lock.lock().await;
    let mut data = load_resource(&state.folder, &resource)?;
    let array = data
        .as_array_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    let index = find_item_index(array, &id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?;
    array.remove(index);

    write_resource(&state.folder, &resource, &data)?;
    Ok(StatusCode::NO_CONTENT)
}

fn load_resource(folder: &Path, resource: &str) -> Result<Value, AppError> {
    let file = resource_file_path(folder, resource)?;
    if !file.exists() {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            format!("Resource '{resource}' not found"),
        ));
    }

    let raw = fs::read_to_string(&file)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    serde_json::from_str::<Value>(&raw).map_err(|e| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Invalid JSON: {e}"),
        )
    })
}

fn write_resource(folder: &Path, resource: &str, value: &Value) -> Result<(), AppError> {
    let file = resource_file_path(folder, resource)?;
    let content = serde_json::to_string_pretty(value)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    fs::write(file, format!("{content}\n"))
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn resource_file_path(folder: &Path, resource: &str) -> Result<PathBuf, AppError> {
    if !is_valid_resource_name(resource) {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Resource name must only contain letters, numbers, underscore, and dash",
        ));
    }

    Ok(folder.join(format!("{resource}.json")))
}

fn is_valid_resource_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn find_item<'a>(items: &'a [Value], id: &str) -> Option<&'a Value> {
    items.iter().find(|item| id_matches(item, id))
}

fn find_item_index(items: &[Value], id: &str) -> Option<usize> {
    items.iter().position(|item| id_matches(item, id))
}

fn id_matches(item: &Value, expected: &str) -> bool {
    item.as_object()
        .and_then(|obj| obj.get("id"))
        .is_some_and(|id| match id {
            Value::Number(n) => n.to_string() == expected,
            Value::String(s) => s == expected,
            _ => false,
        })
}

fn next_numeric_id(items: &[Value]) -> i64 {
    items
        .iter()
        .filter_map(|item| item.as_object().and_then(|obj| obj.get("id")))
        .filter_map(|id| id.as_i64())
        .max()
        .map_or(1, |max| max + 1)
}

fn coerce_id_value(id: &str) -> Value {
    id.parse::<i64>()
        .map_or_else(|_| Value::String(id.to_string()), Value::from)
}

fn scan_resources(folder: &Path) -> Result<BTreeSet<String>, std::io::Error> {
    let mut resources = BTreeSet::new();
    let entries = fs::read_dir(folder)?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            && is_valid_resource_name(stem)
        {
            resources.insert(stem.to_owned());
        }
    }

    Ok(resources)
}

fn start_resource_watcher(folder: Arc<PathBuf>, resources: Arc<RwLock<BTreeSet<String>>>) {
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();

        let mut watcher = match RecommendedWatcher::new(
            move |result| {
                let _ = tx.send(result);
            },
            Config::default(),
        ) {
            Ok(watcher) => watcher,
            Err(err) => {
                tracing::error!("Failed to create filesystem watcher: {err}");
                return;
            }
        };

        if let Err(err) = watcher.watch(&folder, RecursiveMode::NonRecursive) {
            tracing::error!("Failed to watch folder {}: {err}", folder.display());
            return;
        }

        for event in rx {
            match event {
                Ok(_) => match scan_resources(&folder) {
                    Ok(new_resources) => {
                        if let Ok(mut cache) = resources.write() {
                            *cache = new_resources;
                        }
                    }
                    Err(err) => tracing::error!(
                        "Failed to refresh resources for folder {}: {err}",
                        folder.display()
                    ),
                },
                Err(err) => tracing::warn!("File watch event error: {err}"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_resource_names() {
        assert!(is_valid_resource_name("users"));
        assert!(is_valid_resource_name("blog_posts-2025"));
        assert!(!is_valid_resource_name(""));
        assert!(!is_valid_resource_name("../evil"));
        assert!(!is_valid_resource_name("with space"));
    }

    #[test]
    fn finds_next_numeric_id() {
        let items = serde_json::json!([
            {"id": 1, "name": "a"},
            {"id": 5, "name": "b"},
            {"id": "abc", "name": "c"}
        ]);

        assert_eq!(next_numeric_id(items.as_array().expect("array")), 6);
    }

    #[test]
    fn writes_and_reads_resource_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let value = serde_json::json!([{"id": 1, "name": "example"}]);

        write_resource(temp.path(), "users", &value).expect("write resource");
        let loaded = load_resource(temp.path(), "users").expect("load resource");

        assert_eq!(value, loaded);
    }

    #[test]
    fn scans_only_valid_json_resource_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(temp.path().join("users.json"), "[]").expect("write users");
        fs::write(temp.path().join("posts.json"), "[]").expect("write posts");
        fs::write(temp.path().join("notes.txt"), "hello").expect("write txt");
        fs::write(temp.path().join("bad name.json"), "[]").expect("write invalid");

        let resources = scan_resources(temp.path()).expect("scan resources");

        assert_eq!(
            resources.into_iter().collect::<Vec<_>>(),
            vec!["posts".to_string(), "users".to_string()]
        );
    }
}
