use std::{
    collections::{BTreeSet, HashMap},
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::http::StatusCode;
use fs2::FileExt;
use serde_json::Value;
use tokio::fs;

use crate::{
    app::{AppState, CachedResource, DataSource},
    error::AppError,
    schema::{
        ColumnType, TableSchema, infer_schema_from_data_source, is_valid_identifier,
        primary_key_name,
    },
};

pub async fn load_resource(state: &AppState, resource: &str) -> Result<Arc<Value>, AppError> {
    let file = resource_file_path(&state.data_source, resource)?;

    if let Some(value) =
        state.resource_cache.read().await.get(resource).map(|cached| cached.value.clone())
    {
        return Ok(value);
    }

    if !state.resources.read().await.contains(resource) {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            format!("Resource '{resource}' not found"),
        ));
    }

    let value = Arc::new(match &*state.data_source {
        DataSource::Folder(_) => {
            let raw = fs::read_to_string(&file).await.map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => {
                    AppError::new(StatusCode::NOT_FOUND, format!("Resource '{resource}' not found"))
                }
                _ => AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            })?;
            serde_json::from_str::<Value>(&raw).map_err(|e| {
                AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Invalid JSON: {e}"))
            })?
        }
        DataSource::File(_) => {
            let raw = fs::read_to_string(&file).await.map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => {
                    AppError::new(StatusCode::NOT_FOUND, format!("Resource '{resource}' not found"))
                }
                _ => AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            })?;
            let root: Value = serde_json::from_str(&raw).map_err(|e| {
                AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Invalid JSON: {e}"))
            })?;
            root.as_object().and_then(|obj| obj.get(resource).cloned()).ok_or_else(|| {
                AppError::new(StatusCode::NOT_FOUND, format!("Resource '{resource}' not found"))
            })?
        }
    });

    let table = state.schema_table(resource);
    state.resource_cache.write().await.insert(
        resource.to_string(),
        CachedResource {
            value: value.clone(),
            id_index: build_id_index(value.as_ref(), table.as_ref()),
            primary_key: primary_key_name(table.as_ref()).to_string(),
        },
    );
    Ok(value)
}

pub async fn write_resource(
    state: &AppState,
    resource: &str,
    value: &Value,
) -> Result<(), AppError> {
    let file = resource_file_path(&state.data_source, resource)?;

    let payload = match &*state.data_source {
        DataSource::Folder(_) => {
            let content = serde_json::to_string_pretty(value)
                .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            format!("{content}\n")
        }
        DataSource::File(_) => String::new(),
    };

    let file_for_write = file.clone();
    let resource_name = resource.to_string();
    let value_for_write = value.clone();
    match &*state.data_source {
        DataSource::Folder(_) => {
            tokio::task::spawn_blocking(move || write_json_atomically(&file_for_write, payload))
                .await
                .map_err(|e| {
                    AppError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Atomic write task failed: {e}"),
                    )
                })??;
        }
        DataSource::File(_) => {
            let _guard = state.write_lock_for_resource("__db_file__").await;
            tokio::task::spawn_blocking(move || {
                write_resource_in_db_file(&file_for_write, &resource_name, &value_for_write)
            })
            .await
            .map_err(|e| {
                AppError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Atomic write task failed: {e}"),
                )
            })??;
        }
    }

    let table = state.schema_table(resource);
    state.resource_cache.write().await.insert(
        resource.to_string(),
        CachedResource {
            value: Arc::new(value.clone()),
            id_index: build_id_index(value, table.as_ref()),
            primary_key: primary_key_name(table.as_ref()).to_string(),
        },
    );
    refresh_inferred_schema(state).await?;
    state.invalidate_graphql_schema().await;
    state.emit_event("resource_changed", Some(resource.to_string()));
    state.emit_event("schema_changed", None);
    state.emit_event("overview_changed", None);
    state.health.mark_ready();
    Ok(())
}

fn write_json_atomically(file: &Path, payload: String) -> Result<(), AppError> {
    with_exclusive_file_lock(file, || write_json_atomically_unlocked(file, &payload))
}

fn write_json_atomically_unlocked(file: &Path, payload: &str) -> Result<(), AppError> {
    let temp_file = temp_file_path(file);
    let mut handle = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_file)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    handle
        .write_all(payload.as_bytes())
        .and_then(|_| handle.flush())
        .and_then(|_| handle.sync_all())
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    std::fs::rename(&temp_file, file)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(parent) = file.parent() {
        sync_parent_dir(parent)
            .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    Ok(())
}

fn write_resource_in_db_file(file: &Path, resource: &str, value: &Value) -> Result<(), AppError> {
    with_exclusive_file_lock(file, || {
        let raw = std::fs::read_to_string(file)
            .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let mut root: Value = serde_json::from_str(&raw).map_err(|e| {
            AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Invalid JSON: {e}"))
        })?;
        let root_obj = root.as_object_mut().ok_or_else(|| {
            AppError::new(StatusCode::BAD_REQUEST, "Database file must contain a JSON object")
        })?;
        root_obj.insert(resource.to_string(), value.clone());
        let content = serde_json::to_string_pretty(&root)
            .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        write_json_atomically_unlocked(file, &format!("{content}\n"))
    })
}

fn with_exclusive_file_lock<T>(
    file: &Path,
    f: impl FnOnce() -> Result<T, AppError>,
) -> Result<T, AppError> {
    let lock_path = lock_file_path(file);
    let lock_handle = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    lock_handle
        .lock_exclusive()
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let result = f();
    let _ = lock_handle.unlock();
    result
}

fn lock_file_path(file: &Path) -> PathBuf {
    let file_name = file.file_name().and_then(|name| name.to_str()).unwrap_or("resource");
    file.with_file_name(format!("{file_name}.lock"))
}

fn temp_file_path(file: &Path) -> PathBuf {
    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    let file_name = file.file_name().and_then(|name| name.to_str()).unwrap_or("resource.json");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let unique_id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    file.with_file_name(format!("{file_name}.tmp.{now}-{unique_id}"))
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path) -> std::io::Result<()> {
    std::fs::File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) -> std::io::Result<()> {
    Ok(())
}

pub fn scan_resources(data_source: &DataSource) -> Result<BTreeSet<String>, std::io::Error> {
    match data_source {
        DataSource::Folder(folder) => scan_resources_folder(folder),
        DataSource::File(file) => scan_resources_file(file),
    }
}

fn scan_resources_folder(folder: &Path) -> Result<BTreeSet<String>, std::io::Error> {
    let mut resources = BTreeSet::new();
    let entries = std::fs::read_dir(folder)?;
    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if is_valid_resource_name(stem) && !is_reserved_resource_name(stem) {
            resources.insert(stem.to_owned());
        }
    }
    Ok(resources)
}

fn scan_resources_file(file: &Path) -> Result<BTreeSet<String>, std::io::Error> {
    let mut resources = BTreeSet::new();
    if !file.exists() {
        return Ok(resources);
    }
    let raw = std::fs::read_to_string(file)?;
    let parsed: Value = serde_json::from_str(&raw).map_err(std::io::Error::other)?;
    if let Some(root) = parsed.as_object() {
        for key in root.keys() {
            if is_valid_resource_name(key) && !is_reserved_resource_name(key) {
                resources.insert(key.to_string());
            }
        }
    }
    Ok(resources)
}

pub async fn resource_exists(state: &AppState, resource: &str) -> Result<bool, AppError> {
    Ok(state.resources.read().await.contains(resource))
}

pub fn validate_resource_data(
    state: &AppState,
    resource: &str,
    data: &Value,
) -> Result<(), AppError> {
    let Some(table) = state.validation_schema_table(resource) else {
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

pub fn resource_file_path(data_source: &DataSource, resource: &str) -> Result<PathBuf, AppError> {
    if !is_valid_resource_name(resource) {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Resource name must only contain letters, numbers, underscore, and dash",
        ));
    }
    Ok(match data_source {
        DataSource::Folder(folder) => folder.join(format!("{resource}.json")),
        DataSource::File(file) => file.clone(),
    })
}

pub fn is_valid_resource_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn is_reserved_resource_name(name: &str) -> bool {
    matches!(name, "schema" | "graphql" | "sql" | "events" | "healthz" | "readyz" | "metrics")
}

pub fn find_item_by_key<'a>(items: &'a [Value], key_name: &str, id: &str) -> Option<&'a Value> {
    items.iter().find(|item| id_matches(item, key_name, id))
}
pub fn find_item_index_by_key(items: &[Value], key_name: &str, id: &str) -> Option<usize> {
    items.iter().position(|item| id_matches(item, key_name, id))
}
fn id_matches(item: &Value, key_name: &str, expected: &str) -> bool {
    item.as_object().and_then(|obj| obj.get(key_name)).is_some_and(|id| match id {
        Value::Number(n) => n.to_string() == expected,
        Value::String(s) => s == expected,
        Value::Bool(value) => value.to_string() == expected,
        _ => false,
    })
}

pub fn build_id_index(
    value: &Value,
    table: Option<&TableSchema>,
) -> Option<HashMap<String, usize>> {
    let items = value.as_array()?;
    let key_name = primary_key_name(table);
    let mut index = HashMap::with_capacity(items.len());
    let mut has_any_id = false;

    for (position, item) in items.iter().enumerate() {
        let Some(id_value) = item.as_object().and_then(|obj| obj.get(key_name)) else {
            continue;
        };
        match id_value {
            Value::Number(number) => {
                index.insert(number.to_string(), position);
                has_any_id = true;
            }
            Value::String(text) => {
                index.insert(text.clone(), position);
                has_any_id = true;
            }
            Value::Bool(value) => {
                index.insert(value.to_string(), position);
                has_any_id = true;
            }
            _ => {}
        }
    }

    has_any_id.then_some(index)
}

pub fn next_numeric_id(items: &[Value], key_name: &str) -> i64 {
    items
        .iter()
        .filter_map(|item| item.as_object().and_then(|obj| obj.get(key_name)))
        .filter_map(|id| id.as_i64())
        .max()
        .map_or(1, |max| max + 1)
}

pub fn coerce_id_value(id: &str, table: Option<&TableSchema>) -> Value {
    let key_name = primary_key_name(table);
    match table.and_then(|table| table.columns.get(key_name)) {
        Some(column) if matches!(column.column_type, ColumnType::String) => {
            Value::String(id.to_string())
        }
        _ => id.parse::<i64>().map_or_else(|_| Value::String(id.to_string()), Value::from),
    }
}

pub fn validate_sql_identifier(identifier: &str, kind: &str) -> Result<(), AppError> {
    if !is_valid_identifier(identifier) {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            format!("Invalid {kind} identifier '{identifier}'"),
        ));
    }
    Ok(())
}

async fn refresh_inferred_schema(state: &AppState) -> Result<(), AppError> {
    let resources = state.resources.read().await.clone();
    let data_source = state.data_source.clone();
    let inferred = tokio::task::spawn_blocking(move || {
        infer_schema_from_data_source(&data_source, &resources)
    })
    .await
    .map_err(|err| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Schema refresh task failed: {err}"),
        )
    })?
    .map_err(|err| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, err))?;
    state
        .update_inferred_schema(inferred)
        .map_err(|err| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, err))?;
    state.health.mark_ready();
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use tokio::sync::RwLock;

    use super::*;
    use crate::app::AppState;

    fn test_state(folder: PathBuf) -> AppState {
        AppState {
            data_source: Arc::new(DataSource::Folder(folder)),
            config: Arc::new(crate::app::AppConfig {
                readonly: false,
                enable_log: false,
                auth_token: None,
                cors_origin: None,
                max_body_bytes: 1024 * 1024,
                max_per_page: 100,
                max_sql_scan_rows: 50_000,
                max_sql_selected_rows: 1_000,
            }),
            resources: Arc::new(RwLock::new(BTreeSet::new())),
            resource_cache: Arc::new(RwLock::new(HashMap::new())),
            resource_locks: Arc::new(RwLock::new(HashMap::new())),
            schema_store: Arc::new(std::sync::RwLock::new(crate::app::SchemaStore::default())),
            graphql_store: Arc::new(RwLock::new(crate::app::GraphqlStore::default())),
            metrics: Arc::new(crate::app::MetricsStore::default()),
            health: Arc::new(crate::app::HealthState::new(true, None)),
            event_bus: tokio::sync::broadcast::channel(16).0,
        }
    }

    #[tokio::test]
    async fn write_resource_survives_interrupted_temp_file_and_keeps_output_intact() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let state = test_state(temp.path().to_path_buf());
        let resource = "users";

        let target_file = temp.path().join("users.json");
        std::fs::write(&target_file, "[{\"id\":1}]\n").expect("write initial resource");

        let stale_temp = temp.path().join("users.json.tmp.crash-simulation");
        std::fs::write(&stale_temp, "[{\"id\":").expect("write stale temp file");

        let updated_value = serde_json::json!([
            {"id": 2, "name": "Ada"},
            {"id": 3, "name": "Lin"}
        ]);
        write_resource(&state, resource, &updated_value).await.expect("atomic write succeeds");

        let final_text = std::fs::read_to_string(&target_file).expect("read final resource file");
        let parsed: Value =
            serde_json::from_str(&final_text).expect("final file should be valid json");
        assert_eq!(parsed, updated_value);

        let tmp_entries = std::fs::read_dir(temp.path())
            .expect("list directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("users.json.tmp."))
            })
            .collect::<Vec<_>>();
        assert_eq!(tmp_entries, vec![stale_temp]);
    }
}
