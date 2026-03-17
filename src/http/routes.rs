use std::{
    collections::{BTreeSet, HashMap},
    fmt::Write,
    hash::{Hash, Hasher},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, Request, StatusCode, header::ACCEPT},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use serde::Serialize;
use serde_json::Value;

use crate::{
    app::{AppState, DataSource},
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

pub async fn list_resources(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let resources = state.resource_names_sorted().await;
    if request_prefers_html(&headers) {
        return Ok(render_root_overview(&state, &resources).await?.into_response());
    }
    Ok(Json(serde_json::json!({"resources": resources})).into_response())
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

#[derive(Serialize)]
struct OverviewPageData {
    schema_enabled: bool,
    data_source_kind: &'static str,
    source_label: String,
    source_rule: String,
    resource_name_rule: &'static str,
    resources: Vec<ResourceOverview>,
    edges: Vec<OverviewEdge>,
}

#[derive(Serialize)]
struct ResourceOverview {
    name: String,
    kind: &'static str,
    count_label: String,
    detail_label: String,
    field_names: Vec<String>,
    row_samples: Vec<String>,
    columns: Vec<OverviewColumn>,
    outgoing_relations: Vec<String>,
    incoming_relations: Vec<String>,
    sample_item_id: Option<String>,
}

#[derive(Serialize)]
struct OverviewColumn {
    name: String,
    column_type: &'static str,
    nullable: bool,
    relation: Option<String>,
}

#[derive(Serialize)]
struct OverviewEdge {
    source_table: String,
    source_column: String,
    target_table: String,
    target_column: String,
}

fn request_prefers_html(headers: &HeaderMap) -> bool {
    headers
        .get(ACCEPT)
        .and_then(|value| value.to_str().ok())
        .map(|accept| {
            accept.split(',').map(str::trim).any(|value| {
                value.starts_with("text/html") || value.starts_with("application/xhtml+xml")
            })
        })
        .unwrap_or(false)
}

async fn render_root_overview(
    state: &AppState,
    resources: &[String],
) -> Result<Html<String>, AppError> {
    let _guards = state.read_locks_for_resources(resources).await;
    let page = build_overview_page_data(state, resources).await?;
    Ok(Html(render_overview_html(&page)))
}

async fn build_overview_page_data(
    state: &AppState,
    resources: &[String],
) -> Result<OverviewPageData, AppError> {
    let resource_set = resources.iter().cloned().collect::<BTreeSet<_>>();
    let mut incoming_relations: HashMap<String, Vec<String>> = HashMap::new();
    let mut outgoing_relations: HashMap<String, Vec<String>> = HashMap::new();
    let mut edges = Vec::new();

    if let Some(schema) = state.schema.as_ref() {
        for (source_table, table) in &schema.tables {
            if !resource_set.contains(source_table) {
                continue;
            }
            for (source_column, fk) in &table.foreign_keys {
                if !resource_set.contains(&fk.target_table) {
                    continue;
                }
                outgoing_relations
                    .entry(source_table.clone())
                    .or_default()
                    .push(format!("{} -> {}.{}", source_column, fk.target_table, fk.target_column));
                incoming_relations
                    .entry(fk.target_table.clone())
                    .or_default()
                    .push(format!("{}.{} -> {}", source_table, source_column, fk.target_column));
                edges.push(OverviewEdge {
                    source_table: source_table.clone(),
                    source_column: source_column.clone(),
                    target_table: fk.target_table.clone(),
                    target_column: fk.target_column.clone(),
                });
            }
        }
    }

    let (data_source_kind, source_label, source_rule) = match &*state.data_source {
        DataSource::Folder(folder) => (
            "folder",
            folder.display().to_string(),
            "Each valid `*.json` filename becomes `/{resource}`.".to_string(),
        ),
        DataSource::File(file) => (
            "file",
            file.display().to_string(),
            "Each valid top-level key in the JSON file becomes `/{resource}`.".to_string(),
        ),
    };

    let mut summaries = Vec::with_capacity(resources.len());
    for resource in resources {
        let value = load_resource(state, resource).await?;
        let table_schema =
            state.schema.as_ref().as_ref().and_then(|schema| schema.tables.get(resource.as_str()));
        let (kind, count_label, mut detail_label, field_names, row_samples) =
            summarize_resource_value(value.as_ref(), table_schema.is_some());
        let columns = table_schema
            .map(|table| {
                table
                    .columns
                    .iter()
                    .map(|(column_name, column)| OverviewColumn {
                        name: column_name.clone(),
                        column_type: column_type_label(&column.column_type),
                        nullable: column.nullable,
                        relation: table
                            .foreign_keys
                            .get(column_name)
                            .map(|fk| format!("{}.{}", fk.target_table, fk.target_column)),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if !columns.is_empty() {
            detail_label = format!("{} columns", columns.len());
        }

        summaries.push(ResourceOverview {
            name: resource.clone(),
            kind,
            count_label,
            detail_label,
            field_names,
            row_samples,
            columns,
            outgoing_relations: outgoing_relations.remove(resource).unwrap_or_default(),
            incoming_relations: incoming_relations.remove(resource).unwrap_or_default(),
            sample_item_id: sample_item_id(value.as_ref()),
        });
    }

    Ok(OverviewPageData {
        schema_enabled: state.schema.is_some(),
        data_source_kind,
        source_label,
        source_rule,
        resource_name_rule: "Resource names may only use letters, numbers, `_`, and `-`.",
        resources: summaries,
        edges,
    })
}

fn summarize_resource_value(
    value: &Value,
    has_schema: bool,
) -> (&'static str, String, String, Vec<String>, Vec<String>) {
    match value {
        Value::Array(items) => {
            let mut field_names = BTreeSet::new();
            for item in items.iter().take(24) {
                if let Some(object) = item.as_object() {
                    for key in object.keys() {
                        field_names.insert(key.clone());
                        if field_names.len() >= 8 {
                            break;
                        }
                    }
                }
                if field_names.len() >= 8 {
                    break;
                }
            }
            let detail = if has_schema {
                format!("{} columns", field_names.len().max(1))
            } else {
                format!("{} sampled fields", field_names.len())
            };
            (
                "table",
                format!("{} rows", items.len()),
                detail,
                field_names.into_iter().collect(),
                items.iter().take(2).map(preview_value).collect(),
            )
        }
        Value::Object(object) => (
            "object",
            "object resource".to_string(),
            format!("{} keys", object.len()),
            object.keys().take(8).cloned().collect(),
            vec![preview_value(value)],
        ),
        _ => (
            "value",
            "scalar resource".to_string(),
            "single JSON value".to_string(),
            Vec::new(),
            vec![preview_value(value)],
        ),
    }
}

fn preview_value(value: &Value) -> String {
    let raw = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
    truncate_for_preview(&raw, 160)
}

fn sample_item_id(value: &Value) -> Option<String> {
    value
        .as_array()
        .and_then(|items| items.iter().find_map(|item| item.as_object()))
        .and_then(|object| object.get("id"))
        .and_then(path_segment_value)
}

fn path_segment_value(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn truncate_for_preview(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let truncated = value.chars().take(max_chars.saturating_sub(1)).collect::<String>();
    format!("{truncated}...")
}

fn column_type_label(column_type: &crate::schema::ColumnType) -> &'static str {
    match column_type {
        crate::schema::ColumnType::Integer => "integer",
        crate::schema::ColumnType::Float => "float",
        crate::schema::ColumnType::Boolean => "boolean",
        crate::schema::ColumnType::String => "string",
        crate::schema::ColumnType::Json => "json",
    }
}

fn render_overview_html(page: &OverviewPageData) -> String {
    let mut html = String::new();
    let total_rows = page
        .resources
        .iter()
        .filter_map(|resource| match resource.kind {
            "table" => resource
                .count_label
                .split_whitespace()
                .next()
                .and_then(|value| value.parse::<usize>().ok()),
            _ => None,
        })
        .sum::<usize>();
    let page_json = serde_json::to_string(page)
        .unwrap_or_else(|_| "{\"resources\":[],\"edges\":[]}".to_string())
        .replace("</", "<\\/");
    let sample_resource_name =
        page.resources.first().map(|resource| resource.name.as_str()).unwrap_or("resource");
    let sample_collection_path = format!("/{sample_resource_name}");
    let sample_item_path = page
        .resources
        .iter()
        .find_map(|resource| {
            resource.sample_item_id.as_ref().map(|id| format!("/{}/{}", resource.name, id))
        })
        .unwrap_or_else(|| format!("{sample_collection_path}/1"));
    let sample_field = page
        .resources
        .iter()
        .find_map(|resource| {
            resource
                .columns
                .iter()
                .map(|column| column.name.as_str())
                .chain(resource.field_names.iter().map(String::as_str))
                .find(|name| *name != "id")
                .map(str::to_string)
        })
        .unwrap_or_else(|| "field".to_string());
    let sample_filter_path = format!("{sample_collection_path}?{sample_field}=value");
    let sample_sort_path = format!("{sample_collection_path}?sort=-{sample_field}");
    let sample_page_path = format!("{sample_collection_path}?page=1&per_page=10");
    let sample_embed_path = page
        .resources
        .iter()
        .find_map(|resource| {
            resource.columns.iter().find_map(|column| {
                column.relation.as_ref().map(|_| {
                    format!("/{name}?embed={column}", name = resource.name, column = column.name)
                })
            })
        })
        .unwrap_or_else(|| format!("{sample_collection_path}?embed=foreign_key"));

    html.push_str(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
    );
    html.push_str("<title>folder-server guide</title>");
    html.push_str(
        r#"<style>
        :root {
            --bg: #f4efe6;
            --panel: rgba(255, 252, 247, 0.84);
            --panel-strong: #fffaf3;
            --line: rgba(35, 77, 66, 0.18);
            --text: #1f2e2a;
            --muted: #5e6d68;
            --accent: #0f766e;
            --accent-soft: rgba(15, 118, 110, 0.12);
            --accent-warm: #c26a3d;
            --shadow: 0 20px 60px rgba(44, 51, 48, 0.12);
            --error: #8f3531;
        }
        * { box-sizing: border-box; }
        body {
            margin: 0;
            color: var(--text);
            font-family: "IBM Plex Sans", "Avenir Next", "Segoe UI", sans-serif;
            background:
                radial-gradient(circle at top left, rgba(194, 106, 61, 0.22), transparent 28rem),
                radial-gradient(circle at top right, rgba(15, 118, 110, 0.16), transparent 24rem),
                linear-gradient(180deg, #f7f1e7 0%, #efe6d8 100%);
        }
        .page {
            max-width: 1280px;
            margin: 0 auto;
            padding: 2.5rem 1.2rem 4rem;
        }
        .page > * + * {
            margin-top: 1.3rem;
        }
        .hero {
            padding: 1.5rem;
            border: 1px solid rgba(31, 46, 42, 0.08);
            border-radius: 28px;
            background: linear-gradient(135deg, rgba(255, 250, 243, 0.9), rgba(249, 242, 232, 0.74));
            box-shadow: var(--shadow);
        }
        .eyebrow {
            margin: 0 0 0.65rem;
            text-transform: uppercase;
            letter-spacing: 0.18em;
            font-size: 0.74rem;
            color: var(--accent);
            font-weight: 700;
        }
        h1 {
            margin: 0;
            font-family: "Iowan Old Style", "Palatino Linotype", serif;
            font-size: clamp(2rem, 5vw, 3.6rem);
            line-height: 0.95;
        }
        .lede {
            margin: 1rem 0 0;
            max-width: 50rem;
            color: var(--muted);
            font-size: 1rem;
            line-height: 1.6;
        }
        .stats {
            display: flex;
            flex-wrap: wrap;
            gap: 0.75rem;
            margin-top: 1.25rem;
        }
        .stat {
            padding: 0.7rem 0.95rem;
            border-radius: 999px;
            background: var(--panel-strong);
            border: 1px solid rgba(31, 46, 42, 0.08);
            font-size: 0.92rem;
        }
        .section-title {
            margin: 0 0 0.8rem;
            font-size: 0.95rem;
            text-transform: uppercase;
            letter-spacing: 0.16em;
            color: var(--muted);
        }
        .panel-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
            gap: 1rem;
        }
        .info-panel,
        .playground-shell,
        .inspector {
            border-radius: 30px;
            border: 1px solid rgba(31, 46, 42, 0.08);
            background: linear-gradient(180deg, rgba(255, 251, 245, 0.94), rgba(251, 247, 240, 0.84));
            box-shadow: var(--shadow);
            padding: 1.2rem;
        }
        .panel-title {
            margin: 0;
            font-size: 1.35rem;
            line-height: 1.1;
        }
        .lede-tight {
            margin: 0.85rem 0 0;
            color: var(--muted);
            line-height: 1.6;
        }
        .source-line,
        .inline-code,
        .generated-url {
            display: inline-flex;
            align-items: center;
            gap: 0.45rem;
            padding: 0.38rem 0.62rem;
            border-radius: 999px;
            border: 1px solid rgba(31, 46, 42, 0.08);
            background: rgba(255, 255, 255, 0.8);
            font-family: "IBM Plex Mono", "SFMono-Regular", monospace;
            font-size: 0.84rem;
            overflow-wrap: anywhere;
        }
        .source-line {
            display: inline-block;
            margin-top: 0.85rem;
            border-radius: 16px;
            padding: 0.65rem 0.8rem;
        }
        .rule-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
            gap: 0.8rem;
            margin-top: 1rem;
        }
        .rule-card {
            padding: 0.9rem;
            border-radius: 20px;
            border: 1px solid rgba(31, 46, 42, 0.08);
            background: rgba(255, 255, 255, 0.72);
        }
        .rule-title {
            margin: 0;
            font-size: 0.9rem;
            font-weight: 700;
        }
        .path-code {
            display: block;
            margin-top: 0.7rem;
            padding: 0.72rem 0.82rem;
            border-radius: 16px;
            background: #f7f3ec;
            border: 1px solid rgba(31, 46, 42, 0.08);
            font-family: "IBM Plex Mono", "SFMono-Regular", monospace;
            font-size: 0.84rem;
            line-height: 1.45;
            overflow-wrap: anywhere;
        }
        .method {
            color: var(--accent);
            font-weight: 700;
        }
        .rule-copy {
            margin: 0.7rem 0 0;
            color: var(--muted);
            font-size: 0.9rem;
            line-height: 1.55;
        }
        .note-list {
            display: grid;
            gap: 0.65rem;
            margin-top: 1rem;
        }
        .note-item {
            padding: 0.75rem 0.85rem;
            border-left: 3px solid rgba(15, 118, 110, 0.35);
            background: rgba(255, 255, 255, 0.62);
            border-radius: 0 16px 16px 0;
            color: var(--muted);
            line-height: 1.55;
        }
        .playground-shell {
            display: grid;
            grid-template-columns: minmax(280px, 360px) minmax(0, 1fr);
            gap: 1rem;
        }
        .controls {
            display: grid;
            gap: 0.85rem;
        }
        .control label {
            display: block;
            margin-bottom: 0.35rem;
            font-size: 0.78rem;
            text-transform: uppercase;
            letter-spacing: 0.12em;
            color: var(--muted);
        }
        .control select,
        .control input,
        .control textarea,
        .button-row button {
            width: 100%;
            border: 1px solid rgba(31, 46, 42, 0.12);
            border-radius: 16px;
            padding: 0.75rem 0.85rem;
            background: rgba(255, 255, 255, 0.82);
            color: var(--text);
            font: inherit;
        }
        .control textarea {
            min-height: 5.25rem;
            resize: vertical;
        }
        .button-row {
            display: flex;
            flex-wrap: wrap;
            gap: 0.75rem;
            margin-top: 1rem;
        }
        .button-row button {
            width: auto;
            cursor: pointer;
            transition: transform 120ms ease, border-color 120ms ease, background 120ms ease;
        }
        .button-row button:hover {
            transform: translateY(-1px);
            border-color: rgba(15, 118, 110, 0.35);
            background: rgba(255, 255, 255, 0.94);
        }
        .button-row button:disabled {
            opacity: 0.55;
            cursor: not-allowed;
            transform: none;
        }
        .button-primary {
            background: linear-gradient(135deg, rgba(15, 118, 110, 0.14), rgba(15, 118, 110, 0.08));
        }
        .output-panel {
            display: grid;
            gap: 0.9rem;
            min-width: 0;
        }
        .status-row {
            display: flex;
            flex-wrap: wrap;
            gap: 0.6rem;
            align-items: center;
        }
        .status-pill {
            display: inline-flex;
            align-items: center;
            padding: 0.38rem 0.65rem;
            border-radius: 999px;
            background: rgba(15, 118, 110, 0.1);
            color: var(--accent);
            font-size: 0.84rem;
            font-weight: 700;
        }
        .status-pill.error {
            background: rgba(143, 53, 49, 0.12);
            color: var(--error);
        }
        .response-copy {
            margin: 0;
            color: var(--muted);
            line-height: 1.55;
        }
        .graph-shell {
            position: relative;
            border-radius: 30px;
            border: 1px solid rgba(31, 46, 42, 0.08);
            background: linear-gradient(180deg, rgba(255, 251, 245, 0.94), rgba(251, 247, 240, 0.84));
            box-shadow: var(--shadow);
            overflow: hidden;
        }
        .graph-lines {
            position: absolute;
            inset: 0;
            width: 100%;
            height: 100%;
            pointer-events: none;
            overflow: visible;
        }
        .graph-grid {
            position: relative;
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
            gap: 1rem;
            padding: 1.1rem;
        }
        .resource-card {
            position: relative;
            min-height: 19rem;
            border-radius: 22px;
            border: 1px solid rgba(31, 46, 42, 0.08);
            background: var(--panel);
            backdrop-filter: blur(14px);
            box-shadow: 0 12px 30px rgba(44, 51, 48, 0.08);
            padding: 1rem;
            cursor: pointer;
            transition: transform 140ms ease, border-color 140ms ease, box-shadow 140ms ease;
        }
        .resource-card:hover {
            transform: translateY(-2px);
        }
        .resource-card.is-selected {
            border-color: rgba(15, 118, 110, 0.45);
            box-shadow: 0 16px 34px rgba(15, 118, 110, 0.12);
        }
        .resource-head {
            display: flex;
            justify-content: space-between;
            gap: 1rem;
            align-items: flex-start;
        }
        .resource-name {
            margin: 0;
            font-size: 1.4rem;
            line-height: 1;
        }
        .kind {
            display: inline-flex;
            align-items: center;
            padding: 0.18rem 0.55rem;
            border-radius: 999px;
            background: var(--accent-soft);
            color: var(--accent);
            font-size: 0.74rem;
            text-transform: uppercase;
            letter-spacing: 0.12em;
            font-weight: 700;
        }
        .resource-meta {
            margin: 0.7rem 0 0;
            display: flex;
            flex-wrap: wrap;
            gap: 0.55rem;
        }
        .meta-pill {
            padding: 0.38rem 0.65rem;
            border-radius: 999px;
            background: rgba(255, 255, 255, 0.74);
            border: 1px solid rgba(31, 46, 42, 0.08);
            font-size: 0.84rem;
            color: var(--muted);
        }
        .label {
            margin: 1rem 0 0.45rem;
            font-size: 0.72rem;
            text-transform: uppercase;
            letter-spacing: 0.14em;
            color: var(--muted);
        }
        .chips {
            display: flex;
            flex-wrap: wrap;
            gap: 0.45rem;
        }
        .chip {
            padding: 0.35rem 0.55rem;
            border-radius: 999px;
            background: rgba(15, 118, 110, 0.08);
            color: #164e4a;
            font-size: 0.82rem;
        }
        .relation {
            background: rgba(194, 106, 61, 0.12);
            color: #8c4b27;
        }
        .columns {
            display: grid;
            gap: 0.45rem;
            margin-top: 0.4rem;
        }
        .column {
            display: flex;
            justify-content: space-between;
            gap: 0.75rem;
            padding: 0.55rem 0.65rem;
            border-radius: 14px;
            background: rgba(255, 255, 255, 0.74);
            border: 1px solid rgba(31, 46, 42, 0.06);
            font-size: 0.84rem;
        }
        .column strong {
            display: block;
            margin-bottom: 0.2rem;
        }
        .column small {
            color: var(--muted);
        }
        .samples {
            display: grid;
            gap: 0.55rem;
            margin-top: 0.4rem;
        }
        code.sample {
            display: block;
            padding: 0.65rem 0.75rem;
            border-radius: 14px;
            border: 1px solid rgba(31, 46, 42, 0.06);
            background: #f7f3ec;
            color: #2e3f3a;
            font-size: 0.8rem;
            line-height: 1.45;
            white-space: pre-wrap;
            word-break: break-word;
        }
        .empty {
            margin: 0;
            padding: 1rem;
            color: var(--muted);
        }
        .edge-line {
            fill: none;
            stroke-width: 2.2;
        }
        .edge-label {
            font-size: 12px;
            font-family: "IBM Plex Sans", "Avenir Next", "Segoe UI", sans-serif;
        }
        .explore-grid {
            display: grid;
            grid-template-columns: minmax(0, 1fr);
            gap: 1rem;
        }
        .inspector-head {
            display: flex;
            flex-wrap: wrap;
            gap: 0.75rem;
            justify-content: space-between;
            align-items: flex-start;
        }
        .inspector p {
            margin: 0.8rem 0 0;
            color: var(--muted);
            line-height: 1.6;
        }
        .inspector-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
            gap: 0.85rem;
            margin-top: 1rem;
        }
        .inspector-card {
            padding: 0.9rem;
            border-radius: 18px;
            background: rgba(255, 255, 255, 0.74);
            border: 1px solid rgba(31, 46, 42, 0.08);
        }
        .inspector-card h3 {
            margin: 0;
            font-size: 0.92rem;
        }
        .inspector-list {
            display: grid;
            gap: 0.5rem;
            margin-top: 0.7rem;
        }
        .mini-code {
            display: block;
            padding: 0.55rem 0.65rem;
            border-radius: 12px;
            background: #f7f3ec;
            border: 1px solid rgba(31, 46, 42, 0.08);
            font-family: "IBM Plex Mono", "SFMono-Regular", monospace;
            font-size: 0.78rem;
            line-height: 1.45;
            overflow-wrap: anywhere;
        }
        @media (max-width: 720px) {
            .page { padding-inline: 0.8rem; }
            .playground-shell { grid-template-columns: 1fr; }
            .resource-card { min-height: 0; }
            .graph-grid { grid-template-columns: 1fr; }
        }
        </style>"#,
    );
    html.push_str("</head><body><main class=\"page\">");
    html.push_str("<section class=\"hero\">");
    html.push_str("<p class=\"eyebrow\">folder-server</p>");
    html.push_str("<h1>Visual overview of your data</h1>");
    html.push_str("<p class=\"lede\">Use this page as both a route guide and a live explorer. It explains how folder-server maps paths to files, shows the rules around resource names and item paths, and lets you run example requests against the current data.</p>");
    html.push_str("<div class=\"stats\">");
    let _ = write!(
        html,
        "<span class=\"stat\">{} resources</span><span class=\"stat\">{} table links</span><span class=\"stat\">{} total rows</span><span class=\"stat\">Schema {}</span><span class=\"stat\">Source mode: {}</span>",
        page.resources.len(),
        page.edges.len(),
        total_rows,
        if page.schema_enabled { "loaded" } else { "not loaded" },
        escape_html(page.data_source_kind)
    );
    html.push_str("</div></section>");

    html.push_str("<section class=\"panel-grid\">");
    html.push_str("<article class=\"info-panel\">");
    html.push_str("<p class=\"section-title\">Rules of paths</p>");
    html.push_str("<h2 class=\"panel-title\">How folder-server derives routes</h2>");
    let _ = write!(
        html,
        "<p class=\"lede-tight\">{}</p><code class=\"source-line\">{}</code><p class=\"lede-tight\">{}</p>",
        escape_html(&page.source_rule),
        escape_html(&page.source_label),
        escape_html(page.resource_name_rule),
    );
    html.push_str("<div class=\"rule-grid\">");
    render_rule_card(
        &mut html,
        "Resource index",
        "GET",
        "/",
        "Lists all resources as JSON for API clients and renders this guide for browsers.",
    );
    render_rule_card(
        &mut html,
        "Collection or object",
        "GET",
        &sample_collection_path,
        "Reads the full JSON resource. Arrays stay arrays unless pagination is requested.",
    );
    render_rule_card(
        &mut html,
        "Single item",
        "GET",
        &sample_item_path,
        "Works for array resources whose rows are objects with an `id` field.",
    );
    render_rule_card(
        &mut html,
        "SQL endpoint",
        "GET",
        "/sql?q=SELECT%20*%20FROM%20resource",
        "Runs query-style reads when you need projection, joins, or aggregate-style access.",
    );
    html.push_str("</div></article>");

    html.push_str("<article class=\"info-panel\">");
    html.push_str("<p class=\"section-title\">Query options</p>");
    html.push_str("<h2 class=\"panel-title\">Filters, sorting, pagination, and embeds</h2>");
    html.push_str("<div class=\"rule-grid\">");
    render_rule_card(
        &mut html,
        "Filtering",
        "GET",
        &sample_filter_path,
        "Basic filters use `field=value`. Advanced filters use `field:operator=value`.",
    );
    render_rule_card(
        &mut html,
        "Sorting",
        "GET",
        &sample_sort_path,
        "Use `sort` or `_sort`; prefix a field with `-` for descending order.",
    );
    render_rule_card(
        &mut html,
        "Pagination",
        "GET",
        &sample_page_path,
        "Use `page` and `per_page` to get a metadata envelope with navigation links.",
    );
    render_rule_card(
        &mut html,
        "Embedding",
        "GET",
        &sample_embed_path,
        "Use `embed` only when a schema defines foreign keys for that resource.",
    );
    html.push_str("</div>");
    html.push_str("<div class=\"note-list\">");
    html.push_str("<div class=\"note-item\">Array resources support item routes like <span class=\"inline-code\">/{resource}/{id}</span>. Object resources stay on <span class=\"inline-code\">/{resource}</span>.</div>");
    html.push_str("<div class=\"note-item\">Write routes exist on the same paths for mutable mode: arrays use <span class=\"inline-code\">POST /{resource}</span> and <span class=\"inline-code\">PUT/PATCH/DELETE /{resource}/{id}</span>; object resources use <span class=\"inline-code\">PUT/PATCH /{resource}</span>.</div>");
    html.push_str("<div class=\"note-item\">When a DBML schema is loaded, resources must match table names and embed paths follow declared foreign-key columns.</div>");
    html.push_str("</div></article></section>");

    html.push_str("<p class=\"section-title\">Interactive playground</p>");
    html.push_str("<section class=\"playground-shell\">");
    html.push_str("<div class=\"controls\">");
    html.push_str("<div class=\"control\"><label for=\"resource-select\">Resource</label><select id=\"resource-select\">");
    if page.resources.is_empty() {
        html.push_str("<option value=\"\">No resources loaded</option>");
    } else {
        for resource in &page.resources {
            let _ = write!(
                html,
                "<option value=\"{value}\">{value}</option>",
                value = escape_html(&resource.name)
            );
        }
    }
    html.push_str("</select></div>");
    html.push_str(
        "<div class=\"control\"><label for=\"route-kind\">Path preset</label><select id=\"route-kind\"><option value=\"collection\">Read full resource</option><option value=\"item\">Read single item</option><option value=\"filter\">Filter rows</option><option value=\"sort\">Sort rows</option><option value=\"page\">Paginate rows</option><option value=\"embed\">Embed related rows</option></select></div>",
    );
    html.push_str(
        "<div class=\"control\"><label for=\"item-id\">Item id</label><input id=\"item-id\" type=\"text\" placeholder=\"Uses a sampled id when available\"></div>",
    );
    html.push_str(
        "<div class=\"control\"><label for=\"custom-query\">Extra query string</label><textarea id=\"custom-query\" placeholder=\"Optional. Example: status=draft&sort=-created_at\"></textarea></div>",
    );
    html.push_str("<div class=\"button-row\"><button type=\"button\" id=\"copy-path\">Copy path</button><button type=\"button\" id=\"run-request\" class=\"button-primary\">Run request</button></div>");
    html.push_str("</div>");
    html.push_str("<div class=\"output-panel\">");
    html.push_str("<div class=\"status-row\"><span id=\"playground-status\" class=\"status-pill\">Ready</span><code id=\"playground-path\" class=\"generated-url\">/</code></div>");
    html.push_str("<p id=\"playground-copy\" class=\"response-copy\">Choose a resource and preset to generate a request path, then run it to preview the live JSON response.</p>");
    html.push_str("<pre id=\"playground-response\"><code class=\"sample\">Select a resource to inspect the live response.</code></pre>");
    html.push_str("</div></section>");

    html.push_str("<p class=\"section-title\">Relationship map</p>");
    html.push_str("<section class=\"graph-shell\">");
    if page.resources.is_empty() {
        html.push_str("<p class=\"empty\">No resources found yet. Add JSON files to the configured folder and refresh this page.</p>");
    } else {
        html.push_str("<svg class=\"graph-lines\" id=\"graph-lines\" aria-hidden=\"true\"></svg>");
        html.push_str("<div class=\"graph-grid\" id=\"graph-grid\">");
        for resource in &page.resources {
            let _ = write!(
                html,
                "<article class=\"resource-card graph-node{}\" data-resource=\"{}\" tabindex=\"0\" role=\"button\" aria-label=\"Inspect resource {}\"><div class=\"resource-head\"><div><div class=\"kind\">{}</div><h2 class=\"resource-name\">{}</h2></div></div>",
                if page.resources.first().map(|first| first.name == resource.name).unwrap_or(false)
                {
                    " is-selected"
                } else {
                    ""
                },
                escape_html(&resource.name),
                escape_html(&resource.name),
                escape_html(resource.kind),
                escape_html(&resource.name)
            );
            let _ = write!(
                html,
                "<div class=\"resource-meta\"><span class=\"meta-pill\">{}</span><span class=\"meta-pill\">{}</span></div>",
                escape_html(&resource.count_label),
                escape_html(&resource.detail_label)
            );

            if !resource.columns.is_empty() {
                html.push_str("<p class=\"label\">Schema columns</p><div class=\"columns\">");
                for column in &resource.columns {
                    let relation = column
                        .relation
                        .as_ref()
                        .map(|value| format!("refs {}", escape_html(value)))
                        .unwrap_or_else(|| "no relation".to_string());
                    let _ = write!(
                        html,
                        "<div class=\"column\"><div><strong>{}</strong><small>{}</small></div><div><small>{} · {}</small></div></div>",
                        escape_html(&column.name),
                        escape_html(column.column_type),
                        if column.nullable { "nullable" } else { "required" },
                        relation
                    );
                }
                html.push_str("</div>");
            } else if !resource.field_names.is_empty() {
                html.push_str("<p class=\"label\">Observed fields</p><div class=\"chips\">");
                for field in &resource.field_names {
                    let _ = write!(html, "<span class=\"chip\">{}</span>", escape_html(field));
                }
                html.push_str("</div>");
            }

            if !resource.outgoing_relations.is_empty() || !resource.incoming_relations.is_empty() {
                html.push_str("<p class=\"label\">Connections</p><div class=\"chips\">");
                for relation in &resource.outgoing_relations {
                    let _ = write!(
                        html,
                        "<span class=\"chip relation\">{}</span>",
                        escape_html(relation)
                    );
                }
                for relation in &resource.incoming_relations {
                    let _ = write!(html, "<span class=\"chip\">{}</span>", escape_html(relation));
                }
                html.push_str("</div>");
            }

            if !resource.row_samples.is_empty() {
                html.push_str("<p class=\"label\">Sample content</p><div class=\"samples\">");
                for sample in &resource.row_samples {
                    let _ = write!(html, "<code class=\"sample\">{}</code>", escape_html(sample));
                }
                html.push_str("</div>");
            }

            html.push_str("</article>");
        }
        html.push_str("</div>");
    }
    html.push_str("</section>");

    html.push_str("<section class=\"explore-grid\">");
    html.push_str("<article class=\"inspector\">");
    html.push_str("<p class=\"section-title\">Resource explorer</p>");
    html.push_str("<div id=\"resource-inspector\"><p class=\"empty\">Select a resource card above to inspect its fields, relationships, and route examples.</p></div>");
    html.push_str("</article></section>");

    html.push_str(
        r#"<script>
        (() => {
            const page = "#,
    );
    html.push_str(&page_json);
    html.push_str(
        r#";
            const resources = Array.isArray(page.resources) ? page.resources : [];
            const edges = Array.isArray(page.edges) ? page.edges : [];
            const grid = document.getElementById('graph-grid');
            const svg = document.getElementById('graph-lines');
            const resourceSelect = document.getElementById('resource-select');
            const routeKind = document.getElementById('route-kind');
            const itemIdInput = document.getElementById('item-id');
            const customQuery = document.getElementById('custom-query');
            const playgroundStatus = document.getElementById('playground-status');
            const playgroundCopy = document.getElementById('playground-copy');
            const playgroundPath = document.getElementById('playground-path');
            const playgroundResponse = document.getElementById('playground-response');
            const copyButton = document.getElementById('copy-path');
            const runButton = document.getElementById('run-request');
            const inspector = document.getElementById('resource-inspector');
            const cardNodes = grid ? Array.from(grid.querySelectorAll('.graph-node')) : [];
            const cards = new Map(cardNodes.map((node) => [node.dataset.resource, node]));
            const escapeHtml = (value) => String(value)
                .replaceAll('&', '&amp;')
                .replaceAll('<', '&lt;')
                .replaceAll('>', '&gt;')
                .replaceAll('"', '&quot;')
                .replaceAll("'", '&#39;');
            const uniqueFields = (resource) => {
                const fields = [];
                for (const column of resource.columns || []) {
                    if (column && column.name && !fields.includes(column.name)) fields.push(column.name);
                }
                for (const field of resource.field_names || []) {
                    if (field && !fields.includes(field)) fields.push(field);
                }
                return fields;
            };
            const firstFilterField = (resource) => uniqueFields(resource).find((field) => field !== 'id') || uniqueFields(resource)[0] || 'field';
            const firstEmbedField = (resource) => {
                const column = (resource.columns || []).find((entry) => entry && entry.relation);
                return column ? column.name : null;
            };
            const getResource = (name) => resources.find((resource) => resource.name === name);
            let selectedResourceName = resources[0] ? resources[0].name : '';

            if (!resources.length) {
                if (resourceSelect) resourceSelect.disabled = true;
                if (routeKind) routeKind.disabled = true;
                if (itemIdInput) itemIdInput.disabled = true;
                if (customQuery) customQuery.disabled = true;
                if (copyButton) copyButton.disabled = true;
                if (runButton) runButton.disabled = true;
            }

            const buildPlaygroundState = () => {
                const resource = getResource(selectedResourceName);
                if (!resource) {
                    return {
                        path: '/',
                        note: 'No resource is available yet. Add JSON data and refresh the page.',
                        canRun: false
                    };
                }

                const preset = routeKind ? routeKind.value : 'collection';
                const sampledId = resource.sample_item_id || '';
                const explicitId = itemIdInput ? itemIdInput.value.trim() : '';
                const itemId = explicitId || sampledId;
                const filterField = firstFilterField(resource);
                const embedField = firstEmbedField(resource);
                let path = `/${resource.name}`;
                let note = `Read the full ${resource.name} resource.`;
                let canRun = true;

                if (preset === 'item') {
                    if (!itemId) {
                        canRun = false;
                        note = 'This preset needs an item `id`. The selected resource has no sampled id yet.';
                    } else {
                        path = `/${resource.name}/${itemId}`;
                        note = `Read the single item identified by \`${itemId}\`.`;
                    }
                } else if (preset === 'filter') {
                    path = `/${resource.name}?${filterField}=value`;
                    note = `Filter rows using \`${filterField}=value\` or advanced operators such as \`${filterField}:gt=10\`.`;
                } else if (preset === 'sort') {
                    path = `/${resource.name}?sort=-${filterField}`;
                    note = `Sort rows by \`${filterField}\`, descending because of the leading \`-\`.`;
                } else if (preset === 'page') {
                    path = `/${resource.name}?page=1&per_page=10`;
                    note = 'Paginate an array resource and receive metadata plus the current page of rows.';
                } else if (preset === 'embed') {
                    if (!embedField) {
                        canRun = false;
                        note = 'Embed is available only for schema-backed foreign key columns.';
                    } else {
                        path = `/${resource.name}?embed=${embedField}`;
                        note = `Replace the \`${embedField}\` foreign key with the related object.`;
                    }
                }

                const extraQueryString = customQuery ? customQuery.value.trim().replace(/^\?/, '') : '';
                if (extraQueryString) {
                    path += path.includes('?') ? `&${extraQueryString}` : `?${extraQueryString}`;
                    note += ' Extra query parameters were appended exactly as entered.';
                }

                return { path, note, canRun };
            };

            const renderInspector = () => {
                if (!inspector) return;
                const resource = getResource(selectedResourceName);
                if (!resource) {
                    inspector.innerHTML = '<p class="empty">No resources loaded yet.</p>';
                    return;
                }

                const filterField = firstFilterField(resource);
                const embedField = firstEmbedField(resource);
                const routeExamples = [
                    `GET /${resource.name}`,
                    resource.sample_item_id ? `GET /${resource.name}/${resource.sample_item_id}` : null,
                    resource.kind === 'table' ? `GET /${resource.name}?${filterField}=value` : null,
                    resource.kind === 'table' ? `GET /${resource.name}?page=1&per_page=10` : null,
                    embedField ? `GET /${resource.name}?embed=${embedField}` : null
                ].filter(Boolean);
                const relationRows = [...(resource.outgoing_relations || []), ...(resource.incoming_relations || [])];
                const fieldRows = uniqueFields(resource);
                const sampleRows = (resource.row_samples || []).length
                    ? resource.row_samples
                    : ['No sample rows are available for this resource yet.'];

                inspector.innerHTML = `
                    <div class="inspector-head">
                        <div>
                            <h2 class="panel-title">${escapeHtml(resource.name)}</h2>
                            <p>${escapeHtml(resource.count_label)} · ${escapeHtml(resource.detail_label)} · ${escapeHtml(resource.kind)}</p>
                        </div>
                        <span class="kind">${escapeHtml(resource.kind)}</span>
                    </div>
                    <p>${escapeHtml(resource.kind === 'table'
                        ? 'Array resource. Use the collection route for full reads, then move to item/filter/page presets for focused inspection.'
                        : 'Non-array resource. It stays on the collection path and does not expose item-level routes unless the JSON shape changes.')}</p>
                    <div class="inspector-grid">
                        <div class="inspector-card">
                            <h3>Fields</h3>
                            <div class="inspector-list">
                                ${fieldRows.length ? fieldRows.map((field) => `<span class="chip">${escapeHtml(field)}</span>`).join('') : '<span class="empty">No fields detected.</span>'}
                            </div>
                        </div>
                        <div class="inspector-card">
                            <h3>Path examples</h3>
                            <div class="inspector-list">
                                ${routeExamples.map((route) => `<code class="mini-code">${escapeHtml(route)}</code>`).join('')}
                            </div>
                        </div>
                        <div class="inspector-card">
                            <h3>Relationships</h3>
                            <div class="inspector-list">
                                ${relationRows.length ? relationRows.map((relation) => `<code class="mini-code">${escapeHtml(relation)}</code>`).join('') : '<span class="empty">No schema relation is attached to this resource.</span>'}
                            </div>
                        </div>
                        <div class="inspector-card">
                            <h3>Sample content</h3>
                            <div class="inspector-list">
                                ${sampleRows.map((row) => `<code class="mini-code">${escapeHtml(row)}</code>`).join('')}
                            </div>
                        </div>
                    </div>
                `;
            };

            const setStatus = (text, isError = false) => {
                if (!playgroundStatus) return;
                playgroundStatus.textContent = text;
                playgroundStatus.classList.toggle('error', isError);
            };

            const renderPlayground = () => {
                const state = buildPlaygroundState();
                if (playgroundPath) playgroundPath.textContent = state.path;
                if (playgroundCopy) playgroundCopy.textContent = state.note;
                if (copyButton) copyButton.disabled = !state.canRun;
                if (runButton) runButton.disabled = !state.canRun;
                setStatus(state.canRun ? 'Ready' : 'Needs input', !state.canRun);
            };

            const setSelectedResource = (name) => {
                selectedResourceName = name;
                if (resourceSelect) resourceSelect.value = name;
                for (const [resourceName, node] of cards.entries()) {
                    node.classList.toggle('is-selected', resourceName === name);
                }
                renderInspector();
                renderPlayground();
                window.requestAnimationFrame(draw);
            };

            const draw = () => {
                if (!grid || !svg) return;
                const nodes = new Map(Array.from(grid.querySelectorAll('.graph-node')).map((node) => [node.dataset.resource, node]));
                const rect = grid.getBoundingClientRect();
                const width = Math.max(grid.scrollWidth, Math.ceil(rect.width));
                const height = Math.max(grid.scrollHeight, Math.ceil(grid.getBoundingClientRect().height));
                svg.setAttribute('viewBox', `0 0 ${width} ${height}`);
                svg.setAttribute('width', String(width));
                svg.setAttribute('height', String(height));
                svg.innerHTML = '<defs><marker id="edge-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z" fill="rgba(15, 118, 110, 0.38)"></path></marker></defs>';

                for (const edge of edges) {
                    const source = nodes.get(edge.source_table);
                    const target = nodes.get(edge.target_table);
                    if (!source || !target) continue;

                    const sourceRect = source.getBoundingClientRect();
                    const targetRect = target.getBoundingClientRect();
                    const x1 = sourceRect.left - rect.left + sourceRect.width / 2;
                    const y1 = sourceRect.top - rect.top + sourceRect.height / 2;
                    const x2 = targetRect.left - rect.left + targetRect.width / 2;
                    const y2 = targetRect.top - rect.top + targetRect.height / 2;
                    const bend = Math.max(48, Math.abs(x2 - x1) * 0.32);
                    const isSelected = edge.source_table === selectedResourceName || edge.target_table === selectedResourceName;
                    const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
                    path.setAttribute('class', 'edge-line');
                    path.setAttribute('marker-end', 'url(#edge-arrow)');
                    path.setAttribute('stroke', isSelected ? 'rgba(15, 118, 110, 0.72)' : 'rgba(15, 118, 110, 0.32)');
                    path.setAttribute('d', `M ${x1} ${y1} C ${x1 + bend} ${y1}, ${x2 - bend} ${y2}, ${x2} ${y2}`);
                    svg.appendChild(path);

                    const label = document.createElementNS('http://www.w3.org/2000/svg', 'text');
                    label.setAttribute('class', 'edge-label');
                    label.setAttribute('x', String((x1 + x2) / 2));
                    label.setAttribute('y', String((y1 + y2) / 2 - 8));
                    label.setAttribute('text-anchor', 'middle');
                    label.setAttribute('fill', isSelected ? '#0f766e' : '#25645d');
                    label.setAttribute('opacity', isSelected ? '1' : '0.78');
                    label.textContent = `${edge.source_column} -> ${edge.target_table}.${edge.target_column}`;
                    svg.appendChild(label);
                }
            };

            const redraw = () => window.requestAnimationFrame(draw);

            if (resourceSelect) {
                resourceSelect.addEventListener('change', (event) => {
                    setSelectedResource(event.target.value);
                });
            }
            if (routeKind) routeKind.addEventListener('change', renderPlayground);
            if (itemIdInput) itemIdInput.addEventListener('input', renderPlayground);
            if (customQuery) customQuery.addEventListener('input', renderPlayground);

            for (const [name, node] of cards.entries()) {
                node.addEventListener('click', () => setSelectedResource(name));
                node.addEventListener('keydown', (event) => {
                    if (event.key === 'Enter' || event.key === ' ') {
                        event.preventDefault();
                        setSelectedResource(name);
                    }
                });
            }

            if (copyButton) {
                copyButton.addEventListener('click', async () => {
                    const state = buildPlaygroundState();
                    if (!state.canRun) return;
                    try {
                        await navigator.clipboard.writeText(state.path);
                        setStatus('Path copied');
                    } catch (_) {
                        setStatus('Copy failed', true);
                    }
                });
            }

            if (runButton) {
                runButton.addEventListener('click', async () => {
                    const state = buildPlaygroundState();
                    if (!state.canRun) return;
                    setStatus('Loading...');
                    if (playgroundResponse) {
                        playgroundResponse.innerHTML = '<code class="sample">Fetching live JSON response...</code>';
                    }
                    try {
                        const response = await fetch(state.path, {
                            headers: { Accept: 'application/json' }
                        });
                        const text = await response.text();
                        let formatted = text || '(empty response body)';
                        try {
                            formatted = JSON.stringify(JSON.parse(text), null, 2);
                        } catch (_) {}
                        if (playgroundResponse) {
                            playgroundResponse.innerHTML = `<code class="sample">${escapeHtml(formatted)}</code>`;
                        }
                        setStatus(`${response.status} ${response.statusText}`, !response.ok);
                    } catch (error) {
                        if (playgroundResponse) {
                            playgroundResponse.innerHTML = `<code class="sample">${escapeHtml(String(error))}</code>`;
                        }
                        setStatus('Request failed', true);
                    }
                });
            }

            window.addEventListener('load', redraw, { once: true });
            window.addEventListener('resize', redraw);

            if (selectedResourceName) {
                setSelectedResource(selectedResourceName);
            } else {
                renderInspector();
                renderPlayground();
            }
            redraw();
        })();
        </script>"#,
    );
    html.push_str("</main></body></html>");
    html
}

fn render_rule_card(html: &mut String, title: &str, method: &str, path: &str, copy: &str) {
    let _ = write!(
        html,
        "<article class=\"rule-card\"><p class=\"rule-title\">{}</p><code class=\"path-code\"><span class=\"method\">{}</span> {}</code><p class=\"rule-copy\">{}</p></article>",
        escape_html(title),
        escape_html(method),
        escape_html(path),
        escape_html(copy),
    );
}

fn escape_html(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}
