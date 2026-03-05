use std::{
    collections::{BTreeSet, HashMap},
    hash::{Hash, Hasher},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use serde_json::Value;

use crate::{
    app::AppState,
    error::AppError,
    query::filters::{
        filter_collection_data, paginate_collection_data, parse_collection_query_params,
        sort_collection_data, value_to_filter_string,
    },
    schema::TableSchema,
    sql::{export_sql, sql_query, sql_query_post},
    storage::{
        coerce_id_value, find_item, find_item_index, load_resource, next_numeric_id,
        validate_resource_data, write_resource,
    },
};

pub fn build_router(state: AppState, readonly: bool, enable_log: bool) -> Router {
    let app = if readonly {
        Router::new()
            .route("/", get(list_resources))
            .route("/sql", get(sql_query))
            .route("/export.sql", get(export_sql))
            .route("/sql/export", get(export_sql))
            .route("/{resource}", get(get_collection))
            .route("/{resource}/{id}", get(get_item))
            .with_state(state.clone())
    } else {
        Router::new()
            .route("/", get(list_resources))
            .route("/sql", get(sql_query).post(sql_query_post))
            .route("/export.sql", get(export_sql))
            .route("/sql/export", get(export_sql))
            .route(
                "/{resource}",
                get(get_collection)
                    .post(create_item)
                    .put(replace_resource_object)
                    .patch(patch_resource_object),
            )
            .route(
                "/{resource}/{id}",
                get(get_item).put(replace_item).patch(patch_item).delete(delete_item),
            )
            .with_state(state.clone())
    };

    if enable_log {
        app.layer(middleware::from_fn_with_state(state, log_requests_middleware))
    } else {
        app
    }
}

pub async fn log_requests_middleware(
    State(_state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let query_hash = request_query_hash(request.uri().path(), request.uri().query());
    let response = next.run(request).await;
    let status = response.status();

    let timestamp =
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
    let suffix = query_hash.map(|hash| format!(" query_hash={hash}")).unwrap_or_default();
    let line = format!("{timestamp} {method} {path} {}{suffix}", status.as_u16());
    tracing::info!(target: "folder_server::request", "{line}");
    response
}

pub async fn list_resources(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let resources = state.resources.read().await.iter().cloned().collect::<Vec<_>>();
    Ok(Json(serde_json::json!({"resources": resources})))
}

pub async fn get_collection(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Query(query_params): Query<Vec<(String, String)>>,
) -> Result<Json<Value>, AppError> {
    let parsed = parse_collection_query_params(query_params)?;
    let lock_resources = embed_lock_resources(&state, &resource, &parsed.embeds)?;
    let _guards = state.read_locks_for_resources(&lock_resources).await;

    let data = load_resource(&state, &resource).await?;
    let data = data.as_ref().clone();
    validate_resource_data(&state, &resource, &data)?;

    let filtered = if parsed.filters.is_empty() {
        data
    } else {
        filter_collection_data(data, &parsed.filters, None)?
    };
    let sorted = if parsed.sort_columns.is_empty() {
        filtered
    } else {
        sort_collection_data(filtered, &parsed.sort_columns)?
    };
    let embedded = if parsed.embeds.is_empty() {
        sorted
    } else {
        embed_collection_data(&state, &resource, sorted, &parsed.embeds).await?
    };

    if let Some(pagination) = parsed.pagination {
        return Ok(Json(paginate_collection_data(embedded, pagination)?));
    }
    Ok(Json(embedded))
}

pub async fn create_item(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Json(mut payload): Json<Value>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let _guard = state.write_lock_for_resource(&resource).await;
    let mut data = load_resource(&state, &resource).await?.as_ref().clone();
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
    write_resource(&state, &resource, &data).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

pub async fn get_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.read_lock_for_resource(&resource).await;
    let data = load_resource(&state, &resource).await?;
    let data = data.as_ref().clone();
    validate_resource_data(&state, &resource, &data)?;
    let array = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    Ok(Json(
        find_item(array, &id)
            .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?
            .clone(),
    ))
}

pub async fn replace_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
    Json(mut payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.write_lock_for_resource(&resource).await;
    let mut data = load_resource(&state, &resource).await?.as_ref().clone();
    let array = data
        .as_array_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    let object = payload
        .as_object_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Payload must be a JSON object"))?;
    object.insert("id".to_string(), coerce_id_value(&id, state.schema_table(&resource)?));
    let replacement = Value::Object(object.clone());
    let position = find_item_index(array, &id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?;
    array[position] = replacement.clone();
    validate_resource_data(&state, &resource, &data)?;
    write_resource(&state, &resource, &data).await?;
    Ok(Json(replacement))
}

pub async fn patch_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.write_lock_for_resource(&resource).await;
    let mut data = load_resource(&state, &resource).await?.as_ref().clone();
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
    write_resource(&state, &resource, &data).await?;
    Ok(Json(updated))
}

pub async fn delete_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
) -> Result<StatusCode, AppError> {
    let _guard = state.write_lock_for_resource(&resource).await;
    let mut data = load_resource(&state, &resource).await?.as_ref().clone();
    validate_resource_data(&state, &resource, &data)?;
    let array = data
        .as_array_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    let index = find_item_index(array, &id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?;
    array.remove(index);
    validate_resource_data(&state, &resource, &data)?;
    write_resource(&state, &resource, &data).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn replace_resource_object(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.write_lock_for_resource(&resource).await;
    let mut data = load_resource(&state, &resource).await?.as_ref().clone();
    if !data.is_object() || !payload.is_object() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Payload and resource must be JSON objects",
        ));
    }
    data = payload;
    write_resource(&state, &resource, &data).await?;
    Ok(Json(data))
}

pub async fn patch_resource_object(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.write_lock_for_resource(&resource).await;
    let mut data = load_resource(&state, &resource).await?.as_ref().clone();
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
    write_resource(&state, &resource, &data).await?;
    Ok(Json(updated))
}

fn maybe_fill_missing_id(
    item: &mut serde_json::Map<String, Value>,
    array: &[Value],
    table: Option<&TableSchema>,
) -> Result<(), AppError> {
    if item.contains_key("id") {
        return Ok(());
    }
    let id_value = match table.and_then(|table| table.columns.get("id")) {
        Some(column) if matches!(column.column_type, crate::schema::ColumnType::String) => {
            Value::String(format!("{}", next_numeric_id(array)))
        }
        _ => Value::from(next_numeric_id(array)),
    };
    item.insert("id".to_string(), id_value);
    Ok(())
}

fn embed_lock_resources(
    state: &AppState,
    resource: &str,
    embeds: &[String],
) -> Result<Vec<String>, AppError> {
    let mut resources = BTreeSet::new();
    resources.insert(resource.to_string());
    if !embeds.is_empty() {
        let table = state.schema_table(resource)?.ok_or_else(|| {
            AppError::new(
                StatusCode::BAD_REQUEST,
                "Embedding requires an active schema with foreign key definitions",
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

async fn embed_collection_data(
    state: &AppState,
    resource: &str,
    data: Value,
    embeds: &[String],
) -> Result<Value, AppError> {
    let table = state.schema_table(resource)?.ok_or_else(|| {
        AppError::new(
            StatusCode::BAD_REQUEST,
            "Embedding requires an active schema with foreign key definitions",
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
        for item in target_items {
            if let Some((_, key)) = item
                .as_object()
                .and_then(|object| object.get(&fk.target_column).map(|key| (object, key)))
            {
                lookup.insert(value_to_filter_string(key), item);
            }
        }

        for item in &mut embedded_items {
            let Some(object) = item.as_object_mut() else {
                continue;
            };
            let Some(current_value) = object.get(embed).cloned() else {
                continue;
            };
            if current_value.is_object() || current_value.is_null() {
                continue;
            }
            let key = value_to_filter_string(&current_value);
            let replacement = lookup.get(&key).map(|row| (*row).clone()).unwrap_or(Value::Null);
            object.insert(embed.clone(), replacement);
        }
    }
    Ok(Value::Array(embedded_items))
}

fn request_query_hash(path: &str, query: Option<&str>) -> Option<String> {
    if path != "/sql" && path != "/export.sql" {
        return None;
    }
    let query = query?;
    let sql = query
        .split('&')
        .find_map(|pair| pair.split_once('=').and_then(|(k, v)| (k == "q").then_some(v)))?;
    Some(stable_hash(sql))
}

fn stable_hash(value: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
