use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::http::StatusCode;
use serde_json::Value;
use tokio::fs;

use crate::{
    app::{AppState, CachedMetadata, CachedResource},
    error::AppError,
    schema::{ColumnType, TableSchema, is_valid_identifier},
};

pub async fn load_resource(state: &AppState, resource: &str) -> Result<Arc<Value>, AppError> {
    let file = resource_file_path(&state.folder, resource)?;
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

    let raw = fs::read_to_string(&file)
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let value = Arc::new(serde_json::from_str::<Value>(&raw).map_err(|e| {
        AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Invalid JSON: {e}"))
    })?);

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
    let file = resource_file_path(&state.folder, resource)?;
    let content = serde_json::to_string_pretty(value)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    fs::write(&file, format!("{content}\n"))
        .await
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

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

pub fn scan_resources(folder: &Path) -> Result<BTreeSet<String>, std::io::Error> {
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

pub fn resource_file_path(folder: &Path, resource: &str) -> Result<PathBuf, AppError> {
    if !is_valid_resource_name(resource) {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Resource name must only contain letters, numbers, underscore, and dash",
        ));
    }
    Ok(folder.join(format!("{resource}.json")))
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
