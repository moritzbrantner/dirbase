use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
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
        Html, IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use serde::Serialize;
use serde_json::Value;
use tokio_stream::{StreamExt, wrappers::BroadcastStream};

use crate::{
    app::AppState,
    error::AppError,
    graphql::{graphql_get, graphql_post},
    http::html::{encode_path_segment, escape_html},
    http::overview,
    mutation_service,
    query::filters::{
        filter_collection_refs, paginate_collection_refs, parse_collection_query_params,
        sort_collection_refs,
    },
    relations::{build_relation_lookup, resolve_related_row_in_lookup},
    schema::{
        ColumnType, DeclaredSchema, TableSchema, default_schema_output_path,
        export_declared_schema_snapshot, infer_schema_from_data_source, primary_key_name,
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
            .route("/schema/editor", get(get_schema_editor))
            .route("/sql", get(sql_query))
            .route("/export.sql", get(export_sql))
            .route("/sql/export", get(export_sql))
            .route("/{resource}/edit", get(get_resource_editor))
            .route("/{resource}/create", get(get_create_item_form))
            .route("/{resource}/{id}/edit", get(get_item_editor))
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
            .route("/schema/editor", get(get_schema_editor))
            .route("/schema/infer", axum::routing::post(infer_and_save_schema))
            .route("/sql", get(sql_query).post(sql_query_post))
            .route("/export.sql", get(export_sql))
            .route("/sql/export", get(export_sql))
            .route("/{resource}/edit", get(get_resource_editor))
            .route("/{resource}/create", get(get_create_item_form))
            .route("/{resource}/{id}/edit", get(get_item_editor))
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

pub async fn get_resource_editor(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
) -> Html<String> {
    Html(render_patch_editor_html(
        &format!("/{}", encode_path_segment(&resource)),
        state.config.readonly,
    ))
}

pub async fn get_item_editor(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
) -> Html<String> {
    Html(render_patch_editor_html(
        &format!("/{}/{}", encode_path_segment(&resource), encode_path_segment(&id)),
        state.config.readonly,
    ))
}

pub async fn get_create_item_form(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
) -> Result<Html<String>, AppError> {
    let _guard = state.read_lock_for_resource(&resource).await;
    let data = load_resource(&state, &resource).await?;
    validate_resource_data(&state, &resource, data.as_ref())?;
    let items = data.as_array().ok_or_else(|| {
        AppError::new(StatusCode::BAD_REQUEST, "Create forms require a JSON array resource")
    })?;
    let table = state.schema_table(&resource);
    let fields = create_form_fields(items, table.as_ref());
    Ok(Html(render_create_item_form_html(
        &resource,
        &format!("/{}", encode_path_segment(&resource)),
        &fields,
        state.config.readonly,
    )))
}

#[derive(Serialize)]
pub struct SchemaEditorPayload {
    pub inferred: crate::schema::Schema,
    pub declared: Option<DeclaredSchema>,
    pub effective: crate::schema::Schema,
    pub save_path: String,
}

pub async fn get_schema_editor(State(state): State<AppState>) -> Json<SchemaEditorPayload> {
    let save_path = default_schema_output_path(&state.data_source);
    Json(SchemaEditorPayload {
        inferred: state.inferred_schema_snapshot(),
        declared: state.declared_schema_snapshot(),
        effective: state.schema_snapshot(),
        save_path: save_path.display().to_string(),
    })
}

pub async fn save_schema(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let schema = state.schema_snapshot();
    let declared_snapshot =
        export_declared_schema_snapshot(state.declared_schema_snapshot().as_ref(), &schema);
    let path = default_schema_output_path(&state.data_source);
    let path_for_write = path.clone();
    tokio::task::spawn_blocking(move || {
        let payload = serde_json::to_string_pretty(&declared_snapshot)
            .map_err(|err| format!("{}: {err}", path_for_write.display()))?;
        std::fs::write(&path_for_write, format!("{payload}\n"))
            .map_err(|err| format!("{}: {err}", path_for_write.display()))
    })
    .await
    .map_err(|err| {
        AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Schema save task failed: {err}"))
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

    let declared_snapshot = export_declared_schema_snapshot(None, &inferred);
    let path = default_schema_output_path(&state.data_source);
    let path_for_write = path.clone();
    let declared_for_write = declared_snapshot.clone();
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
        .update_inferred_schema(inferred)
        .map_err(|err| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, err))?;
    state
        .update_declared_schema(Some(declared_snapshot))
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

fn render_patch_editor_html(target_path: &str, readonly: bool) -> String {
    let target_path_json =
        serde_json::to_string(target_path).expect("serializing an editor path cannot fail");
    let readonly_json =
        serde_json::to_string(&readonly).expect("serializing readonly flag cannot fail");
    let escaped_target_path = escape_html(target_path);
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Edit {escaped_target_path}</title>
  <style>
    :root {{
      color-scheme: light;
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      background: #f8fafc;
      color: #1f2937;
    }}
    body {{
      margin: 0;
      min-height: 100vh;
      display: flex;
      flex-direction: column;
    }}
    header {{
      border-bottom: 1px solid #d6d3d1;
      background: #ffffff;
      padding: 16px clamp(16px, 4vw, 40px);
    }}
    main {{
      flex: 1;
      display: grid;
      grid-template-rows: auto minmax(320px, 1fr) auto;
      gap: 12px;
      padding: 16px clamp(16px, 4vw, 40px) 24px;
    }}
    h1 {{
      font-size: 20px;
      line-height: 1.25;
      margin: 0 0 6px;
      overflow-wrap: anywhere;
    }}
    .route {{
      display: inline-block;
      max-width: 100%;
      overflow-wrap: anywhere;
      border: 1px solid #d6d3d1;
      background: #f5f5f4;
      border-radius: 6px;
      padding: 4px 8px;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      font-size: 13px;
    }}
    .toolbar {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 12px;
      flex-wrap: wrap;
    }}
    .status {{
      min-height: 22px;
      font-size: 14px;
      color: #57534e;
    }}
    .status.error {{
      color: #b91c1c;
    }}
    .status.ok {{
      color: #047857;
    }}
    textarea {{
      width: 100%;
      min-height: 100%;
      box-sizing: border-box;
      resize: vertical;
      border: 1px solid #a8a29e;
      border-radius: 8px;
      background: #ffffff;
      padding: 14px;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      font-size: 13px;
      line-height: 1.55;
      color: #1c1917;
      tab-size: 2;
    }}
    button {{
      border: 1px solid #78716c;
      border-radius: 6px;
      background: #292524;
      color: #ffffff;
      font: inherit;
      font-weight: 600;
      padding: 8px 12px;
      cursor: pointer;
    }}
    button.secondary {{
      background: #ffffff;
      color: #292524;
    }}
    button:disabled {{
      cursor: not-allowed;
      opacity: 0.55;
    }}
    .actions {{
      display: flex;
      justify-content: flex-end;
      gap: 8px;
      flex-wrap: wrap;
    }}
  </style>
</head>
<body>
  <header>
    <h1>Edit JSON resource</h1>
    <code class="route" id="target-path">{escaped_target_path}</code>
  </header>
  <main>
    <div class="toolbar">
      <div class="status" id="status" role="status">Loading resource...</div>
      <button type="button" class="secondary" id="reload-button">Reload</button>
    </div>
    <textarea id="editor" spellcheck="false" autocomplete="off" autocorrect="off" autocapitalize="off"></textarea>
    <div class="actions">
      <button type="button" class="secondary" id="format-button">Format JSON</button>
      <button type="button" id="save-button">Save patch</button>
    </div>
  </main>
  <script>
    const targetPath = {target_path_json};
    const readonly = {readonly_json};
    const editor = document.getElementById('editor');
    const statusNode = document.getElementById('status');
    const saveButton = document.getElementById('save-button');
    const reloadButton = document.getElementById('reload-button');
    const formatButton = document.getElementById('format-button');
    let originalValue = null;

    function setStatus(message, kind = '') {{
      statusNode.textContent = message;
      statusNode.className = kind ? `status ${{kind}}` : 'status';
    }}

    function formatJson(value) {{
      return JSON.stringify(value, null, 2) + '\n';
    }}

    function isPlainObject(value) {{
      return value !== null && typeof value === 'object' && !Array.isArray(value);
    }}

    function sameJson(left, right) {{
      return JSON.stringify(left) === JSON.stringify(right);
    }}

    function buildPatch(original, next) {{
      if (!isPlainObject(original) || !isPlainObject(next)) {{
        throw new Error('PATCH editing requires the loaded resource to be a JSON object.');
      }}
      const removedKeys = Object.keys(original).filter((key) => !Object.prototype.hasOwnProperty.call(next, key));
      if (removedKeys.length > 0) {{
        throw new Error(`PATCH cannot remove keys: ${{removedKeys.join(', ')}}. Set a value to null or use PUT from the API.`);
      }}
      const patch = {{}};
      for (const [key, value] of Object.entries(next)) {{
        if (!sameJson(original[key], value)) {{
          patch[key] = value;
        }}
      }}
      return patch;
    }}

    async function loadResource() {{
      saveButton.disabled = true;
      reloadButton.disabled = true;
      setStatus('Loading resource...');
      try {{
        const response = await fetch(targetPath, {{ headers: {{ Accept: 'application/json' }} }});
        const text = await response.text();
        if (!response.ok) {{
          throw new Error(text || `GET failed: ${{response.status}} ${{response.statusText}}`);
        }}
        originalValue = JSON.parse(text);
        editor.value = formatJson(originalValue);
        setStatus(readonly ? 'Read-only mode: editing is disabled.' : 'Loaded. Edit JSON and save a PATCH request.', readonly ? 'error' : 'ok');
      }} catch (error) {{
        setStatus(error instanceof Error ? error.message : 'Unable to load resource.', 'error');
      }} finally {{
        saveButton.disabled = readonly;
        reloadButton.disabled = false;
      }}
    }}

    async function savePatch() {{
      if (readonly) {{
        return;
      }}
      saveButton.disabled = true;
      reloadButton.disabled = true;
      try {{
        const nextValue = JSON.parse(editor.value);
        const patch = buildPatch(originalValue, nextValue);
        const changedKeys = Object.keys(patch);
        if (changedKeys.length === 0) {{
          setStatus('No top-level changes to patch.', 'error');
          return;
        }}
        setStatus(`Saving PATCH with ${{changedKeys.length}} changed key(s)...`);
        const response = await fetch(targetPath, {{
          method: 'PATCH',
          headers: {{
            Accept: 'application/json',
            'Content-Type': 'application/json'
          }},
          body: JSON.stringify(patch)
        }});
        const text = await response.text();
        if (!response.ok) {{
          throw new Error(text || `PATCH failed: ${{response.status}} ${{response.statusText}}`);
        }}
        originalValue = JSON.parse(text);
        editor.value = formatJson(originalValue);
        setStatus(`Saved PATCH for: ${{changedKeys.join(', ')}}`, 'ok');
      }} catch (error) {{
        setStatus(error instanceof Error ? error.message : 'Unable to save patch.', 'error');
      }} finally {{
        saveButton.disabled = readonly;
        reloadButton.disabled = false;
      }}
    }}

    function formatDraft() {{
      try {{
        editor.value = formatJson(JSON.parse(editor.value));
        setStatus('Formatted JSON.', 'ok');
      }} catch (error) {{
        setStatus(error instanceof Error ? error.message : 'Invalid JSON.', 'error');
      }}
    }}

    reloadButton.addEventListener('click', () => void loadResource());
    saveButton.addEventListener('click', () => void savePatch());
    formatButton.addEventListener('click', formatDraft);
    void loadResource();
  </script>
</body>
</html>
"#
    )
}

#[derive(Serialize)]
struct CreateFormField {
    name: String,
    field_type: &'static str,
    nullable: bool,
    primary_key: bool,
    sample: Option<Value>,
}

fn create_form_fields(items: &[Value], table: Option<&TableSchema>) -> Vec<CreateFormField> {
    let primary_key = primary_key_name(table);
    let sample_object = items.iter().find_map(Value::as_object);
    let mut fields = Vec::new();
    let mut seen = BTreeSet::new();

    if let Some(table) = table {
        for (name, column) in &table.columns {
            seen.insert(name.clone());
            fields.push(CreateFormField {
                name: name.clone(),
                field_type: create_form_column_type(&column.column_type),
                nullable: column.nullable,
                primary_key: name == primary_key,
                sample: sample_object.and_then(|object| object.get(name)).cloned(),
            });
        }
    }

    if let Some(object) = sample_object {
        let mut sampled = BTreeMap::new();
        for (name, value) in object {
            sampled.insert(name.clone(), value.clone());
        }
        for (name, value) in sampled {
            if !seen.insert(name.clone()) {
                continue;
            }
            fields.push(CreateFormField {
                name: name.clone(),
                field_type: infer_create_form_field_type(&value),
                nullable: true,
                primary_key: name == primary_key,
                sample: Some(value),
            });
        }
    }

    if fields.is_empty() {
        fields.push(CreateFormField {
            name: "name".to_string(),
            field_type: "string",
            nullable: false,
            primary_key: false,
            sample: None,
        });
    }

    fields.sort_by(|left, right| {
        right.primary_key.cmp(&left.primary_key).then_with(|| left.name.cmp(&right.name))
    });
    fields
}

fn create_form_column_type(column_type: &ColumnType) -> &'static str {
    column_type.label()
}

fn infer_create_form_field_type(value: &Value) -> &'static str {
    match value {
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer",
        Value::Number(_) => "float",
        Value::Bool(_) => "boolean",
        Value::Array(_) | Value::Object(_) => "json",
        _ => "string",
    }
}

fn render_create_item_form_html(
    resource: &str,
    target_path: &str,
    fields: &[CreateFormField],
    readonly: bool,
) -> String {
    let target_path_json =
        serde_json::to_string(target_path).expect("serializing a create path cannot fail");
    let fields_json =
        serde_json::to_string(fields).expect("serializing create form fields cannot fail");
    let readonly_json =
        serde_json::to_string(&readonly).expect("serializing readonly flag cannot fail");
    let escaped_target_path = escape_html(target_path);
    let escaped_resource = escape_html(resource);
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Create {escaped_target_path}</title>
  <style>
    :root {{
      color-scheme: light;
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      background: #f8fafc;
      color: #1f2937;
    }}
    body {{
      margin: 0;
      min-height: 100vh;
      display: flex;
      flex-direction: column;
    }}
    header {{
      border-bottom: 1px solid #d6d3d1;
      background: #ffffff;
      padding: 16px clamp(16px, 4vw, 40px);
    }}
    main {{
      flex: 1;
      display: grid;
      gap: 16px;
      padding: 16px clamp(16px, 4vw, 40px) 24px;
      max-width: 920px;
      width: 100%;
      box-sizing: border-box;
    }}
    h1 {{
      font-size: 20px;
      line-height: 1.25;
      margin: 0 0 6px;
      overflow-wrap: anywhere;
    }}
    .route, .result {{
      display: block;
      max-width: 100%;
      overflow-wrap: anywhere;
      border: 1px solid #d6d3d1;
      background: #f5f5f4;
      border-radius: 6px;
      padding: 8px 10px;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      font-size: 13px;
      white-space: pre-wrap;
    }}
    .status {{
      min-height: 22px;
      font-size: 14px;
      color: #57534e;
    }}
    .status.error {{
      color: #b91c1c;
    }}
    .status.ok {{
      color: #047857;
    }}
    form {{
      display: grid;
      gap: 14px;
    }}
    .field-row {{
      display: grid;
      gap: 8px;
      border: 1px solid #e7e5e4;
      border-radius: 8px;
      background: #ffffff;
      padding: 12px;
    }}
    .field-head {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 8px;
      flex-wrap: wrap;
    }}
    label {{
      font-weight: 650;
      color: #292524;
    }}
    .badges {{
      display: flex;
      gap: 6px;
      flex-wrap: wrap;
    }}
    .badge {{
      border: 1px solid #d6d3d1;
      border-radius: 999px;
      padding: 2px 7px;
      font-size: 12px;
      color: #57534e;
      background: #fafaf9;
    }}
    .control-grid {{
      display: grid;
      grid-template-columns: minmax(120px, 180px) minmax(0, 1fr);
      gap: 8px;
    }}
    @media (max-width: 640px) {{
      .control-grid {{
        grid-template-columns: 1fr;
      }}
    }}
    input, select, textarea {{
      width: 100%;
      box-sizing: border-box;
      border: 1px solid #a8a29e;
      border-radius: 6px;
      background: #ffffff;
      padding: 8px 10px;
      font: inherit;
      color: #1c1917;
    }}
    textarea {{
      min-height: 96px;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      font-size: 13px;
      line-height: 1.5;
      resize: vertical;
    }}
    button {{
      border: 1px solid #78716c;
      border-radius: 6px;
      background: #292524;
      color: #ffffff;
      font: inherit;
      font-weight: 600;
      padding: 8px 12px;
      cursor: pointer;
    }}
    button.secondary {{
      background: #ffffff;
      color: #292524;
    }}
    button:disabled, input:disabled, select:disabled, textarea:disabled {{
      cursor: not-allowed;
      opacity: 0.55;
    }}
    .actions {{
      display: flex;
      justify-content: flex-end;
      gap: 8px;
      flex-wrap: wrap;
    }}
    .custom-name {{
      display: none;
    }}
    .field-row.is-custom .custom-name {{
      display: block;
    }}
  </style>
</head>
<body>
  <header>
    <h1>Create item in {escaped_resource}</h1>
    <code class="route" id="target-path">POST {escaped_target_path}</code>
  </header>
  <main>
    <div class="status" id="status" role="status"></div>
    <form id="create-form">
      <div id="fields"></div>
      <div class="actions">
        <button type="button" class="secondary" id="add-field-button">Add field</button>
        <button type="reset" class="secondary" id="reset-button">Reset</button>
        <button type="submit" id="submit-button">Create item</button>
      </div>
    </form>
    <pre class="result" id="result" hidden></pre>
  </main>
  <script>
    const targetPath = {target_path_json};
    const readonly = {readonly_json};
    const fields = {fields_json};
    const fieldsNode = document.getElementById('fields');
    const form = document.getElementById('create-form');
    const statusNode = document.getElementById('status');
    const resultNode = document.getElementById('result');
    const submitButton = document.getElementById('submit-button');
    const addFieldButton = document.getElementById('add-field-button');
    let customFieldIndex = 0;

    function setStatus(message, kind = '') {{
      statusNode.textContent = message;
      statusNode.className = kind ? `status ${{kind}}` : 'status';
    }}

    function formatSample(value) {{
      if (value === null || value === undefined) {{
        return '';
      }}
      if (typeof value === 'object') {{
        return JSON.stringify(value);
      }}
      return String(value);
    }}

    function createValueControl(field) {{
      if (field.field_type === 'boolean') {{
        const select = document.createElement('select');
        select.name = 'value';
        select.append(new Option(field.nullable || field.primary_key ? 'Blank' : 'False', ''));
        select.append(new Option('True', 'true'));
        select.append(new Option('False', 'false'));
        select.required = !field.nullable && !field.primary_key;
        return select;
      }}
      if (field.field_type === 'json') {{
        const textarea = document.createElement('textarea');
        textarea.name = 'value';
        textarea.placeholder = formatSample(field.sample) || '{{}}';
        textarea.required = !field.nullable && !field.primary_key;
        return textarea;
      }}
      const input = document.createElement('input');
      input.name = 'value';
      input.type = field.field_type === 'integer' || field.field_type === 'float' ? 'number' : 'text';
      if (field.field_type === 'integer') {{
        input.step = '1';
      }} else if (field.field_type === 'float') {{
        input.step = 'any';
      }}
      input.placeholder = field.primary_key ? 'auto-generated if blank' : formatSample(field.sample);
      input.required = !field.nullable && !field.primary_key;
      return input;
    }}

    function renderField(field, custom = false) {{
      const row = document.createElement('section');
      row.className = custom ? 'field-row is-custom' : 'field-row';
      row.dataset.fieldType = field.field_type;
      if (!custom) {{
        row.dataset.fieldName = field.name;
      }}
      row.dataset.nullable = String(field.nullable);
      row.dataset.primaryKey = String(field.primary_key);

      const head = document.createElement('div');
      head.className = 'field-head';
      const label = document.createElement('label');
      label.textContent = field.name;
      const badges = document.createElement('div');
      badges.className = 'badges';
      for (const badgeText of [
        field.field_type,
        field.primary_key ? 'primary key' : null,
        field.nullable || field.primary_key ? 'optional' : 'required'
      ].filter(Boolean)) {{
        const badge = document.createElement('span');
        badge.className = 'badge';
        badge.textContent = badgeText;
        badges.append(badge);
      }}
      head.append(label, badges);

      const grid = document.createElement('div');
      grid.className = 'control-grid';
      const typeSelect = document.createElement('select');
      typeSelect.name = 'type';
      for (const type of ['string', 'integer', 'float', 'boolean', 'json']) {{
        typeSelect.append(new Option(type, type));
      }}
      typeSelect.value = field.field_type;
      typeSelect.addEventListener('change', () => {{
        row.dataset.fieldType = typeSelect.value;
        valueSlot.replaceChildren(createValueControl({{ ...field, field_type: typeSelect.value }}));
      }});

      const valueSlot = document.createElement('div');
      valueSlot.append(createValueControl(field));
      grid.append(typeSelect, valueSlot);

      const customName = document.createElement('input');
      customName.className = 'custom-name';
      customName.name = 'name';
      customName.placeholder = 'field_name';
      customName.required = custom;

      row.append(head, customName, grid);
      if (readonly) {{
        for (const control of row.querySelectorAll('input, select, textarea')) {{
          control.disabled = true;
        }}
      }}
      return row;
    }}

    function renderFields() {{
      fieldsNode.replaceChildren(...fields.map((field) => renderField(field)));
      resultNode.hidden = true;
      resultNode.textContent = '';
      setStatus(readonly ? 'Read-only mode: item creation is disabled.' : 'Enter values and submit a POST request.', readonly ? 'error' : 'ok');
      submitButton.disabled = readonly;
      addFieldButton.disabled = readonly;
    }}

    function parseFieldValue(type, rawValue, fieldName) {{
      if (type === 'integer') {{
        const number = Number(rawValue);
        if (!Number.isInteger(number)) {{
          throw new Error(`${{fieldName}} must be an integer.`);
        }}
        return number;
      }}
      if (type === 'float') {{
        const number = Number(rawValue);
        if (!Number.isFinite(number)) {{
          throw new Error(`${{fieldName}} must be a number.`);
        }}
        return number;
      }}
      if (type === 'boolean') {{
        if (rawValue !== 'true' && rawValue !== 'false') {{
          throw new Error(`${{fieldName}} must be true or false.`);
        }}
        return rawValue === 'true';
      }}
      if (type === 'json') {{
        return JSON.parse(rawValue);
      }}
      return rawValue;
    }}

    function buildPayload() {{
      const payload = {{}};
      for (const row of fieldsNode.querySelectorAll('.field-row')) {{
        const fieldName = row.dataset.fieldName || row.querySelector('input[name="name"]').value.trim();
        const type = row.querySelector('select[name="type"]').value;
        const valueControl = row.querySelector('[name="value"]');
        const rawValue = valueControl.value;
        const optional = row.dataset.nullable === 'true' || row.dataset.primaryKey === 'true';
        if (!fieldName) {{
          throw new Error('Custom fields need a name.');
        }}
        if (rawValue === '' && optional) {{
          continue;
        }}
        payload[fieldName] = parseFieldValue(type, rawValue, fieldName);
      }}
      return payload;
    }}

    form.addEventListener('submit', async (event) => {{
      event.preventDefault();
      if (readonly) {{
        return;
      }}
      submitButton.disabled = true;
      resultNode.hidden = true;
      try {{
        const payload = buildPayload();
        setStatus('Creating item...');
        const response = await fetch(targetPath, {{
          method: 'POST',
          headers: {{
            Accept: 'application/json',
            'Content-Type': 'application/json'
          }},
          body: JSON.stringify(payload)
        }});
        const text = await response.text();
        if (!response.ok) {{
          throw new Error(text || `POST failed: ${{response.status}} ${{response.statusText}}`);
        }}
        const created = JSON.parse(text);
        resultNode.hidden = false;
        resultNode.textContent = JSON.stringify(created, null, 2);
        setStatus('Created item.', 'ok');
      }} catch (error) {{
        setStatus(error instanceof Error ? error.message : 'Unable to create item.', 'error');
      }} finally {{
        submitButton.disabled = readonly;
      }}
    }});

    form.addEventListener('reset', () => {{
      setTimeout(renderFields, 0);
    }});

    addFieldButton.addEventListener('click', () => {{
      customFieldIndex += 1;
      fieldsNode.append(renderField({{
        name: `Custom field ${{customFieldIndex}}`,
        field_type: 'string',
        nullable: true,
        primary_key: false,
        sample: null
      }}, true));
    }});

    renderFields();
  </script>
</body>
</html>
"#
    )
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
