use std::{collections::BTreeSet, sync::Arc};

use async_graphql::{Error as GraphqlError, dynamic::FieldValue};
use axum::http::StatusCode;
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::{
    app::AppState,
    error::AppError,
    query::filters::value_to_filter_string,
    relations::{build_relation_lookup, resolve_related_row_in_lookup},
    schema::ManyToManyRelation,
    storage::{load_resource, validate_resource_data},
};

use super::{
    types::GraphqlRelationCache,
    values::{app_error_to_graphql, typed_object_value},
};

pub(crate) async fn resolve_graphql_related_row(
    state: &AppState,
    cache: &GraphqlRelationCache,
    resource: &str,
    source_object: &JsonMap<String, JsonValue>,
    source_column: &str,
) -> Result<Option<JsonValue>, AppError> {
    let table = state.schema_table(resource).ok_or_else(|| {
        AppError::new(
            StatusCode::BAD_REQUEST,
            "Relation lookup requires schema metadata with foreign key definitions",
        )
    })?;
    let fk = table.foreign_keys.get(source_column).ok_or_else(|| {
        AppError::new(
            StatusCode::BAD_REQUEST,
            format!("Cannot resolve relation '{source_column}' for resource '{resource}'"),
        )
    })?;
    let target_resource = load_cached_graphql_resource(cache, state, &fk.target_table).await?;
    let target_items = target_resource.as_array().ok_or_else(|| {
        AppError::new(
            StatusCode::BAD_REQUEST,
            format!("Embedded resource '{}' is not a JSON array", fk.target_table),
        )
    })?;
    let lookup = build_relation_lookup(target_items, &fk.target_column);
    Ok(resolve_related_row_in_lookup(source_object, source_column, &lookup))
}

pub(crate) async fn load_cached_graphql_resource(
    cache: &GraphqlRelationCache,
    state: &AppState,
    resource: &str,
) -> Result<Arc<JsonValue>, AppError> {
    if let Some(value) = cache.resources.lock().await.get(resource).cloned() {
        return Ok(value);
    }

    let value = load_resource(state, resource).await?;
    cache.resources.lock().await.insert(resource.to_string(), value.clone());
    Ok(value)
}

pub(crate) async fn resolve_graphql_many_to_many_rows(
    state: &AppState,
    cache: &GraphqlRelationCache,
    parent: &JsonMap<String, JsonValue>,
    relation: &ManyToManyRelation,
    target_type_name: &str,
) -> Result<Vec<FieldValue<'static>>, GraphqlError> {
    let source_value = match parent.get(&relation.source_target_column) {
        Some(value) if value.is_null() || value.is_object() || value.is_array() => {
            return Ok(Vec::new());
        }
        Some(value) => value_to_filter_string(value),
        None => return Ok(Vec::new()),
    };

    let through_resource = load_cached_graphql_resource(cache, state, &relation.through_table)
        .await
        .map_err(app_error_to_graphql)?;
    validate_resource_data(state, &relation.through_table, through_resource.as_ref())
        .map_err(app_error_to_graphql)?;
    let through_items = through_resource.as_array().ok_or_else(|| {
        GraphqlError::new(format!("Resource '{}' is not a JSON array", relation.through_table))
    })?;

    let mut target_ids = BTreeSet::new();
    for row in through_items {
        let Some(object) = row.as_object() else {
            continue;
        };
        let Some(candidate) = object.get(&relation.source_column) else {
            continue;
        };
        if candidate.is_null() || candidate.is_object() || candidate.is_array() {
            continue;
        }
        if value_to_filter_string(candidate) != source_value {
            continue;
        }
        let Some(target_value) = object.get(&relation.through_target_column) else {
            continue;
        };
        if target_value.is_null() || target_value.is_object() || target_value.is_array() {
            continue;
        }
        target_ids.insert(value_to_filter_string(target_value));
    }

    if target_ids.is_empty() {
        return Ok(Vec::new());
    }

    let target_resource = load_cached_graphql_resource(cache, state, &relation.target_table)
        .await
        .map_err(app_error_to_graphql)?;
    validate_resource_data(state, &relation.target_table, target_resource.as_ref())
        .map_err(app_error_to_graphql)?;
    let target_items = target_resource.as_array().ok_or_else(|| {
        GraphqlError::new(format!("Resource '{}' is not a JSON array", relation.target_table))
    })?;

    let values = target_items
        .iter()
        .filter_map(|item| item.as_object())
        .filter_map(|object| {
            let id = object.get(&relation.target_column)?;
            (!id.is_null()
                && !id.is_object()
                && !id.is_array()
                && target_ids.contains(&value_to_filter_string(id)))
            .then(|| typed_object_value(target_type_name, object.clone()))
        })
        .collect::<Vec<_>>();

    Ok(values)
}
