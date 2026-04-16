use std::collections::HashMap;

use serde_json::{Map, Value};

use crate::{
    app::AppState, error::AppError, query::filters::value_to_filter_string, storage::load_resource,
};

pub fn build_relation_lookup<'a>(
    target_items: &'a [Value],
    target_column: &str,
) -> HashMap<String, &'a Value> {
    let mut lookup = HashMap::new();
    for item in target_items {
        if let Some((_, key)) =
            item.as_object().and_then(|object| object.get(target_column).map(|key| (object, key)))
        {
            lookup.insert(value_to_filter_string(key), item);
        }
    }
    lookup
}

pub fn resolve_related_row_in_lookup(
    source_object: &Map<String, Value>,
    source_column: &str,
    lookup: &HashMap<String, &Value>,
) -> Option<Value> {
    let current_value = source_object.get(source_column)?;
    if current_value.is_object() || current_value.is_null() {
        return None;
    }

    let key = value_to_filter_string(current_value);
    lookup.get(&key).map(|row| (*row).clone())
}

pub async fn resolve_related_row(
    state: &AppState,
    resource: &str,
    source_object: &Map<String, Value>,
    source_column: &str,
) -> Result<Option<Value>, AppError> {
    let table = state.schema_table(resource).ok_or_else(|| {
        AppError::new(
            axum::http::StatusCode::BAD_REQUEST,
            "Relation lookup requires schema metadata with foreign key definitions",
        )
    })?;
    let fk = table.foreign_keys.get(source_column).ok_or_else(|| {
        AppError::new(
            axum::http::StatusCode::BAD_REQUEST,
            format!("Cannot resolve relation '{source_column}' for resource '{resource}'"),
        )
    })?;
    let target_resource = load_resource(state, &fk.target_table).await?;
    let target_items = target_resource.as_array().ok_or_else(|| {
        AppError::new(
            axum::http::StatusCode::BAD_REQUEST,
            format!("Embedded resource '{}' is not a JSON array", fk.target_table),
        )
    })?;
    let lookup = build_relation_lookup(target_items, &fk.target_column);
    Ok(resolve_related_row_in_lookup(source_object, source_column, &lookup))
}
