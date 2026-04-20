use std::{
    collections::{BTreeSet, HashMap},
    convert::Infallible,
    hash::{Hash, Hasher},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    body::Body,
    extract::DefaultBodyLimit,
    extract::{Path as AxumPath, Query, State},
    http::{
        HeaderMap, Method, Request, StatusCode,
        header::{
            ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
            ACCESS_CONTROL_ALLOW_ORIGIN, AUTHORIZATION, CONTENT_TYPE, ORIGIN,
        },
    },
    middleware::{self, Next},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use serde_json::Value;
use tokio_stream::{StreamExt, wrappers::BroadcastStream};

use crate::{
    app::AppState,
    error::AppError,
    graphql::{graphql_get, graphql_post},
    http::overview,
    mutation_service,
    query::filters::{
        filter_collection_refs, paginate_collection_refs, parse_collection_query_params,
        sort_collection_refs,
    },
    relations::{build_relation_lookup, resolve_related_row_in_lookup},
    schema::{
        DeclaredSchema, default_schema_output_path, infer_schema_from_data_source,
        primary_key_name, save_schema as save_schema_file,
    },
    sql::{export_sql, sql_query, sql_query_post},
    storage::{find_item_by_key, load_resource, validate_resource_data},
};

pub fn build_router(state: AppState) -> Router {
    let app = if state.config.readonly {
        Router::new()
            .route("/", get(list_resources))
            .route("/events", get(get_events))
            .route("/healthz", get(healthz))
            .route("/readyz", get(readyz))
            .route("/metrics", get(metrics))
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
            .route("/events", get(get_events))
            .route("/healthz", get(healthz))
            .route("/readyz", get(readyz))
            .route("/metrics", get(metrics))
            .route("/overview.json", get(get_overview))
            .route("/assets/overview.css", get(get_overview_css))
            .route("/assets/overview.js", get(get_overview_js))
            .route("/graphql", get(graphql_get).post(graphql_post))
            .route("/schema", get(get_schema).post(save_schema).put(save_declared_schema))
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

    let mut app = app.layer(DefaultBodyLimit::max(state.config.max_body_bytes));
    app = app.layer(middleware::from_fn_with_state(state.clone(), metrics_middleware));
    app = app.layer(middleware::from_fn_with_state(state.clone(), cors_middleware));
    app = app.layer(middleware::from_fn_with_state(state.clone(), auth_middleware));
    if state.config.enable_log {
        app = app.layer(middleware::from_fn_with_state(state.clone(), log_requests_middleware));
    }
    app
}

pub async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        state.metrics.render_prometheus(),
    )
}

pub async fn healthz(State(state): State<AppState>) -> Json<Value> {
    Json(serde_json::json!({
        "ok": true,
        "ready": state.health.is_ready(),
        "last_error": state.health.last_error(),
    }))
}

pub async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    let status =
        if state.health.is_ready() { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (
        status,
        Json(serde_json::json!({
            "ready": state.health.is_ready(),
            "last_error": state.health.last_error(),
        })),
    )
}

pub async fn get_events(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let stream =
        BroadcastStream::new(state.subscribe_events()).filter_map(|message| match message {
            Ok(event) => {
                let payload = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
                Some(Ok(Event::default().event(event.kind).data(payload)))
            }
            Err(_) => None,
        });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub async fn log_requests_middleware(
    State(_state): State<AppState>,
    request: Request<Body>,
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
    tracing::info!(target: "dirbase::request", "{line}");
    response
}

pub async fn metrics_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    state.metrics.record_request();
    let response = next.run(request).await;
    state.metrics.record_response(
        response.status().is_client_error() || response.status().is_server_error(),
    );
    response
}

pub async fn auth_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if request.method() == Method::OPTIONS {
        return next.run(request).await;
    }
    let path = request.uri().path();
    if matches!(path, "/healthz" | "/readyz" | "/metrics") {
        return next.run(request).await;
    }
    let Some(expected) = state.config.auth_token.as_deref() else {
        return next.run(request).await;
    };
    let authorized = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| token == expected);
    if authorized {
        return next.run(request).await;
    }
    state.metrics.record_auth_failure();
    AppError::new(StatusCode::UNAUTHORIZED, "Missing or invalid bearer token")
        .with_code("unauthorized")
        .into_response()
}

pub async fn cors_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let origin =
        request.headers().get(ORIGIN).and_then(|value| value.to_str().ok()).map(str::to_string);
    let allow_origin = state
        .config
        .cors_origin
        .as_deref()
        .zip(origin.as_deref())
        .and_then(|(expected, actual)| (expected == actual).then_some(actual.to_string()));

    if request.method() == Method::OPTIONS {
        let mut response = StatusCode::NO_CONTENT.into_response();
        if let Some(origin) = allow_origin {
            let headers = response.headers_mut();
            headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin.parse().expect("valid origin"));
            headers.insert(
                ACCESS_CONTROL_ALLOW_METHODS,
                "GET,POST,PUT,PATCH,DELETE,OPTIONS".parse().expect("allow methods"),
            );
            headers.insert(
                ACCESS_CONTROL_ALLOW_HEADERS,
                "content-type,authorization".parse().expect("allow headers"),
            );
        }
        return response;
    }

    let mut response = next.run(request).await;
    if let Some(origin) = allow_origin {
        let headers = response.headers_mut();
        headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin.parse().expect("valid origin"));
        headers.insert(
            ACCESS_CONTROL_ALLOW_METHODS,
            "GET,POST,PUT,PATCH,DELETE,OPTIONS".parse().expect("allow methods"),
        );
        headers.insert(
            ACCESS_CONTROL_ALLOW_HEADERS,
            "content-type,authorization".parse().expect("allow headers"),
        );
    }
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

    state.emit_event("schema_changed", None);

    Ok(Json(serde_json::json!({
        "saved": true,
        "path": path.display().to_string(),
    })))
}

pub async fn save_declared_schema(
    State(state): State<AppState>,
    Json(declared): Json<DeclaredSchema>,
) -> Result<Json<Value>, AppError> {
    let inferred = state.schema_store.read().expect("schema store").inferred.clone();
    crate::schema::merge_schemas(Some(&declared), &inferred)
        .map_err(|err| AppError::new(StatusCode::BAD_REQUEST, err))?;

    let path = default_schema_output_path(&state.data_source);
    let path_for_write = path.clone();
    let declared_for_write = declared.clone();
    tokio::task::spawn_blocking(move || {
        let payload = serde_json::to_string_pretty(&declared_for_write)
            .map_err(|err| format!("{}: {err}", path_for_write.display()))?;
        std::fs::write(&path_for_write, format!("{payload}\n"))
            .map_err(|err| format!("{}: {err}", path_for_write.display()))
    })
    .await
    .map_err(|err| {
        AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Schema save task failed: {err}"))
    })?
    .map_err(|err| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, err))?;

    state
        .update_declared_schema(Some(declared))
        .map_err(|err| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, err))?;
    state.invalidate_graphql_schema().await;
    state.emit_event("schema_changed", None);
    state.emit_event("overview_changed", None);
    state.health.mark_ready();

    Ok(Json(serde_json::json!({
        "saved": true,
        "declared": true,
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
    state.emit_event("schema_changed", None);
    state.emit_event("overview_changed", None);
    state.health.mark_ready();

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
    enforce_per_page_limit(&state, parsed.pagination)?;
    let lock_resources = embed_lock_resources(&state, &resource, &parsed.embeds)?;
    let _guards = state.read_locks_for_resources(&lock_resources).await;

    let data = load_resource(&state, &resource).await?;
    validate_resource_data(&state, &resource, data.as_ref())?;
    if !data.is_array() {
        if !collection_query_operators_present(&parsed) {
            return Ok(Json(data.as_ref().clone()));
        }
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Filtering, sorting, pagination, and embedding require a JSON array resource",
        ));
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

fn enforce_per_page_limit(
    state: &AppState,
    pagination: Option<crate::query::filters::Pagination>,
) -> Result<(), AppError> {
    if let Some(pagination) = pagination
        && pagination.per_page > state.config.max_per_page
    {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            format!("per_page exceeds configured max of {}", state.config.max_per_page),
        )
        .with_code("limit_exceeded"));
    }
    Ok(())
}

pub async fn create_item(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Json(payload): Json<Value>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let created = mutation_service::create_item(&state, &resource, payload).await?;
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
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let replacement = mutation_service::replace_item(&state, &resource, &id, payload).await?;
    Ok(Json(replacement))
}

pub async fn patch_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let updated = mutation_service::patch_item(&state, &resource, &id, payload).await?;
    Ok(Json(updated))
}

pub async fn delete_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
) -> Result<StatusCode, AppError> {
    mutation_service::delete_item(&state, &resource, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn replace_resource_object(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let data = mutation_service::replace_resource_object(&state, &resource, payload).await?;
    Ok(Json(data))
}

pub async fn patch_resource_object(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let updated = mutation_service::patch_resource_object(&state, &resource, payload).await?;
    Ok(Json(updated))
}

fn collection_query_operators_present(
    parsed: &crate::query::filters::ParsedCollectionQuery,
) -> bool {
    !parsed.filters.is_empty()
        || !parsed.sort_columns.is_empty()
        || parsed.pagination.is_some()
        || !parsed.embeds.is_empty()
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
