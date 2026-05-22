use std::collections::{BTreeSet, HashMap};

use axum::http::StatusCode;
use serde_json::Value;

use crate::{
    app::AppState,
    error::AppError,
    relations::{build_relation_lookup, resolve_related_row_in_lookup},
    storage::load_resource,
};

pub(crate) fn embed_lock_resources(
    state: &AppState,
    resource: &str,
    embeds: &[String],
) -> Result<Vec<String>, AppError> {
    let mut resources = BTreeSet::new();
    resources.insert(resource.to_string());
    if !embeds.is_empty() {
        let table = state.schema_table(resource).ok_or_else(|| {
            AppError::new(
                StatusCode::BAD_REQUEST,
                "Embedding requires schema metadata with foreign key definitions",
            )
        })?;
        for embed in embeds {
            let fk = table.foreign_keys.get(embed).ok_or_else(|| {
                AppError::new(
                    StatusCode::BAD_REQUEST,
                    format!("Cannot embed '{embed}' for resource '{resource}'"),
                )
            })?;
            resources.insert(fk.target_table.clone());
        }
    }
    Ok(resources.into_iter().collect())
}

pub(crate) async fn embed_collection_data(
    state: &AppState,
    resource: &str,
    data: Value,
    embeds: &[String],
) -> Result<Value, AppError> {
    let table = state.schema_table(resource).ok_or_else(|| {
        AppError::new(
            StatusCode::BAD_REQUEST,
            "Embedding requires schema metadata with foreign key definitions",
        )
    })?;
    let items = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    let mut embedded_items = items.to_vec();
    let mut target_resources: HashMap<String, std::sync::Arc<Value>> = HashMap::new();

    for embed in embeds {
        let fk = table.foreign_keys.get(embed).ok_or_else(|| {
            AppError::new(
                StatusCode::BAD_REQUEST,
                format!("Cannot embed '{embed}' for resource '{resource}'"),
            )
        })?;
        if !target_resources.contains_key(&fk.target_table) {
            target_resources
                .insert(fk.target_table.clone(), load_resource(state, &fk.target_table).await?);
        }
        let target_items = target_resources
            .get(&fk.target_table)
            .ok_or_else(|| {
                AppError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Embedded resource '{}' is unavailable", fk.target_table),
                )
            })?
            .as_array()
            .ok_or_else(|| {
                AppError::new(
                    StatusCode::BAD_REQUEST,
                    format!("Embedded resource '{}' is not a JSON array", fk.target_table),
                )
            })?;

        let mut lookup: HashMap<String, &Value> = HashMap::new();
        lookup.extend(build_relation_lookup(target_items, &fk.target_column));

        for item in &mut embedded_items {
            let Some(object) = item.as_object_mut() else {
                continue;
            };
            let replacement =
                resolve_related_row_in_lookup(object, embed, &lookup).unwrap_or(Value::Null);
            object.insert(embed.clone(), replacement);
        }
    }
    Ok(Value::Array(embedded_items))
}
