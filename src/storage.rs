use std::{
    collections::BTreeSet,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::http::StatusCode;
use serde_json::Value;
use tokio::fs;

use crate::{
    app::{AppState, CachedMetadata, CachedResource, DataSource},
    error::AppError,
    schema::{ColumnType, TableSchema, is_valid_identifier},
};

pub async fn load_resource(state: &AppState, resource: &str) -> Result<Arc<Value>, AppError> {
    let file = resource_file_path(&state.data_source, resource)?;
    if !fs::try_exists(&file)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            format!("Resource '{resource}' not found"),
        ));
    }

    let metadata = fs::metadata(&file)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let current_metadata = CachedMetadata {
        modified: metadata.modified().map_err(|e| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read file metadata: {e}"),
            )
        })?,
        len: metadata.len(),
    };

    if let Some(value) = state
        .resource_cache
        .read()
        .await
        .get(resource)
        .and_then(|cached| (cached.metadata == current_metadata).then(|| cached.value.clone()))
    {
        return Ok(value);
    }

    let value = Arc::new(match &*state.data_source {
        DataSource::Folder(_) => {
            let raw = fs::read_to_string(&file)
                .await
                .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            serde_json::from_str::<Value>(&raw).map_err(|e| {
                AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Invalid JSON: {e}"))
            })?
        }
        DataSource::File(_) => {
            let raw = fs::read_to_string(&file)
                .await
                .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let root: Value = serde_json::from_str(&raw).map_err(|e| {
                AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Invalid JSON: {e}"))
            })?;
            root.as_object().and_then(|obj| obj.get(resource).cloned()).ok_or_else(|| {
                AppError::new(StatusCode::NOT_FOUND, format!("Resource '{resource}' not found"))
            })?
        }
    });

    state.resource_cache.write().await.insert(
        resource.to_string(),
        CachedResource { value: value.clone(), metadata: current_metadata },
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
        DataSource::File(_) => {
            let _guard = state.write_lock_for_resource("__db_file__").await;
            let raw = fs::read_to_string(&file)
                .await
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
            format!("{content}\n")
        }
    };

    let file_for_write = file.clone();
    tokio::task::spawn_blocking(move || write_json_atomically(&file_for_write, payload))
        .await
        .map_err(|e| {
        AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Atomic write task failed: {e}"))
    })??;

    let metadata = fs::metadata(&file)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let modified = metadata.modified().map_err(|e| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read file metadata: {e}"),
        )
    })?;

    state.resource_cache.write().await.insert(
        resource.to_string(),
        CachedResource {
            value: Arc::new(value.clone()),
            metadata: CachedMetadata { modified, len: metadata.len() },
        },
    );
    Ok(())
}

fn write_json_atomically(file: &Path, payload: String) -> Result<(), AppError> {
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
        if is_valid_resource_name(stem) {
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
            if is_valid_resource_name(key) {
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

pub fn find_item<'a>(items: &'a [Value], id: &str) -> Option<&'a Value> {
    items.iter().find(|item| id_matches(item, id))
}
pub fn find_item_index(items: &[Value], id: &str) -> Option<usize> {
    items.iter().position(|item| id_matches(item, id))
}
fn id_matches(item: &Value, expected: &str) -> bool {
    item.as_object().and_then(|obj| obj.get("id")).is_some_and(|id| match id {
        Value::Number(n) => n.to_string() == expected,
        Value::String(s) => s == expected,
        _ => false,
    })
}

pub fn next_numeric_id(items: &[Value]) -> i64 {
    items
        .iter()
        .filter_map(|item| item.as_object().and_then(|obj| obj.get("id")))
        .filter_map(|id| id.as_i64())
        .max()
        .map_or(1, |max| max + 1)
}

pub fn coerce_id_value(id: &str, table: Option<&TableSchema>) -> Value {
    match table.and_then(|table| table.columns.get("id")) {
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use tokio::sync::RwLock;

    use super::*;
    use crate::app::AppState;

    fn test_state(folder: PathBuf) -> AppState {
        AppState {
            data_source: Arc::new(DataSource::Folder(folder)),
            resources: Arc::new(RwLock::new(BTreeSet::new())),
            resource_cache: Arc::new(RwLock::new(HashMap::new())),
            resource_locks: Arc::new(RwLock::new(HashMap::new())),
            schema: Arc::new(None),
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
