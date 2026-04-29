use std::{
    collections::{BTreeSet, HashMap},
    fmt::Write,
};

use axum::{
    Json,
    http::{
        HeaderMap, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE},
    },
    response::{Html, IntoResponse, Response},
};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::{
    app::{AppState, DataSource},
    error::AppError,
    schema::{ColumnType, TableSchema, primary_key_name},
    storage::load_resource,
};

const OVERVIEW_CSS: &str = include_str!("../../ui/dist/overview.css");
const OVERVIEW_JS: &str = include_str!("../../ui/dist/overview.js");

#[derive(Clone, Debug, Serialize)]
pub struct OverviewPageData {
    pub schema_enabled: bool,
    pub server_capabilities: ServerCapabilities,
    pub data_source_kind: &'static str,
    pub source_label: String,
    pub source_rule: String,
    pub resource_name_rule: &'static str,
    pub stats: OverviewStats,
    pub resources: Vec<ResourceOverview>,
    pub edges: Vec<OverviewEdge>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ServerCapabilities {
    pub readonly: bool,
    pub resource_write: bool,
    pub schema_write: bool,
    pub schema_infer: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct OverviewStats {
    pub resource_count: usize,
    pub relation_count: usize,
    pub total_rows: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct ResourceOverview {
    pub name: String,
    pub kind: &'static str,
    pub row_count: Option<usize>,
    pub key_count: Option<usize>,
    pub primary_key: Option<String>,
    pub field_names: Vec<String>,
    pub row_samples: Vec<Value>,
    pub columns: Vec<OverviewColumn>,
    pub outgoing_relations: Vec<OverviewRelation>,
    pub incoming_relations: Vec<OverviewRelation>,
    pub many_to_many_relations: Vec<OverviewManyToManyRelation>,
    pub sample_item_id: Option<String>,
    pub query_capabilities: QueryCapabilities,
    pub mutation_capabilities: MutationCapabilities,
}

#[derive(Clone, Debug, Serialize)]
pub struct OverviewColumn {
    pub name: String,
    pub column_type: &'static str,
    pub nullable: bool,
    pub relation: Option<String>,
    pub is_primary_key: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct OverviewRelation {
    pub label: String,
    pub source_table: String,
    pub source_column: String,
    pub target_table: String,
    pub target_column: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct OverviewEdge {
    pub kind: &'static str,
    pub source_table: String,
    pub source_column: String,
    pub target_table: String,
    pub target_column: String,
    pub through_table: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct OverviewManyToManyRelation {
    pub label: String,
    pub source_table: String,
    pub source_column: String,
    pub source_target_column: String,
    pub through_table: String,
    pub through_target_column: String,
    pub target_table: String,
    pub target_column: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct QueryCapabilities {
    pub filter: bool,
    pub sort: bool,
    pub pagination: bool,
    pub embed: bool,
    pub item_route: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct MutationCapabilities {
    pub create_item: bool,
    pub update_item: bool,
    pub delete_item: bool,
    pub replace_object: bool,
    pub patch_object: bool,
}

pub fn request_prefers_html(headers: &HeaderMap) -> bool {
    headers
        .get(axum::http::header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .map(|accept| {
            accept.split(',').map(str::trim).any(|value| {
                value.starts_with("text/html") || value.starts_with("application/xhtml+xml")
            })
        })
        .unwrap_or(false)
}

pub async fn render_root_overview(
    state: &AppState,
    resources: &[String],
) -> Result<Html<String>, AppError> {
    let page = build_overview_page_data(state, resources).await?;
    Ok(Html(render_overview_html(&page)))
}

pub async fn get_overview_json(state: &AppState) -> Result<Json<OverviewPageData>, AppError> {
    let resources = state.resource_names_sorted().await;
    let page = build_overview_page_data(state, &resources).await?;
    Ok(Json(page))
}

pub fn overview_css() -> Response {
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "text/css; charset=utf-8"), (CACHE_CONTROL, "no-cache")],
        OVERVIEW_CSS,
    )
        .into_response()
}

pub fn overview_js() -> Response {
    (
        StatusCode::OK,
        [(CONTENT_TYPE, "text/javascript; charset=utf-8"), (CACHE_CONTROL, "no-cache")],
        OVERVIEW_JS,
    )
        .into_response()
}

async fn build_overview_page_data(
    state: &AppState,
    resources: &[String],
) -> Result<OverviewPageData, AppError> {
    let _guards = state.read_locks_for_resources(resources).await;
    let schema = state.schema_snapshot();
    let resource_set = resources.iter().cloned().collect::<BTreeSet<_>>();
    let mut incoming_relations: HashMap<String, Vec<OverviewRelation>> = HashMap::new();
    let mut outgoing_relations: HashMap<String, Vec<OverviewRelation>> = HashMap::new();
    let mut many_to_many_relations: HashMap<String, Vec<OverviewManyToManyRelation>> =
        HashMap::new();
    let mut edges = Vec::new();
    let mut many_to_many_edge_keys = BTreeSet::new();

    for (source_table, table) in &schema.tables {
        if !resource_set.contains(source_table) {
            continue;
        }
        for (source_column, fk) in &table.foreign_keys {
            if !resource_set.contains(&fk.target_table) {
                continue;
            }

            let relation = OverviewRelation {
                label: format!("{source_column} -> {}.{}", fk.target_table, fk.target_column),
                source_table: source_table.clone(),
                source_column: source_column.clone(),
                target_table: fk.target_table.clone(),
                target_column: fk.target_column.clone(),
            };
            outgoing_relations.entry(source_table.clone()).or_default().push(relation.clone());
            incoming_relations.entry(fk.target_table.clone()).or_default().push(relation.clone());
            edges.push(OverviewEdge {
                kind: "foreign_key",
                source_table: source_table.clone(),
                source_column: source_column.clone(),
                target_table: fk.target_table.clone(),
                target_column: fk.target_column.clone(),
                through_table: None,
            });
        }

        for relation in table.many_to_many.values() {
            if !resource_set.contains(&relation.target_table)
                || !resource_set.contains(&relation.through_table)
            {
                continue;
            }

            let derived = OverviewManyToManyRelation {
                label: format!("{} via {}", relation.target_table, relation.through_table),
                source_table: source_table.clone(),
                source_column: relation.source_column.clone(),
                source_target_column: relation.source_target_column.clone(),
                through_table: relation.through_table.clone(),
                through_target_column: relation.through_target_column.clone(),
                target_table: relation.target_table.clone(),
                target_column: relation.target_column.clone(),
            };
            many_to_many_relations.entry(source_table.clone()).or_default().push(derived);

            let left_table = source_table.min(&relation.target_table).clone();
            let right_table = source_table.max(&relation.target_table).clone();
            let edge_key = format!("{left_table}:{right_table}:{}", relation.through_table);
            if many_to_many_edge_keys.insert(edge_key) {
                edges.push(OverviewEdge {
                    kind: "many_to_many",
                    source_table: left_table,
                    source_column: relation.source_column.clone(),
                    target_table: right_table,
                    target_column: relation.target_column.clone(),
                    through_table: Some(relation.through_table.clone()),
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
    let mut total_rows = 0usize;
    for resource in resources {
        let value = load_resource(state, resource).await?;
        let table_schema = schema.tables.get(resource.as_str());
        let summary = summarize_resource_value(value.as_ref(), table_schema);
        total_rows += summary.row_count.unwrap_or(0);
        summaries.push(ResourceOverview {
            name: resource.clone(),
            kind: summary.kind,
            row_count: summary.row_count,
            key_count: summary.key_count,
            primary_key: summary.primary_key,
            field_names: summary.field_names,
            row_samples: summary.row_samples,
            columns: summary.columns,
            outgoing_relations: outgoing_relations.remove(resource).unwrap_or_default(),
            incoming_relations: incoming_relations.remove(resource).unwrap_or_default(),
            many_to_many_relations: many_to_many_relations.remove(resource).unwrap_or_default(),
            sample_item_id: summary.sample_item_id,
            query_capabilities: summary.query_capabilities,
            mutation_capabilities: summary.mutation_capabilities,
        });
    }

    Ok(OverviewPageData {
        schema_enabled: !schema.tables.is_empty(),
        server_capabilities: ServerCapabilities {
            readonly: state.config.readonly,
            resource_write: !state.config.readonly,
            schema_write: !state.config.readonly,
            schema_infer: !state.config.readonly,
        },
        data_source_kind,
        source_label,
        source_rule,
        resource_name_rule: "Resource names may only use letters, numbers, `_`, and `-`.",
        stats: OverviewStats {
            resource_count: summaries.len(),
            relation_count: edges.len(),
            total_rows,
        },
        resources: summaries,
        edges,
    })
}

struct ResourceSummary {
    kind: &'static str,
    row_count: Option<usize>,
    key_count: Option<usize>,
    primary_key: Option<String>,
    field_names: Vec<String>,
    row_samples: Vec<Value>,
    columns: Vec<OverviewColumn>,
    sample_item_id: Option<String>,
    query_capabilities: QueryCapabilities,
    mutation_capabilities: MutationCapabilities,
}

fn summarize_resource_value(value: &Value, table: Option<&TableSchema>) -> ResourceSummary {
    let columns = table
        .map(|table| {
            let primary_key = primary_key_name(Some(table));
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
                    is_primary_key: primary_key == column_name,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    match value {
        Value::Array(items) => {
            let mut field_names = BTreeSet::new();
            for item in items.iter().take(24) {
                if let Some(object) = item.as_object() {
                    for key in object.keys() {
                        field_names.insert(key.clone());
                        if field_names.len() >= 24 {
                            break;
                        }
                    }
                }
                if field_names.len() >= 24 {
                    break;
                }
            }
            let sample_item_id = sample_item_id(value, table);
            ResourceSummary {
                kind: "table",
                row_count: Some(items.len()),
                key_count: None,
                primary_key: Some(primary_key_name(table).to_string()),
                field_names: field_names.into_iter().collect(),
                row_samples: items.iter().take(2).cloned().collect(),
                columns,
                sample_item_id: sample_item_id.clone(),
                query_capabilities: QueryCapabilities {
                    filter: true,
                    sort: true,
                    pagination: true,
                    embed: table.is_some_and(|schema| !schema.foreign_keys.is_empty()),
                    item_route: sample_item_id.is_some(),
                },
                mutation_capabilities: MutationCapabilities {
                    create_item: true,
                    update_item: sample_item_id.is_some(),
                    delete_item: sample_item_id.is_some(),
                    replace_object: false,
                    patch_object: false,
                },
            }
        }
        Value::Object(object) => ResourceSummary {
            kind: "object",
            row_count: None,
            key_count: Some(object.len()),
            primary_key: None,
            field_names: object.keys().take(24).cloned().collect(),
            row_samples: vec![value.clone()],
            columns,
            sample_item_id: None,
            query_capabilities: QueryCapabilities {
                filter: false,
                sort: false,
                pagination: false,
                embed: false,
                item_route: false,
            },
            mutation_capabilities: MutationCapabilities {
                create_item: false,
                update_item: false,
                delete_item: false,
                replace_object: true,
                patch_object: true,
            },
        },
        _ => ResourceSummary {
            kind: "value",
            row_count: None,
            key_count: None,
            primary_key: None,
            field_names: Vec::new(),
            row_samples: vec![value.clone()],
            columns,
            sample_item_id: None,
            query_capabilities: QueryCapabilities {
                filter: false,
                sort: false,
                pagination: false,
                embed: false,
                item_route: false,
            },
            mutation_capabilities: MutationCapabilities {
                create_item: false,
                update_item: false,
                delete_item: false,
                replace_object: false,
                patch_object: false,
            },
        },
    }
}

fn sample_item_id(value: &Value, table: Option<&TableSchema>) -> Option<String> {
    let item_key = primary_key_name(table);
    value
        .as_array()
        .and_then(|items| items.iter().find_map(|item| item.as_object()))
        .and_then(|object| object.get(item_key))
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

fn column_type_label(column_type: &ColumnType) -> &'static str {
    column_type.label()
}

fn sample_table_resource(page: &OverviewPageData) -> Option<&ResourceOverview> {
    page.resources.iter().find(|resource| resource.kind == "table")
}

fn sample_create_payload(resource: Option<&ResourceOverview>) -> String {
    let Some(resource) = resource else {
        return "{\n  \"name\": \"New item\"\n}".to_string();
    };

    if let Some(object) = resource.row_samples.iter().find_map(|value| value.as_object()) {
        let primary_key = resource.primary_key.as_deref().unwrap_or("id");
        let mut payload = Map::new();
        for (key, value) in object {
            if key == primary_key {
                continue;
            }
            payload.insert(key.clone(), value.clone());
            if payload.len() >= 3 {
                break;
            }
        }
        if !payload.is_empty() {
            return serde_json::to_string_pretty(&Value::Object(payload))
                .unwrap_or_else(|_| "{\n  \"name\": \"New item\"\n}".to_string());
        }
    }

    let field_name = resource
        .columns
        .iter()
        .map(|column| column.name.as_str())
        .chain(resource.field_names.iter().map(String::as_str))
        .find(|name| Some(*name) != resource.primary_key.as_deref() && *name != "id")
        .unwrap_or("name");
    let field_value = if field_name.contains("email") {
        Value::String("new@example.com".to_string())
    } else if field_name.contains("name")
        || field_name.contains("title")
        || field_name.contains("code")
    {
        Value::String("New item".to_string())
    } else {
        Value::String("value".to_string())
    };

    let mut payload = Map::new();
    payload.insert(field_name.to_string(), field_value);
    serde_json::to_string_pretty(&Value::Object(payload))
        .unwrap_or_else(|_| "{\n  \"name\": \"New item\"\n}".to_string())
}

fn render_empty_state_guide(html: &mut String, page: &OverviewPageData) {
    html.push_str("<article class=\"overview-help-card\">");
    html.push_str("<p class=\"section-title\">First data</p>");
    html.push_str("<h3>Create your first resource</h3>");
    match page.data_source_kind {
        "folder" => {
            let users_path = format!("{}/users.json", page.source_label.trim_end_matches('/'));
            let posts_path = format!("{}/posts.json", page.source_label.trim_end_matches('/'));
            let users_json = "[\n  {\"id\": 1, \"name\": \"Ada\"}\n]";
            let posts_json = "[\n  {\"id\": 1, \"title\": \"Hello\", \"user_id\": 1}\n]";
            let _ = write!(
                html,
                "<p class=\"overview-copy\">This folder is empty right now. Create one or two files like these, then refresh the page:</p>\
                 <code class=\"request-path\">{}</code><pre class=\"request-path\">{}</pre>\
                 <code class=\"request-path\">{}</code><pre class=\"request-path\">{}</pre>",
                escape_html(&users_path),
                escape_html(users_json),
                escape_html(&posts_path),
                escape_html(posts_json),
            );
        }
        _ => {
            let db_json = "{\n  \"users\": [\n    {\"id\": 1, \"name\": \"Ada\"}\n  ],\n  \"settings\": {\n    \"theme\": \"warm\"\n  }\n}";
            let _ = write!(
                html,
                "<p class=\"overview-copy\">This database file has no top-level resources yet. Start with a structure like this, then refresh the page:</p>\
                 <code class=\"request-path\">{}</code><pre class=\"request-path\">{}</pre>",
                escape_html(&page.source_label),
                escape_html(db_json),
            );
        }
    }
    html.push_str("</article>");
}

fn render_request_card(
    html: &mut String,
    title: &str,
    method: &str,
    path: &str,
    copy: &str,
    body: Option<&str>,
) {
    let _ = write!(
        html,
        "<article class=\"overview-help-card\"><p class=\"section-title\">{}</p><code class=\"request-path\"><span class=\"overview-method\">{}</span> {}</code>",
        escape_html(title),
        escape_html(method),
        escape_html(path),
    );
    if let Some(body) = body {
        let _ = write!(html, "<pre class=\"request-path\">{}</pre>", escape_html(body));
    }
    let _ = write!(html, "<p class=\"overview-copy\">{}</p></article>", escape_html(copy));
}

fn render_overview_html(page: &OverviewPageData) -> String {
    let sample_resource = page.resources.first();
    let sample_table_resource = sample_table_resource(page);
    let sample_collection_path = sample_resource
        .map(|resource| format!("/{}", resource.name))
        .unwrap_or_else(|| "/resource".to_string());
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
    let sample_page_path = format!("{sample_collection_path}?page=1&per_page=25");
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
    let sample_create_path = sample_table_resource
        .map(|resource| format!("/{}", resource.name))
        .unwrap_or_else(|| sample_collection_path.clone());
    let sample_create_form_path = sample_table_resource
        .map(|resource| format!("/{}/create", resource.name))
        .unwrap_or_else(|| format!("{sample_collection_path}/create"));
    let sample_create_body = sample_create_payload(sample_table_resource);
    let capability_note = if page.server_capabilities.readonly {
        "This server is in readonly mode. Browse routes, inspect rows, copy request URLs, and use /graphql, but mutations and schema writes are disabled."
    } else {
        "This server is writable. Use the overview to create, edit, and delete rows, patch object resources, and persist schema changes when you want explicit primary keys or relationships."
    };

    let mut html = String::new();
    html.push_str(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
    );
    html.push_str("<title>dirbase overview</title>");
    html.push_str("<link rel=\"stylesheet\" href=\"/assets/overview.css\">");
    html.push_str("</head><body><main class=\"overview-page\">");
    html.push_str("<section class=\"shell-card overview-root-hero\">");
    html.push_str("<div class=\"overview-root-title\">");
    html.push_str("<p class=\"overview-eyebrow\">dirbase</p>");
    html.push_str("<h1>Data workspace</h1>");
    html.push_str("<p class=\"overview-lede\">Use the app below to browse resources, inspect request URLs, and drill through live relationships without leaving the page.</p>");
    html.push_str("</div>");
    html.push_str("<div class=\"overview-status-group\">");
    let _ = write!(
        html,
        "<span class=\"overview-inline-badge\">{} resources</span><span class=\"overview-inline-badge\">{} relations</span><span class=\"overview-inline-badge\">{} rows</span><span class=\"overview-inline-badge\">Source mode: {}</span><span class=\"status-pill\">Schema {}</span>",
        page.stats.resource_count,
        page.stats.relation_count,
        page.stats.total_rows,
        escape_html(page.data_source_kind),
        if page.schema_enabled { "loaded" } else { "not loaded" },
    );
    if page.server_capabilities.readonly {
        html.push_str("<span class=\"status-pill is-warn\">Read-only mode</span>");
    } else {
        html.push_str("<span class=\"status-pill\">Writable mode</span>");
    }
    html.push_str("</div>");
    let _ = write!(
        html,
        "<code class=\"overview-source-line\">{}</code>",
        escape_html(&page.source_label),
    );
    html.push_str("</section>");

    html.push_str(
        "<section class=\"shell-card\"><div id=\"overview-root\" data-overview-endpoint=\"/overview.json\"></div><noscript>",
    );
    html.push_str("<div class=\"noscript-shell\"><p class=\"section-title\">Overview fallback</p><h2>Resources</h2><p class=\"overview-copy\">JavaScript is disabled, so the interactive explorer is unavailable. The compact resource list remains visible below.</p>");
    if page.resources.is_empty() {
        html.push_str("<p class=\"overview-empty\">No resources found yet. Use the help panel below to add the first JSON files.</p>");
    } else {
        html.push_str("<div class=\"noscript-resource-grid\">");
        for resource in &page.resources {
            let _ = write!(
                html,
                "<article class=\"noscript-resource-card\" data-resource=\"{}\"><div class=\"overview-panel-head\"><h3>{}</h3><span class=\"overview-kind-badge\">{}</span></div>",
                escape_html(&resource.name),
                escape_html(&resource.name),
                escape_html(resource.kind),
            );
            if let Some(row_count) = resource.row_count {
                let _ = write!(
                    html,
                    "<p class=\"overview-copy\"><strong>{}</strong> rows · primary key <span class=\"overview-inline-code\">{}</span></p>",
                    row_count,
                    escape_html(resource.primary_key.as_deref().unwrap_or("id")),
                );
            } else if let Some(key_count) = resource.key_count {
                let _ = write!(
                    html,
                    "<p class=\"overview-copy\"><strong>{}</strong> top-level keys</p>",
                    key_count
                );
            } else {
                html.push_str("<p class=\"overview-copy\">Scalar JSON value</p>");
            }

            if !resource.field_names.is_empty() {
                html.push_str("<div class=\"resource-field-list\">");
                for field in resource.field_names.iter().take(4) {
                    let _ = write!(
                        html,
                        "<span class=\"resource-field-pill\">{}</span>",
                        escape_html(field)
                    );
                }
                html.push_str("</div>");
            }

            let _ = write!(
                html,
                "<p class=\"overview-copy\">Collection route: <a href=\"/{name}\"><code class=\"overview-inline-code\">/{name}</code></a></p>",
                name = escape_html(&resource.name),
            );
            if let Some(sample_item_id) = &resource.sample_item_id {
                let _ = write!(
                    html,
                    "<p class=\"overview-copy\">Sample item: <a href=\"/{name}/{id}\"><code class=\"overview-inline-code\">/{name}/{id}</code></a></p>",
                    name = escape_html(&resource.name),
                    id = escape_html(sample_item_id),
                );
            }
            if resource.kind == "table" {
                let _ = write!(
                    html,
                    "<p class=\"overview-copy\">Create form: <a href=\"/{name}/create\"><code class=\"overview-inline-code\">/{name}/create</code></a></p>",
                    name = escape_html(&resource.name),
                );
            }
            html.push_str("</article>");
        }
        html.push_str("</div>");
    }
    html.push_str("</div></noscript></section>");

    if page.resources.is_empty() {
        html.push_str("<section class=\"shell-card\">");
        html.push_str("<details class=\"overview-help\" open>");
        html.push_str("<summary><p class=\"section-title\">Help</p><h2>Routes and quick checks</h2><p class=\"overview-copy\">Start with one or two files, then use the app above to explore them.</p></summary>");
        html.push_str("<div class=\"overview-help-grid\">");
        render_empty_state_guide(&mut html, page);
        html.push_str("</div></details></section>");
    } else {
        html.push_str("<section class=\"shell-card\">");
        html.push_str("<details class=\"overview-help\">");
        html.push_str("<summary><p class=\"section-title\">Help</p><h2>Routes and quick checks</h2><p class=\"overview-copy\">Path rules, example requests, and a few reminders for the interactive overview.</p></summary>");
        html.push_str("<div class=\"overview-help-grid\">");
        let _ = write!(
            html,
            "<article class=\"overview-help-card\"><p class=\"section-title\">Routes</p><h3>How dirbase derives paths</h3><p class=\"overview-copy\">{}</p><code class=\"request-path\">{}</code><p class=\"overview-copy\">{}</p></article>",
            escape_html(&page.source_rule),
            escape_html(&page.source_label),
            escape_html(page.resource_name_rule),
        );
        html.push_str("<div class=\"overview-help-card\">");
        html.push_str("<p class=\"section-title\">Quick checks</p><h3>Try these requests</h3>");
        render_request_card(
            &mut html,
            "List data",
            "GET",
            &sample_collection_path,
            "Open the first collection or object route and confirm it returns JSON.",
            None,
        );
        render_request_card(
            &mut html,
            "Fetch one item",
            "GET",
            &sample_item_path,
            "Use an item route when the resource exposes a sample item or declared primary key.",
            None,
        );
        render_request_card(
            &mut html,
            "Open create form",
            "GET",
            &sample_create_form_path,
            "Use the browser form for an array resource when you want to add one item without writing JSON by hand.",
            None,
        );
        render_request_card(
            &mut html,
            "Create one row",
            "POST",
            &sample_create_path,
            "Send one JSON object to confirm writes persist back to disk. Skip this in readonly mode.",
            Some(&sample_create_body),
        );
        html.push_str("</div>");
        html.push_str("</div>");
        html.push_str("<div class=\"overview-note-list\">");
        let _ = write!(
            html,
            "<div class=\"overview-note-item\">{}</div>",
            escape_html(capability_note)
        );
        html.push_str("<div class=\"overview-note-item\">The React overview keeps its state in the page query string. `resource` and `view` are reserved by the shell; table filters, paging, sorting, and embed state reuse the server’s own query params.</div>");
        let _ = write!(
            html,
            "<div class=\"overview-note-item\">Filtering, sorting, pagination, and embedding all resolve to native REST requests such as <span class=\"overview-inline-code\">{}</span>, <span class=\"overview-inline-code\">{}</span>, <span class=\"overview-inline-code\">{}</span>, and <span class=\"overview-inline-code\">{}</span>.</div>",
            escape_html(&sample_filter_path),
            escape_html(&sample_sort_path),
            escape_html(&sample_page_path),
            escape_html(&sample_embed_path),
        );
        html.push_str("<div class=\"overview-note-item\">GraphQL remains available at <span class=\"overview-inline-code\">/graphql</span>, but the overview explorer uses REST because it already supports filtering, sorting, pagination, and embeds.</div>");
        html.push_str("</div></details></section>");
    }

    html.push_str("<script type=\"module\" src=\"/assets/overview.js\"></script>");
    html.push_str("</main></body></html>");
    html
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
