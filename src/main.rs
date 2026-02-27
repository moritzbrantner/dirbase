use std::{
    fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use clap::Parser;
use serde::Deserialize;
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
    #[arg(short, long)]
    folder: Option<PathBuf>,

    /// Bind address, e.g. 127.0.0.1:3000
    #[arg(short, long)]
    bind: Option<SocketAddr>,
}

#[derive(Debug, Deserialize)]
struct FileConfig {
    folder: Option<PathBuf>,
    bind: Option<SocketAddr>,
}

#[derive(Debug)]
struct ResolvedConfig {
    folder: PathBuf,
    bind: SocketAddr,
}

#[derive(Clone)]
struct AppState {
    folder: Arc<PathBuf>,
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
    let config = resolve_config(cli);

    if let Err(err) = fs::create_dir_all(&config.folder) {
        eprintln!(
            "Failed to create data folder {}: {err}",
            config.folder.display()
        );
        std::process::exit(1);
    }

    let state = AppState {
        folder: Arc::new(config.folder.clone()),
        io_lock: Arc::new(Mutex::new(())),
    };

    let app = Router::new()
        .route("/", get(list_resources))
        .route("/{resource}", get(get_collection).post(create_item))
        .route(
            "/{resource}/{id}",
            get(get_item)
                .put(replace_item)
                .patch(patch_item)
                .delete(delete_item),
        )
        .with_state(state);

    tracing::info!("Listening on http://{}", config.bind);
    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .expect("binding server listener");
    axum::serve(listener, app).await.expect("running server");
}

async fn list_resources(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let _guard = state.io_lock.lock().await;
    let mut resources = Vec::new();

    let entries = fs::read_dir(&*state.folder)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    for entry in entries {
        let entry =
            entry.map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            resources.push(stem.to_owned());
        }
    }

    resources.sort();
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

fn resolve_config(cli: Cli) -> ResolvedConfig {
    let file_config = load_config_file(Path::new("folder-server.toml"));

    ResolvedConfig {
        folder: cli
            .folder
            .or_else(|| file_config.as_ref().and_then(|cfg| cfg.folder.clone()))
            .unwrap_or_else(|| PathBuf::from("./data")),
        bind: cli
            .bind
            .or_else(|| file_config.as_ref().and_then(|cfg| cfg.bind))
            .unwrap_or_else(default_bind),
    }
}

fn load_config_file(path: &Path) -> Option<FileConfig> {
    if !path.exists() {
        return None;
    }

    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) => {
            eprintln!("Failed to read config file {}: {err}", path.display());
            return None;
        }
    };

    match parse_simple_config(&raw) {
        Ok(config) => Some(config),
        Err(err) => {
            eprintln!("Failed to parse config file {}: {err}", path.display());
            None
        }
    }
}

fn parse_simple_config(raw: &str) -> Result<FileConfig, String> {
    let mut config = FileConfig {
        folder: None,
        bind: None,
    };

    for (line_number, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let (key, value) = trimmed
            .split_once('=')
            .ok_or_else(|| format!("line {} is missing '='", line_number + 1))?;

        let key = key.trim();
        let value = value.trim().trim_matches('"');

        match key {
            "folder" => config.folder = Some(PathBuf::from(value)),
            "bind" => {
                config.bind = Some(value.parse::<SocketAddr>().map_err(|e| {
                    format!("line {} has invalid bind address: {e}", line_number + 1)
                })?)
            }
            _ => {}
        }
    }

    Ok(config)
}

fn default_bind() -> SocketAddr {
    "127.0.0.1:3000"
        .parse()
        .expect("parsing default bind address")
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
    fn resolves_values_from_config_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let previous = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(temp.path()).expect("set cwd");

        fs::write(
            "folder-server.toml",
            r#"folder = "./seed"
bind = "127.0.0.1:4100"
"#,
        )
        .expect("write config");

        let resolved = resolve_config(Cli {
            folder: None,
            bind: None,
        });

        std::env::set_current_dir(previous).expect("restore cwd");

        assert_eq!(resolved.folder, PathBuf::from("./seed"));
        assert_eq!(resolved.bind, "127.0.0.1:4100".parse().expect("socket"));
    }

    #[test]
    fn parses_simple_config() {
        let config = parse_simple_config(
            r#"folder = "./data"
# comment
bind = "127.0.0.1:3001"
"#,
        )
        .expect("parse config");

        assert_eq!(config.folder, Some(PathBuf::from("./data")));
        assert_eq!(
            config.bind,
            Some("127.0.0.1:3001".parse().expect("socket address"))
        );
    }

    #[test]
    fn cli_arguments_override_config_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let previous = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(temp.path()).expect("set cwd");

        fs::write(
            "folder-server.toml",
            r#"folder = "./seed"
bind = "127.0.0.1:4100"
"#,
        )
        .expect("write config");

        let resolved = resolve_config(Cli {
            folder: Some(PathBuf::from("./cli")),
            bind: Some("127.0.0.1:4200".parse().expect("socket")),
        });

        std::env::set_current_dir(previous).expect("restore cwd");

        assert_eq!(resolved.folder, PathBuf::from("./cli"));
        assert_eq!(resolved.bind, "127.0.0.1:4200".parse().expect("socket"));
    }
}
