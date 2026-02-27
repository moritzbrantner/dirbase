use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use clap::Parser;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::Mutex;

mod schema;

use schema::{ColumnType, Schema, TableSchema, load_schema};

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

    /// Optional DBML schema file. If omitted, {folder}/schema.dbml is used when present.
    #[arg(long)]
    schema: Option<PathBuf>,

    /// Enable request logging to a file.
    #[arg(long)]
    log: bool,

    /// Log file name/path. Defaults to requests.log in current directory.
    #[arg(long, default_value = "requests.log")]
    logname: PathBuf,
}

#[derive(Clone)]
struct AppState {
    folder: Arc<PathBuf>,
    resources: Arc<RwLock<BTreeSet<String>>>,
    io_lock: Arc<Mutex<()>>,
    schema: Arc<Option<Schema>>,
    request_log: Option<Arc<StdMutex<fs::File>>>,
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

    let schema = match load_schema(&cli.folder, cli.schema.as_deref()) {
        Ok(schema) => schema,
        Err(err) => {
            eprintln!("Failed to load schema: {err}");
            std::process::exit(1);
        }
    };

    let initial_resources = scan_resources(&cli.folder).unwrap_or_else(|err| {
        eprintln!("Failed to scan data folder {}: {err}", cli.folder.display());
        BTreeSet::new()
    });

    let state = AppState {
        folder: Arc::new(cli.folder),
        resources: Arc::new(RwLock::new(initial_resources)),
        io_lock: Arc::new(Mutex::new(())),
        schema: Arc::new(schema),
        request_log: if cli.log {
            match fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&cli.logname)
            {
                Ok(file) => Some(Arc::new(StdMutex::new(file))),
                Err(err) => {
                    eprintln!("Failed to open log file {}: {err}", cli.logname.display());
                    std::process::exit(1);
                }
            }
        } else {
            None
        },
    };

    start_resource_watcher(state.folder.clone(), state.resources.clone());

    let app_state = state.clone();
    let app = if cli.readonly {
        Router::new()
            .route("/", get(list_resources))
            .route("/{resource}", get(get_collection))
            .route("/{resource}/{id}", get(get_item))
            .with_state(app_state)
    } else {
        Router::new()
            .route("/", get(list_resources))
            .route(
                "/{resource}",
                get(get_collection)
                    .post(create_item)
                    .put(replace_resource_object)
                    .patch(patch_resource_object),
            )
            .route(
                "/{resource}/{id}",
                get(get_item)
                    .put(replace_item)
                    .patch(patch_item)
                    .delete(delete_item),
            )
            .with_state(app_state)
    };

    let app = if cli.log {
        app.layer(middleware::from_fn_with_state(
            state.clone(),
            log_requests_middleware,
        ))
    } else {
        app
    };

    tracing::info!(readonly = cli.readonly, "Readonly mode");
    tracing::info!("Listening on http://{}", cli.bind);
    let listener = tokio::net::TcpListener::bind(cli.bind)
        .await
        .expect("binding server listener");
    axum::serve(listener, app).await.expect("running server");
}

async fn log_requests_middleware(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let response = next.run(request).await;
    let status = response.status();

    if let Some(log_file) = &state.request_log {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let line = format!("{timestamp} {method} {path} {}\n", status.as_u16());
        if let Ok(mut file) = log_file.lock() {
            let _ = file.write_all(line.as_bytes());
        }
    }

    response
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
    Query(filters): Query<Vec<(String, String)>>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.io_lock.lock().await;
    let data = load_resource(&state.folder, &resource)?;
    validate_resource_data(&state, &resource, &data)?;

    if filters.is_empty() {
        return Ok(Json(data));
    }

    let filtered = filter_collection_data(data, &filters)?;
    Ok(Json(filtered))
}

fn filter_collection_data(data: Value, filters: &[(String, String)]) -> Result<Value, AppError> {
    let items = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    let filtered = items
        .iter()
        .filter(|item| item_matches_filters(item, filters))
        .cloned()
        .collect::<Vec<_>>();

    Ok(Value::Array(filtered))
}

fn item_matches_filters(item: &Value, filters: &[(String, String)]) -> bool {
    let Some(object) = item.as_object() else {
        return false;
    };

    filters.iter().all(|(key, expected)| {
        object
            .get(key)
            .is_some_and(|value| value_to_filter_string(value) == *expected)
    })
}

fn value_to_filter_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(boolean) => boolean.to_string(),
        Value::Null => "null".to_string(),
        _ => value.to_string(),
    }
}

async fn create_item(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Json(mut payload): Json<Value>,
) -> Result<impl IntoResponse, AppError> {
    let _guard = state.io_lock.lock().await;

    let mut data = load_resource(&state.folder, &resource)?;
    validate_resource_data(&state, &resource, &data)?;
    let array = data
        .as_array_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    let item = payload
        .as_object_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Payload must be a JSON object"))?;

    maybe_fill_missing_id(item, array, state.schema_table(&resource)?)?;

    let created = Value::Object(item.clone());
    array.push(created.clone());
    validate_resource_data(&state, &resource, &data)?;
    write_resource(&state.folder, &resource, &data)?;

    Ok((StatusCode::CREATED, Json(created)))
}

async fn get_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.io_lock.lock().await;
    let data = load_resource(&state.folder, &resource)?;
    validate_resource_data(&state, &resource, &data)?;
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
    object.insert(
        "id".to_string(),
        coerce_id_value(&id, state.schema_table(&resource)?),
    );

    let replacement = Value::Object(object.clone());
    let position = find_item_index(array, &id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?;
    array[position] = replacement.clone();

    validate_resource_data(&state, &resource, &data)?;
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
    validate_resource_data(&state, &resource, &data)?;
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
    validate_resource_data(&state, &resource, &data)?;
    write_resource(&state.folder, &resource, &data)?;
    Ok(Json(updated))
}

async fn delete_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
) -> Result<StatusCode, AppError> {
    let _guard = state.io_lock.lock().await;
    let mut data = load_resource(&state.folder, &resource)?;
    validate_resource_data(&state, &resource, &data)?;
    let array = data
        .as_array_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    let index = find_item_index(array, &id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?;
    array.remove(index);

    validate_resource_data(&state, &resource, &data)?;
    write_resource(&state.folder, &resource, &data)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn replace_resource_object(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.io_lock.lock().await;
    let mut data = load_resource(&state.folder, &resource)?;
    if !data.is_object() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Resource is not a JSON object",
        ));
    }

    if !payload.is_object() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Payload must be a JSON object",
        ));
    }

    data = payload;
    write_resource(&state.folder, &resource, &data)?;
    Ok(Json(data))
}

async fn patch_resource_object(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.io_lock.lock().await;
    let mut data = load_resource(&state.folder, &resource)?;
    let current = data
        .as_object_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON object"))?;
    let patch = payload
        .as_object()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Payload must be a JSON object"))?;

    for (key, value) in patch {
        current.insert(key.clone(), value.clone());
    }

    let updated = Value::Object(current.clone());
    write_resource(&state.folder, &resource, &data)?;
    Ok(Json(updated))
}

impl AppState {
    fn schema_table(&self, resource: &str) -> Result<Option<&TableSchema>, AppError> {
        let Some(schema) = self.schema.as_ref() else {
            return Ok(None);
        };

        schema.tables.get(resource).map(Some).ok_or_else(|| {
            AppError::new(
                StatusCode::BAD_REQUEST,
                format!("Resource '{resource}' is not defined in schema"),
            )
        })
    }
}

fn validate_resource_data(state: &AppState, resource: &str, data: &Value) -> Result<(), AppError> {
    let Some(table) = state.schema_table(resource)? else {
        return Ok(());
    };

    let array = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    for (index, item) in array.iter().enumerate() {
        let object = item.as_object().ok_or_else(|| {
            AppError::new(
                StatusCode::BAD_REQUEST,
                format!("Row {index} in resource '{resource}' is not an object"),
            )
        })?;

        for key in object.keys() {
            if !table.columns.contains_key(key) {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    format!("Row {index} in resource '{resource}' contains unknown column '{key}'"),
                ));
            }
        }

        for (column_name, column) in &table.columns {
            match object.get(column_name) {
                Some(Value::Null) if !column.nullable => {
                    return Err(AppError::new(
                        StatusCode::BAD_REQUEST,
                        format!(
                            "Row {index} in resource '{resource}' has null for non-null column '{column_name}'"
                        ),
                    ));
                }
                Some(value) if !value_matches_type(value, &column.column_type) => {
                    return Err(AppError::new(
                        StatusCode::BAD_REQUEST,
                        format!(
                            "Row {index} in resource '{resource}' has invalid type for '{column_name}'"
                        ),
                    ));
                }
                None if !column.nullable => {
                    return Err(AppError::new(
                        StatusCode::BAD_REQUEST,
                        format!(
                            "Row {index} in resource '{resource}' is missing non-null column '{column_name}'"
                        ),
                    ));
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn value_matches_type(value: &Value, column_type: &ColumnType) -> bool {
    if value.is_null() {
        return true;
    }

    match column_type {
        ColumnType::Integer => value.as_i64().is_some() || value.as_u64().is_some(),
        ColumnType::Float => value.is_number(),
        ColumnType::Boolean => value.is_boolean(),
        ColumnType::String => value.is_string(),
        ColumnType::Json => true,
    }
}

fn maybe_fill_missing_id(
    item: &mut serde_json::Map<String, Value>,
    array: &[Value],
    table: Option<&TableSchema>,
) -> Result<(), AppError> {
    if item.contains_key("id") {
        return Ok(());
    }

    let id_column = table.and_then(|table| table.columns.get("id"));
    if let Some(column) = id_column {
        if matches!(column.column_type, ColumnType::Integer | ColumnType::Float) {
            item.insert("id".to_string(), Value::from(next_numeric_id(array)));
            return Ok(());
        }

        if !column.nullable {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                "Payload is missing required non-numeric id column",
            ));
        }

        return Ok(());
    }

    item.insert("id".to_string(), Value::from(next_numeric_id(array)));
    Ok(())
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

fn coerce_id_value(id: &str, table: Option<&TableSchema>) -> Value {
    match table.and_then(|table| table.columns.get("id")) {
        Some(column) if matches!(column.column_type, ColumnType::String) => {
            Value::String(id.to_string())
        }
        _ => id
            .parse::<i64>()
            .map_or_else(|_| Value::String(id.to_string()), Value::from),
    }
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
    use crate::schema::parse_dbml_schema;

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

    #[test]
    fn validates_rows_against_schema() {
        let schema = parse_dbml_schema(
            r#"
            Table users {
              id int [pk]
              name varchar [not null]
              active bool
            }
            "#,
        )
        .expect("parse schema");

        let state = AppState {
            folder: Arc::new(PathBuf::from(".")),
            resources: Arc::new(RwLock::new(BTreeSet::new())),
            io_lock: Arc::new(Mutex::new(())),
            schema: Arc::new(Some(schema)),
            request_log: None,
        };

        let ok = serde_json::json!([{"id": 1, "name": "Ada", "active": true}]);
        assert!(validate_resource_data(&state, "users", &ok).is_ok());

        let wrong_type = serde_json::json!([{"id": "oops", "name": "Ada"}]);
        assert!(validate_resource_data(&state, "users", &wrong_type).is_err());

        let unknown_col = serde_json::json!([{"id": 1, "name": "Ada", "role": "admin"}]);
        assert!(validate_resource_data(&state, "users", &unknown_col).is_err());
    }

    #[test]
    fn filters_collection_items_with_multiple_query_params() {
        let data = serde_json::json!([
            {"id": 1, "role": "admin", "active": true},
            {"id": 2, "role": "admin", "active": false},
            {"id": 3, "role": "member", "active": true}
        ]);

        let filtered = filter_collection_data(
            data,
            &[
                ("role".to_string(), "admin".to_string()),
                ("active".to_string(), "true".to_string()),
            ],
        )
        .expect("filter collection");

        assert_eq!(
            filtered,
            serde_json::json!([{"id": 1, "role": "admin", "active": true}])
        );
    }
}
