use std::{
    collections::{BTreeSet, HashMap},
    hash::{Hash, Hasher},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde_json::Value;

use crate::{
    app::AppState,
    error::AppError,
    graphql::{graphql_get, graphql_post},
    http::overview,
    query::filters::{
        filter_collection_refs, paginate_collection_refs, parse_collection_query_params,
        sort_collection_refs,
    },
    relations::{build_relation_lookup, resolve_related_row_in_lookup},
    schema::{
        TableSchema, default_schema_output_path, infer_schema_from_data_source, primary_key_name,
        save_schema as save_schema_file,
    },
    sql::{export_sql, sql_query, sql_query_post},
    storage::{
        coerce_id_value, find_item_by_key, find_item_index_by_key, load_resource, next_numeric_id,
        validate_resource_data, write_resource,
    },
};

pub fn build_router(state: AppState, readonly: bool, enable_log: bool) -> Router {
    let app = if readonly {
        Router::new()
            .route("/", get(list_resources))
            .route("/overview.json", get(get_overview))
            .route("/assets/overview.css", get(get_overview_css))
            .route("/assets/overview.js", get(get_overview_js))
            .route("/graphql", get(graphql_get).post(graphql_post))
            .route("/schema", get(get_schema))
            .route("/sql", get(sql_query))
            .route("/export.sql", get(export_sql))
            .route("/sql/export", get(export_sql))
            .route("/{resource}", get(get_collection))
            .route("/{resource}/{id}", get(get_item))
            .with_state(state.clone())
    } else {
        Router::new()
            .route("/", get(list_resources))
            .route("/overview.json", get(get_overview))
            .route("/assets/overview.css", get(get_overview_css))
            .route("/assets/overview.js", get(get_overview_js))
            .route("/graphql", get(graphql_get).post(graphql_post))
            .route("/schema", get(get_schema).post(save_schema))
            .route("/schema/infer", axum::routing::post(infer_and_save_schema))
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

pub async fn list_resources(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let resources = state.resource_names_sorted().await;
    if overview::request_prefers_html(&headers) {
        return Ok(overview::render_root_overview(&state, &resources).await?.into_response());
    }
    Ok(Json(serde_json::json!({"resources": resources})).into_response())
}

pub async fn get_overview(
    State(state): State<AppState>,
) -> Result<Json<overview::OverviewPageData>, AppError> {
    overview::get_overview_json(&state).await
}

pub async fn get_overview_css() -> Response {
    overview::overview_css()
}

pub async fn get_overview_js() -> Response {
    overview::overview_js()
}

pub async fn get_schema(State(state): State<AppState>) -> Json<crate::schema::Schema> {
    Json(state.schema_snapshot())
}

pub async fn save_schema(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let schema = state.schema_snapshot();
    let path = default_schema_output_path(&state.data_source);
    let path_for_write = path.clone();
    tokio::task::spawn_blocking(move || save_schema_file(&path_for_write, &schema))
        .await
        .map_err(|err| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Schema save task failed: {err}"),
            )
        })?
        .map_err(|err| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, err))?;

    Ok(Json(serde_json::json!({
        "saved": true,
        "path": path.display().to_string(),
    })))
}

pub async fn infer_and_save_schema(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let resources = state.resources.read().await.clone();
    let data_source = state.data_source.clone();
    let inferred = tokio::task::spawn_blocking(move || {
        infer_schema_from_data_source(&data_source, &resources)
    })
    .await
    .map_err(|err| {
        AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Schema infer task failed: {err}"))
    })?
    .map_err(|err| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, err))?;

    let path = default_schema_output_path(&state.data_source);
    let path_for_write = path.clone();
    let inferred_for_write = inferred.clone();
    tokio::task::spawn_blocking(move || save_schema_file(&path_for_write, &inferred_for_write))
        .await
        .map_err(|err| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Schema save task failed: {err}"),
            )
        })?
        .map_err(|err| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, err))?;

    state
        .update_inferred_schema(inferred)
        .map_err(|err| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, err))?;
    state.invalidate_graphql_schema().await;

    Ok(Json(serde_json::json!({
        "saved": true,
        "inferred": true,
        "path": path.display().to_string(),
    })))
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
    validate_resource_data(&state, &resource, data.as_ref())?;
    if !data.is_array() {
        if parsed.filters.is_empty()
            && parsed.sort_columns.is_empty()
            && parsed.pagination.is_none()
            && parsed.embeds.is_empty()
        {
            return Ok(Json(data.as_ref().clone()));
        }
        return Err(AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"));
    }
    let items = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    let mut selected = if parsed.filters.is_empty() {
        items.iter().collect::<Vec<_>>()
    } else {
        filter_collection_refs(items, &parsed.filters, None)
    };

    if !parsed.sort_columns.is_empty() {
        sort_collection_refs(selected.as_mut_slice(), &parsed.sort_columns);
    }

    let materialized = if let Some(pagination) = parsed.pagination {
        let mut paginated = paginate_collection_refs(&selected, pagination);
        if parsed.embeds.is_empty() {
            paginated
        } else {
            let data_field = paginated.get_mut("data").ok_or_else(|| {
                AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "Missing page data")
            })?;
            let embedded_page = embed_collection_data(
                &state,
                &resource,
                std::mem::replace(data_field, Value::Array(Vec::new())),
                &parsed.embeds,
            )
            .await?;
            *data_field = embedded_page;
            paginated
        }
    } else {
        let selected = Value::Array(selected.into_iter().cloned().collect());
        if parsed.embeds.is_empty() {
            selected
        } else {
            embed_collection_data(&state, &resource, selected, &parsed.embeds).await?
        }
    };

    Ok(Json(materialized))
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
    let table = state.schema_table(&resource);
    maybe_fill_missing_id(item, array, table.as_ref())?;
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
    validate_resource_data(&state, &resource, data.as_ref())?;
    let array = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    let table = state.schema_table(&resource);
    let item_key = primary_key_name(table.as_ref());
    if let Some(position) = state
        .resource_cache
        .read()
        .await
        .get(&resource)
        .filter(|cached| cached.primary_key == item_key)
        .and_then(|cached| cached.id_index.as_ref())
        .and_then(|index| index.get(&id).copied())
    {
        return Ok(Json(
            array
                .get(position)
                .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?
                .clone(),
        ));
    }
    Ok(Json(
        find_item_by_key(array, item_key, &id)
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
    let table = state.schema_table(&resource);
    let item_key = primary_key_name(table.as_ref()).to_string();
    let array = data
        .as_array_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    let object = payload
        .as_object_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Payload must be a JSON object"))?;
    object.insert(item_key.clone(), coerce_id_value(&id, table.as_ref()));
    let replacement = Value::Object(object.clone());
    let position = find_item_index_by_key(array, &item_key, &id)
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
    let table = state.schema_table(&resource);
    let item_key = primary_key_name(table.as_ref()).to_string();
    validate_resource_data(&state, &resource, &data)?;
    let array = data
        .as_array_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    let patch = payload
        .as_object()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Payload must be a JSON object"))?;
    let index = find_item_index_by_key(array, &item_key, &id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?;
    let current = array[index].as_object_mut().ok_or_else(|| {
        AppError::new(StatusCode::BAD_REQUEST, "Array item must be a JSON object")
    })?;
    for (key, value) in patch {
        if key != &item_key {
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
    let table = state.schema_table(&resource);
    let item_key = primary_key_name(table.as_ref()).to_string();
    validate_resource_data(&state, &resource, &data)?;
    let array = data
        .as_array_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    let index = find_item_index_by_key(array, &item_key, &id)
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
    let item_key = primary_key_name(table);
    if item.contains_key(item_key) {
        return Ok(());
    }
    let id_value = match table.and_then(|table| table.columns.get(item_key)) {
        Some(column) if matches!(column.column_type, crate::schema::ColumnType::String) => {
            Value::String(format!("{}", next_numeric_id(array, item_key)))
        }
        _ => Value::from(next_numeric_id(array, item_key)),
    };
    item.insert(item_key.to_string(), id_value);
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

async fn embed_collection_data(
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
