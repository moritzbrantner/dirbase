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
use serde_json::Value;

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
    pub source_table: String,
    pub source_column: String,
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
    let mut edges = Vec::new();

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
                source_table: source_table.clone(),
                source_column: source_column.clone(),
                target_table: fk.target_table.clone(),
                target_column: fk.target_column.clone(),
            });
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
    match column_type {
        ColumnType::Integer => "integer",
        ColumnType::Float => "float",
        ColumnType::Boolean => "boolean",
        ColumnType::String => "string",
        ColumnType::Json => "json",
    }
}

fn render_overview_html(page: &OverviewPageData) -> String {
    let sample_resource = page.resources.first();
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

    let mut html = String::new();
    html.push_str(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
    );
    html.push_str("<title>dirbase overview</title>");
    html.push_str("<link rel=\"stylesheet\" href=\"/assets/overview.css\">");
    html.push_str("</head><body><main class=\"overview-page\">");
    html.push_str("<section class=\"overview-hero shell-card\">");
    html.push_str("<p class=\"overview-eyebrow\">dirbase</p>");
    html.push_str("<h1>Visual overview of your data</h1>");
    html.push_str("<p class=\"overview-lede\">Use this page as both a route guide and a data explorer. The interactive UI below speaks the same REST query language as the server, so filtering, sorting, paging, and relation drill-down always resolve to copyable request URLs.</p>");
    let _ = write!(
        html,
        "<div class=\"overview-stats\"><span class=\"overview-stat\">{} resources</span><span class=\"overview-stat\">{} table links</span><span class=\"overview-stat\">{} total rows</span><span class=\"overview-stat\">Schema {}</span><span class=\"overview-stat\">Source mode: {}</span></div>",
        page.stats.resource_count,
        page.stats.relation_count,
        page.stats.total_rows,
        if page.schema_enabled { "loaded" } else { "not loaded" },
        escape_html(page.data_source_kind),
    );
    html.push_str("</section>");

    html.push_str("<section class=\"overview-guide-grid\">");
    html.push_str("<article class=\"overview-guide shell-card\">");
    html.push_str("<p class=\"section-title\">Rules of paths</p>");
    html.push_str("<h2>How dirbase derives routes</h2>");
    let _ = write!(
        html,
        "<p class=\"overview-copy\">{}</p><code class=\"overview-inline-code overview-source-line\">{}</code><p class=\"overview-copy\">{}</p>",
        escape_html(&page.source_rule),
        escape_html(&page.source_label),
        escape_html(page.resource_name_rule),
    );
    html.push_str("<div class=\"overview-rule-grid\">");
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
        "Works for array resources whose rows are objects with an `id` field or declared primary key.",
    );
    render_rule_card(
        &mut html,
        "Overview metadata",
        "GET",
        "/overview.json",
        "Returns the schema-aware overview metadata consumed by the interactive explorer.",
    );
    html.push_str("</div></article>");

    html.push_str("<article class=\"overview-guide shell-card\">");
    html.push_str("<p class=\"section-title\">Query options</p>");
    html.push_str("<h2>REST controls that power the UI</h2>");
    html.push_str("<div class=\"overview-rule-grid\">");
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
        "Use `page` and `per_page` to receive metadata plus the current page of rows.",
    );
    render_rule_card(
        &mut html,
        "Embedding",
        "GET",
        &sample_embed_path,
        "Use `embed` when schema metadata defines foreign keys for that resource.",
    );
    html.push_str("</div>");
    html.push_str("<div class=\"overview-note-list\">");
    html.push_str("<div class=\"overview-note-item\">The React overview keeps its state in the page query string. `resource` and `view` are reserved by the shell; table filters, paging, sorting, and embed state reuse the server’s own query params.</div>");
    html.push_str("<div class=\"overview-note-item\">This first version is read-focused: use it to inspect tables, follow foreign keys, preview raw rows, and copy exact request URLs for REST clients.</div>");
    html.push_str("<div class=\"overview-note-item\">GraphQL remains available at <span class=\"overview-inline-code\">/graphql</span>, but the overview explorer uses REST because it already supports filtering, sorting, pagination, and embeds.</div>");
    html.push_str("</div></article></section>");

    html.push_str(
        "<section class=\"shell-card\"><div id=\"overview-root\" data-overview-endpoint=\"/overview.json\"></div><noscript>",
    );
    html.push_str("<div class=\"noscript-shell\"><p class=\"section-title\">Overview fallback</p><h2>Resources</h2><p class=\"overview-copy\">JavaScript is disabled, so the interactive explorer is unavailable. The data model and route guide remain visible below.</p>");
    if page.resources.is_empty() {
        html.push_str("<p class=\"overview-empty\">No resources found yet. Add JSON files to the configured source and reload the page.</p>");
    } else {
        html.push_str("<div class=\"noscript-resource-grid\">");
        for resource in &page.resources {
            let _ = write!(
                html,
                "<article class=\"noscript-resource-card\" data-resource=\"{}\"><div class=\"noscript-resource-head\"><h3>{}</h3><span class=\"overview-kind-badge\">{}</span></div>",
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

            if !resource.columns.is_empty() {
                html.push_str("<div class=\"overview-chip-row\">");
                for column in resource.columns.iter().take(8) {
                    let _ = write!(
                        html,
                        "<span class=\"overview-chip{}\">{} · {}</span>",
                        if column.relation.is_some() { " relation" } else { "" },
                        escape_html(&column.name),
                        escape_html(column.column_type),
                    );
                }
                html.push_str("</div>");
            } else if !resource.field_names.is_empty() {
                html.push_str("<div class=\"overview-chip-row\">");
                for field in resource.field_names.iter().take(8) {
                    let _ =
                        write!(html, "<span class=\"overview-chip\">{}</span>", escape_html(field));
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
            html.push_str("</article>");
        }
        html.push_str("</div>");
    }
    html.push_str("</div></noscript></section>");

    html.push_str("<script type=\"module\" src=\"/assets/overview.js\"></script>");
    html.push_str("</main></body></html>");
    html
}

fn render_rule_card(html: &mut String, title: &str, method: &str, path: &str, copy: &str) {
    let _ = write!(
        html,
        "<article class=\"overview-rule-card\"><p class=\"overview-rule-title\">{}</p><code class=\"overview-path-code\"><span class=\"overview-method\">{}</span> {}</code><p class=\"overview-copy\">{}</p></article>",
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
