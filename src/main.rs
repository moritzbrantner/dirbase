use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    hash::{Hash, Hasher},
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    http::{Request, StatusCode, header::CONTENT_TYPE},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use clap::{CommandFactory, Parser};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlparser::{
    ast::{BinaryOperator, Expr, Select, SelectItem, SetExpr, Statement, Value as SqlValue},
    dialect::GenericDialect,
    parser::Parser as SqlParser,
};
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

mod schema;

use schema::{ColumnSchema, ColumnType, Schema, TableSchema, is_valid_identifier, load_schema};

const MAX_SQL_QUERY_LENGTH: usize = 16_384;
const MAX_SQL_SELECTED_ROWS: usize = 1_000;
const MAX_SQL_SCANNED_ROWS: usize = 50_000;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Serve all JSON files in a folder as a REST API"
)]
struct Cli {
    /// Folder containing .json files
    #[arg(short, long, default_value = "./data")]
    folder: PathBuf,

    /// Bind address, e.g. 127.0.0.1:3000
    #[arg(short, long, default_value = "127.0.0.1:3000")]
    bind: SocketAddr,

    /// Enable read-only mode (only GET endpoints are exposed)
    #[arg(long)]
    readonly: bool,

    /// Optional DBML schema file. If omitted, {folder}/schema.dbml is used when present.
    #[arg(long)]
    schema: Option<PathBuf>,

    /// Enable request logging to a file.
    #[arg(long)]
    log: bool,

    /// Log file name/path. Defaults to requests.log in current directory.
    #[arg(long, default_value = "requests.log")]
    logname: PathBuf,
}

#[derive(Clone)]
struct AppState {
    folder: Arc<PathBuf>,
    resources: Arc<StdRwLock<BTreeSet<String>>>,
    resource_cache: Arc<StdRwLock<HashMap<String, CachedResource>>>,
    resource_locks: Arc<RwLock<HashMap<String, Arc<RwLock<()>>>>>,
    schema: Arc<Option<Schema>>,
    request_log: Option<Arc<StdMutex<fs::File>>>,
}

#[derive(Clone)]
struct CachedResource {
    value: Value,
    metadata: CachedMetadata,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct CachedMetadata {
    modified: SystemTime,
    len: u64,
}

#[derive(Debug)]
struct AppError {
    status: StatusCode,
    message: String,
    code: Option<&'static str>,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'static str>,
}

impl AppError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            code: None,
        }
    }

    fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                error: self.message,
                code: self.code,
            }),
        )
            .into_response()
    }
}

#[tokio::main]
async fn main() {
    if std::env::args_os().len() == 1 {
        let mut command = Cli::command();
        command.print_help().expect("print CLI help");
        println!();
        return;
    }

    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    if let Err(err) = fs::create_dir_all(&cli.folder) {
        eprintln!(
            "Failed to create data folder {}: {err}",
            cli.folder.display()
        );
        std::process::exit(1);
    }

    let schema = match load_schema(&cli.folder, cli.schema.as_deref()) {
        Ok(schema) => schema,
        Err(err) => {
            eprintln!("Failed to load schema: {err}");
            std::process::exit(1);
        }
    };

    let initial_resources = scan_resources(&cli.folder).unwrap_or_else(|err| {
        eprintln!("Failed to scan data folder {}: {err}", cli.folder.display());
        BTreeSet::new()
    });

    let state = AppState {
        folder: Arc::new(cli.folder),
        resources: Arc::new(StdRwLock::new(initial_resources)),
        resource_cache: Arc::new(StdRwLock::new(HashMap::new())),
        resource_locks: Arc::new(RwLock::new(HashMap::new())),
        schema: Arc::new(schema),
        request_log: if cli.log {
            match fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&cli.logname)
            {
                Ok(file) => Some(Arc::new(StdMutex::new(file))),
                Err(err) => {
                    eprintln!("Failed to open log file {}: {err}", cli.logname.display());
                    std::process::exit(1);
                }
            }
        } else {
            None
        },
    };

    start_resource_watcher(
        state.folder.clone(),
        state.resources.clone(),
        state.resource_cache.clone(),
    );

    let app_state = state.clone();
    let app = if cli.readonly {
        Router::new()
            .route("/", get(list_resources))
            .route("/graphql", get(graphql).post(graphql))
            .route("/sql", get(sql_query).post(sql_query_post))
            .route("/export.sql", get(export_sql))
            .route("/sql/export", get(export_sql))
            .route("/{resource}", get(get_collection))
            .route("/{resource}/{id}", get(get_item))
            .with_state(app_state)
    } else {
        Router::new()
            .route("/", get(list_resources))
            .route("/graphql", get(graphql).post(graphql))
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
                get(get_item)
                    .put(replace_item)
                    .patch(patch_item)
                    .delete(delete_item),
            )
            .with_state(app_state)
    };

    let app = if cli.log {
        app.layer(middleware::from_fn_with_state(
            state.clone(),
            log_requests_middleware,
        ))
    } else {
        app
    };

    tracing::info!(readonly = cli.readonly, "Readonly mode");
    tracing::info!("Listening on http://{}", cli.bind);
    let listener = tokio::net::TcpListener::bind(cli.bind)
        .await
        .expect("binding server listener");
    axum::serve(listener, app).await.expect("running server");
}

async fn log_requests_middleware(
    State(state): State<AppState>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let query_hash = request_query_hash(request.uri().path(), request.uri().query());
    let response = next.run(request).await;
    let status = response.status();

    if let Some(log_file) = &state.request_log {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let suffix = query_hash
            .map(|hash| format!(" query_hash={hash}"))
            .unwrap_or_default();
        let line = format!("{timestamp} {method} {path} {}{suffix}\n", status.as_u16());
        if let Ok(mut file) = log_file.lock() {
            let _ = file.write_all(line.as_bytes());
        }
    }

    response
}

async fn graphql() -> Json<Value> {
    Json(serde_json::json!({
        "name": "graphql",
        "path": "/graphql"
    }))
}

async fn list_resources(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let resources = state
        .resources
        .read()
        .map_err(|_| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Resource cache lock poisoned",
            )
        })?
        .iter()
        .cloned()
        .collect::<Vec<_>>();

    Ok(Json(serde_json::json!({ "resources": resources })))
}

#[derive(Deserialize)]
struct SqlGetParams {
    q: String,
}

#[derive(Deserialize)]
struct SqlPostBody {
    query: String,
}

#[derive(Deserialize)]
struct SqlExportParams {
    dialect: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SqlExportDialect {
    Postgres,
    Sqlite,
}

impl SqlExportDialect {
    fn parse(value: Option<&str>) -> Result<Self, AppError> {
        match value.unwrap_or("postgres").to_ascii_lowercase().as_str() {
            "postgres" | "postgresql" => Ok(Self::Postgres),
            "sqlite" => Ok(Self::Sqlite),
            other => Err(AppError::new(
                StatusCode::BAD_REQUEST,
                format!("Unsupported SQL dialect '{other}'. Expected 'postgres' or 'sqlite'"),
            )),
        }
    }

    fn type_name(self, column_type: &ColumnType) -> &'static str {
        match (self, column_type) {
            (_, ColumnType::Integer) => "INTEGER",
            (_, ColumnType::Float) => "REAL",
            (_, ColumnType::Boolean) => "BOOLEAN",
            (Self::Sqlite, ColumnType::Json) => "TEXT",
            (Self::Postgres, ColumnType::Json) => "JSONB",
            (_, ColumnType::String) => "TEXT",
        }
    }
}

#[derive(Debug)]
struct ParsedSqlQuery {
    resource: String,
    selected_columns: Option<Vec<String>>,
    filters: Vec<FilterCondition>,
    sort_columns: Vec<SortColumn>,
    pagination: Option<Pagination>,
}

async fn sql_query(
    State(state): State<AppState>,
    Query(params): Query<SqlGetParams>,
) -> Result<Json<Value>, AppError> {
    run_sql_query(state, params.q).await
}

async fn sql_query_post(
    State(state): State<AppState>,
    Json(payload): Json<SqlPostBody>,
) -> Result<Json<Value>, AppError> {
    run_sql_query(state, payload.query).await
}

async fn export_sql(
    State(state): State<AppState>,
    Query(params): Query<SqlExportParams>,
) -> Result<impl IntoResponse, AppError> {
    let dialect = SqlExportDialect::parse(params.dialect.as_deref())?;
    let resource_names = state.resource_names_sorted()?;
    let _guards = state.read_locks_for_resources(&resource_names).await;
    let sql = build_sql_export(&state, dialect)?;

    Ok(([(CONTENT_TYPE, "text/sql; charset=utf-8")], sql))
}

fn build_sql_export(state: &AppState, dialect: SqlExportDialect) -> Result<String, AppError> {
    let resources = state
        .resources
        .read()
        .map_err(|_| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Resource cache lock poisoned",
            )
        })?
        .iter()
        .cloned()
        .collect::<Vec<_>>();

    let mut chunks = vec![format!(
        "-- SQL export generated by folder-server\n-- dialect: {}\n-- object resources exported as single-row tables with synthetic id=1\n\n",
        match dialect {
            SqlExportDialect::Postgres => "postgres",
            SqlExportDialect::Sqlite => "sqlite",
        }
    )];

    for resource in resources {
        let data = load_resource(state, &resource)?;
        append_table_export(&mut chunks, state, &resource, &data, dialect)?;
    }

    Ok(chunks.concat())
}

fn append_table_export(
    chunks: &mut Vec<String>,
    state: &AppState,
    resource: &str,
    data: &Value,
    dialect: SqlExportDialect,
) -> Result<(), AppError> {
    let rows = normalize_resource_rows(data)?;
    let columns = resolve_export_columns(state, resource, &rows)?;

    append_chunk(chunks, format!("-- Resource: {resource}\n"));
    append_chunk(
        chunks,
        build_create_table_statement(resource, &columns, dialect),
    );

    for row in rows {
        append_chunk(
            chunks,
            build_insert_statement(resource, &columns, row, dialect),
        );
    }
    append_chunk(chunks, "\n".to_string());
    Ok(())
}

fn normalize_resource_rows(data: &Value) -> Result<Vec<BTreeMap<String, Value>>, AppError> {
    match data {
        Value::Array(items) => items
            .iter()
            .map(|item| {
                let object = item.as_object().ok_or_else(|| {
                    AppError::new(
                        StatusCode::BAD_REQUEST,
                        "Array resource row is not an object",
                    )
                })?;
                Ok(object
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect::<BTreeMap<_, _>>())
            })
            .collect(),
        Value::Object(object) => {
            let mut row = BTreeMap::new();
            row.insert("id".to_string(), Value::from(1));
            for (key, value) in object {
                row.insert(key.clone(), value.clone());
            }
            Ok(vec![row])
        }
        _ => Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Resource must be a JSON array or object for SQL export",
        )),
    }
}

fn resolve_export_columns(
    state: &AppState,
    resource: &str,
    rows: &[BTreeMap<String, Value>],
) -> Result<Vec<(String, ColumnSchema)>, AppError> {
    if let Some(schema) = state.schema.as_ref()
        && let Some(table) = schema.tables.get(resource)
    {
        return Ok(table
            .columns
            .iter()
            .map(|(name, schema)| (name.clone(), schema.clone()))
            .collect());
    }

    let mut inferred = BTreeMap::<String, ColumnSchema>::new();
    for row in rows {
        for (key, value) in row {
            let inferred_type = infer_column_type(value);
            let entry = inferred.entry(key.clone()).or_insert(ColumnSchema {
                column_type: inferred_type.clone().unwrap_or(ColumnType::String),
                nullable: false,
            });
            if let Some(inferred_type) = inferred_type {
                entry.column_type = merge_column_type(&entry.column_type, inferred_type);
            }
            if value.is_null() {
                entry.nullable = true;
            }
        }
    }

    for row in rows {
        for (name, column) in &mut inferred {
            if !row.contains_key(name) {
                column.nullable = true;
            }
        }
    }

    Ok(inferred.into_iter().collect())
}

fn merge_column_type(current: &ColumnType, next: ColumnType) -> ColumnType {
    if *current == next {
        return current.clone();
    }

    match (current, next) {
        (ColumnType::String, _) | (_, ColumnType::String) => ColumnType::String,
        (ColumnType::Json, _) | (_, ColumnType::Json) => ColumnType::Json,
        (ColumnType::Float, ColumnType::Integer) | (ColumnType::Integer, ColumnType::Float) => {
            ColumnType::Float
        }
        (ColumnType::Boolean, ColumnType::Boolean) => ColumnType::Boolean,
        _ => ColumnType::String,
    }
}

fn infer_column_type(value: &Value) -> Option<ColumnType> {
    if value.is_i64() || value.is_u64() {
        return Some(ColumnType::Integer);
    }
    if value.is_number() {
        return Some(ColumnType::Float);
    }
    if value.is_boolean() {
        return Some(ColumnType::Boolean);
    }
    if value.is_array() || value.is_object() {
        return Some(ColumnType::Json);
    }
    if value.is_null() {
        return None;
    }
    Some(ColumnType::String)
}

fn build_create_table_statement(
    resource: &str,
    columns: &[(String, ColumnSchema)],
    dialect: SqlExportDialect,
) -> String {
    let defs = columns
        .iter()
        .map(|(name, schema)| {
            let nullability = if schema.nullable { "" } else { " NOT NULL" };
            format!(
                "  {} {}{}",
                quote_identifier(name),
                dialect.type_name(&schema.column_type),
                nullability
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");

    format!(
        "CREATE TABLE {} (\n{}\n);\n",
        quote_identifier(resource),
        defs
    )
}

fn build_insert_statement(
    resource: &str,
    columns: &[(String, ColumnSchema)],
    row: BTreeMap<String, Value>,
    dialect: SqlExportDialect,
) -> String {
    let column_sql = columns
        .iter()
        .map(|(name, _)| quote_identifier(name))
        .collect::<Vec<_>>()
        .join(", ");

    let values_sql = columns
        .iter()
        .map(|(name, schema)| serialize_sql_value(row.get(name), &schema.column_type, dialect))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "INSERT INTO {} ({}) VALUES ({});\n",
        quote_identifier(resource),
        column_sql,
        values_sql
    )
}

fn serialize_sql_value(
    value: Option<&Value>,
    column_type: &ColumnType,
    dialect: SqlExportDialect,
) -> String {
    let Some(value) = value else {
        return "NULL".to_string();
    };

    match value {
        Value::Null => "NULL".to_string(),
        Value::Bool(v) => {
            if *v {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        Value::Number(v) => v.to_string(),
        Value::String(v) => quote_sql_string(v),
        Value::Array(_) | Value::Object(_) => {
            let json = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
            if matches!(column_type, ColumnType::Json)
                && matches!(dialect, SqlExportDialect::Postgres)
            {
                format!("{}::jsonb", quote_sql_string(&json))
            } else {
                quote_sql_string(&json)
            }
        }
    }
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn quote_sql_string(input: &str) -> String {
    format!("'{}'", input.replace('\'', "''"))
}

fn append_chunk(chunks: &mut Vec<String>, segment: String) {
    const CHUNK_LIMIT: usize = 128 * 1024;
    if chunks.last().map(|last| last.len()).unwrap_or_default() + segment.len() > CHUNK_LIMIT {
        chunks.push(segment);
    } else if let Some(last) = chunks.last_mut() {
        last.push_str(&segment);
    } else {
        chunks.push(segment);
    }
}

async fn run_sql_query(state: AppState, query: String) -> Result<Json<Value>, AppError> {
    let parsed = parse_sql_query(&query, &state)?;

    let _guard = state.read_lock_for_resource(&parsed.resource).await;
    let data = load_resource(&state, &parsed.resource)?;
    validate_resource_data(&state, &parsed.resource, &data)?;

    let table = state.schema_table(&parsed.resource)?;
    let scanned_rows = data
        .as_array()
        .map(|rows| rows.len())
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    if scanned_rows > MAX_SQL_SCANNED_ROWS {
        return Err(AppError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "Query exceeds scan guard: {scanned_rows} rows scanned (max {MAX_SQL_SCANNED_ROWS})"
            ),
        )
        .with_code("unsupported_feature"));
    }

    let filtered = if parsed.filters.is_empty() {
        data
    } else {
        filter_collection_data(data, &parsed.filters, table)?
    };

    let sorted = if parsed.sort_columns.is_empty() {
        filtered
    } else {
        sort_collection_data(filtered, &parsed.sort_columns)?
    };

    let paginated_rows = if let Some(pagination) = parsed.pagination {
        let paginated = paginate_collection_data(sorted, pagination)?;
        paginated
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| {
                AppError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Invalid pagination payload",
                )
            })?
    } else {
        sorted
            .as_array()
            .cloned()
            .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?
    };

    let rows = apply_column_selection(paginated_rows, parsed.selected_columns)?;
    let row_count = rows.len();
    if row_count > MAX_SQL_SELECTED_ROWS {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "Query returned {row_count} rows; maximum allowed is {MAX_SQL_SELECTED_ROWS}. Use LIMIT to reduce the result set"
            ),
        )
        .with_code("unsupported_feature"));
    }

    Ok(Json(serde_json::json!({
        "dialect": "generic",
        "query": query,
        "row_count": row_count,
        "rows": rows,
    })))
}

async fn get_collection(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Query(query_params): Query<Vec<(String, String)>>,
) -> Result<Json<Value>, AppError> {
    let parsed = parse_collection_query_params(query_params)?;
    let lock_resources = state.embed_lock_resources(&resource, &parsed.embeds)?;
    let _guards = state.read_locks_for_resources(&lock_resources).await;
    let data = load_resource(&state, &resource)?;
    validate_resource_data(&state, &resource, &data)?;

    if parsed.filters.is_empty()
        && parsed.sort_columns.is_empty()
        && parsed.pagination.is_none()
        && parsed.embeds.is_empty()
    {
        return Ok(Json(data));
    }

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
        embed_collection_data(&state, &resource, sorted, &parsed.embeds)?
    };

    if let Some(pagination) = parsed.pagination {
        return Ok(Json(paginate_collection_data(embedded, pagination)?));
    }

    Ok(Json(embedded))
}

#[derive(Debug, Clone, Copy)]
enum FilterOperator {
    Eq,
    Ne,
    Lt,
    Lte,
    Gt,
    Gte,
    In,
    Contains,
    StartsWith,
    EndsWith,
    IsNull,
    IsNotNull,
}

#[derive(Debug, Clone)]
struct FilterCondition {
    field_path: String,
    operator: FilterOperator,
    value: String,
}

#[derive(Debug, Clone, PartialEq)]
enum ComparableValue {
    Null,
    Number(f64),
    Bool(bool),
    String(String),
}

#[derive(Debug, Clone)]
struct SortColumn {
    field_path: String,
    descending: bool,
}

#[derive(Debug, Clone, Copy)]
struct Pagination {
    page: usize,
    per_page: usize,
}

#[derive(Debug, Default)]
struct ParsedCollectionQuery {
    filters: Vec<FilterCondition>,
    sort_columns: Vec<SortColumn>,
    pagination: Option<Pagination>,
    embeds: Vec<String>,
}

fn parse_collection_query_params(
    query_params: Vec<(String, String)>,
) -> Result<ParsedCollectionQuery, AppError> {
    let mut filters = Vec::new();
    let mut sort_columns = Vec::new();
    let mut page = None;
    let mut per_page = None;
    let mut embeds = Vec::new();

    for (key, value) in query_params {
        if key == "sort" || key == "_sort" {
            for column in value.split(',') {
                let column = column.trim();
                if !column.is_empty() {
                    let (descending, field_path) = if let Some(stripped) = column.strip_prefix('-')
                    {
                        (true, stripped)
                    } else {
                        (false, column)
                    };

                    if !field_path.is_empty() {
                        sort_columns.push(SortColumn {
                            field_path: field_path.to_string(),
                            descending,
                        });
                    }
                }
            }
            continue;
        }

        if key == "page" || key == "_page" {
            page = Some(parse_positive_usize(&key, &value)?);
            continue;
        }

        if key == "per_page" || key == "_per_page" {
            per_page = Some(parse_positive_usize(&key, &value)?);
            continue;
        }

        if key == "embed" || key == "_embed" {
            for field in value.split(',') {
                let field = field.trim();
                if !field.is_empty() {
                    embeds.push(field.to_string());
                }
            }
            continue;
        }

        let (field_path, operator) = parse_filter_key(&key)?;

        filters.push(FilterCondition {
            field_path,
            operator,
            value,
        });
    }

    let pagination = match (page, per_page) {
        (None, None) => None,
        (Some(page), Some(per_page)) => Some(Pagination { page, per_page }),
        (Some(page), None) => Some(Pagination { page, per_page: 10 }),
        (None, Some(per_page)) => Some(Pagination { page: 1, per_page }),
    };

    Ok(ParsedCollectionQuery {
        filters,
        sort_columns,
        pagination,
        embeds,
    })
}

fn validate_sql_identifier(identifier: &str, kind: &str) -> Result<(), AppError> {
    if !is_valid_identifier(identifier) {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            format!("Invalid {kind} identifier '{identifier}'"),
        ));
    }

    Ok(())
}

fn validate_sql_query_fields(
    state: &AppState,
    resource: &str,
    selected_columns: Option<&[String]>,
    filters: &[FilterCondition],
    sort_columns: &[SortColumn],
) -> Result<(), AppError> {
    let Some(table) = state.schema_table(resource)? else {
        return Ok(());
    };

    if let Some(selected_columns) = selected_columns {
        for column in selected_columns {
            if !table.columns.contains_key(column) {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    format!("Unknown column '{column}' for resource '{resource}'"),
                ));
            }
        }
    }

    for filter in filters {
        if !table.columns.contains_key(&filter.field_path) {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                format!(
                    "Unknown column '{}' in WHERE clause for resource '{}'",
                    filter.field_path, resource
                ),
            ));
        }
    }

    for sort in sort_columns {
        if !table.columns.contains_key(&sort.field_path) {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                format!(
                    "Unknown column '{}' in ORDER BY clause for resource '{}'",
                    sort.field_path, resource
                ),
            ));
        }
    }

    Ok(())
}

fn parse_sql_query(query: &str, state: &AppState) -> Result<ParsedSqlQuery, AppError> {
    if query.len() > MAX_SQL_QUERY_LENGTH {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            format!("SQL query length exceeds {MAX_SQL_QUERY_LENGTH} characters"),
        )
        .with_code("invalid_sql"));
    }

    let dialect = GenericDialect {};
    let statements = SqlParser::parse_sql(&dialect, query).map_err(|err| {
        AppError::new(StatusCode::BAD_REQUEST, format!("Invalid SQL query: {err}"))
            .with_code("invalid_sql")
    })?;

    if statements.len() != 1 {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Only a single SQL statement is supported",
        )
        .with_code("unsupported_feature"));
    }

    let statement = statements.into_iter().next().expect("single statement");
    let select = match statement {
        Statement::Query(query_box) => {
            if !query_box.limit_by.is_empty() {
                return Err(
                    AppError::new(StatusCode::BAD_REQUEST, "LIMIT BY is not supported")
                        .with_code("unsupported_feature"),
                );
            }

            if query_box.with.is_some() {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    "WITH clauses are not supported",
                )
                .with_code("unsupported_feature"));
            }

            let offset = parse_sql_offset(query_box.offset)?;
            let limit = parse_sql_limit(query_box.limit)?;

            let pagination = match (limit, offset) {
                (None, None) => None,
                (Some(per_page), Some(offset)) => Some(Pagination {
                    page: (offset / per_page) + 1,
                    per_page,
                }),
                (Some(per_page), None) => Some(Pagination { page: 1, per_page }),
                (None, Some(_)) => {
                    return Err(
                        AppError::new(StatusCode::BAD_REQUEST, "OFFSET requires LIMIT")
                            .with_code("invalid_sql"),
                    );
                }
            };

            if matches!(
                pagination.as_ref(),
                Some(pagination) if pagination.per_page > MAX_SQL_SELECTED_ROWS
            ) {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    format!("LIMIT exceeds max selected rows ({MAX_SQL_SELECTED_ROWS})"),
                )
                .with_code("unsupported_feature"));
            }

            let sort_columns = parse_sql_order_by(query_box.order_by.as_ref())?;

            match *query_box.body {
                SetExpr::Select(select) => {
                    parse_sql_select(*select, sort_columns, pagination, state)?
                }
                _ => {
                    return Err(AppError::new(
                        StatusCode::BAD_REQUEST,
                        "Only SELECT queries are supported",
                    )
                    .with_code("unsupported_feature"));
                }
            }
        }
        _ => {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                "Only SELECT statements are supported",
            )
            .with_code("unsupported_feature"));
        }
    };

    Ok(select)
}

fn parse_sql_select(
    select: Select,
    sort_columns: Vec<SortColumn>,
    pagination: Option<Pagination>,
    state: &AppState,
) -> Result<ParsedSqlQuery, AppError> {
    if !matches!(select.group_by, sqlparser::ast::GroupByExpr::Expressions(ref exprs, _) if exprs.is_empty())
    {
        return Err(
            AppError::new(StatusCode::BAD_REQUEST, "GROUP BY is not supported")
                .with_code("unsupported_feature"),
        );
    }

    if select.having.is_some() {
        return Err(
            AppError::new(StatusCode::BAD_REQUEST, "HAVING is not supported")
                .with_code("unsupported_feature"),
        );
    }

    if select.distinct.is_some() {
        return Err(
            AppError::new(StatusCode::BAD_REQUEST, "DISTINCT is not supported")
                .with_code("unsupported_feature"),
        );
    }

    if select.from.len() != 1 {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Exactly one table/resource in FROM is required",
        )
        .with_code("invalid_sql"));
    }

    let from = &select.from[0];
    if !from.joins.is_empty() {
        return Err(
            AppError::new(StatusCode::BAD_REQUEST, "JOIN is not supported")
                .with_code("unsupported_feature"),
        );
    }

    let resource = match &from.relation {
        sqlparser::ast::TableFactor::Table { name, .. } => name,
        _ => {
            return Err(
                AppError::new(StatusCode::BAD_REQUEST, "Unsupported FROM clause")
                    .with_code("unsupported_feature"),
            );
        }
    };
    let resource = resource
        .0
        .last()
        .ok_or_else(|| {
            AppError::new(StatusCode::BAD_REQUEST, "Missing table/resource name")
                .with_code("invalid_sql")
        })?
        .value
        .clone();

    validate_sql_identifier(&resource, "resource")?;
    if !resource_exists(state, &resource)? {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            format!("Unknown table/resource '{resource}'"),
        )
        .with_code("unknown_table"));
    }
    let selected_columns = parse_sql_projection(&select.projection)?;
    let filters = if let Some(selection) = select.selection {
        parse_sql_where(&selection)?
    } else {
        Vec::new()
    };

    validate_sql_query_fields(
        state,
        &resource,
        selected_columns.as_deref(),
        &filters,
        &sort_columns,
    )?;

    Ok(ParsedSqlQuery {
        resource,
        selected_columns,
        filters,
        sort_columns,
        pagination,
    })
}

fn parse_sql_projection(projection: &[SelectItem]) -> Result<Option<Vec<String>>, AppError> {
    if projection.len() == 1 && matches!(projection[0], SelectItem::Wildcard(_)) {
        return Ok(None);
    }

    let mut columns = Vec::new();
    for item in projection {
        match item {
            SelectItem::UnnamedExpr(Expr::Identifier(identifier)) => {
                validate_sql_identifier(&identifier.value, "column")?;
                columns.push(identifier.value.clone())
            }
            SelectItem::UnnamedExpr(Expr::CompoundIdentifier(parts)) => {
                let column = parts.last().ok_or_else(|| {
                    AppError::new(StatusCode::BAD_REQUEST, "Invalid column reference")
                })?;
                validate_sql_identifier(&column.value, "column")?;
                columns.push(column.value.clone());
            }
            SelectItem::ExprWithAlias { .. } => {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    "Column aliases are not supported",
                )
                .with_code("unsupported_feature"));
            }
            SelectItem::UnnamedExpr(Expr::Function(_)) => {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    "Aggregate/functions are not supported",
                )
                .with_code("unsupported_feature"));
            }
            _ => {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    "Unsupported SELECT projection",
                )
                .with_code("unsupported_feature"));
            }
        }
    }

    Ok(Some(columns))
}

fn parse_sql_where(expr: &Expr) -> Result<Vec<FilterCondition>, AppError> {
    match expr {
        Expr::BinaryOp { left, op, right } if *op == BinaryOperator::And => {
            let mut left_filters = parse_sql_where(left)?;
            let mut right_filters = parse_sql_where(right)?;
            left_filters.append(&mut right_filters);
            Ok(left_filters)
        }
        Expr::BinaryOp { left, op, right } => {
            let field_path = parse_sql_column_expr(left)?;
            let value = parse_sql_literal(right)?;
            let operator = match op {
                BinaryOperator::Eq => {
                    if value.eq_ignore_ascii_case("null") {
                        return Err(AppError::new(
                            StatusCode::BAD_REQUEST,
                            "Use IS NULL instead of = NULL",
                        ));
                    }
                    FilterOperator::Eq
                }
                BinaryOperator::NotEq => {
                    if value.eq_ignore_ascii_case("null") {
                        return Err(AppError::new(
                            StatusCode::BAD_REQUEST,
                            "Use IS NOT NULL instead of != NULL",
                        ));
                    }
                    FilterOperator::Ne
                }
                BinaryOperator::Lt => FilterOperator::Lt,
                BinaryOperator::LtEq => FilterOperator::Lte,
                BinaryOperator::Gt => FilterOperator::Gt,
                BinaryOperator::GtEq => FilterOperator::Gte,
                _ => {
                    return Err(AppError::new(
                        StatusCode::BAD_REQUEST,
                        format!("Unsupported WHERE operator '{op}'"),
                    ));
                }
            };

            if !matches!(operator, FilterOperator::Eq | FilterOperator::Ne)
                && value.eq_ignore_ascii_case("null")
            {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    "NULL can only be compared with IS NULL / IS NOT NULL",
                ));
            }

            Ok(vec![FilterCondition {
                field_path,
                operator,
                value,
            }])
        }
        Expr::InList {
            expr,
            list,
            negated,
        } => {
            if *negated {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    "NOT IN is not supported",
                ));
            }

            let field_path = parse_sql_column_expr(expr)?;
            let values = list
                .iter()
                .map(parse_sql_literal)
                .collect::<Result<Vec<_>, _>>()?
                .join(",");

            Ok(vec![FilterCondition {
                field_path,
                operator: FilterOperator::In,
                value: values,
            }])
        }
        Expr::Like {
            negated,
            expr,
            pattern,
            ..
        } => {
            if *negated {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    "NOT LIKE is not supported",
                ));
            }

            let field_path = parse_sql_column_expr(expr)?;
            let pattern = parse_sql_literal(pattern)?;
            let (operator, value) = sql_like_to_filter(&pattern)?;

            Ok(vec![FilterCondition {
                field_path,
                operator,
                value,
            }])
        }
        Expr::IsNull(expr) => {
            let field_path = parse_sql_column_expr(expr)?;
            Ok(vec![FilterCondition {
                field_path,
                operator: FilterOperator::IsNull,
                value: String::new(),
            }])
        }
        Expr::IsNotNull(expr) => {
            let field_path = parse_sql_column_expr(expr)?;
            Ok(vec![FilterCondition {
                field_path,
                operator: FilterOperator::IsNotNull,
                value: String::new(),
            }])
        }
        _ => Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Unsupported WHERE clause. Only AND-combined simple predicates are supported",
        )),
    }
}

fn parse_sql_column_expr(expr: &Expr) -> Result<String, AppError> {
    match expr {
        Expr::Identifier(identifier) => {
            validate_sql_identifier(&identifier.value, "column")?;
            Ok(identifier.value.clone())
        }
        Expr::CompoundIdentifier(parts) => {
            let column = parts.last().ok_or_else(|| {
                AppError::new(StatusCode::BAD_REQUEST, "Expected a column identifier")
            })?;
            validate_sql_identifier(&column.value, "column")?;
            Ok(column.value.clone())
        }
        _ => Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Expected a column identifier",
        )),
    }
}

fn parse_sql_literal(expr: &Expr) -> Result<String, AppError> {
    match expr {
        Expr::Value(value) => parse_sql_value(value),
        Expr::UnaryOp { op, expr } if op.to_string() == "-" => {
            let value = parse_sql_literal(expr)?;
            Ok(format!("-{value}"))
        }
        _ => Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Expected a literal value",
        )),
    }
}

fn parse_sql_value(value: &SqlValue) -> Result<String, AppError> {
    match value {
        SqlValue::SingleQuotedString(v) | SqlValue::DoubleQuotedString(v) => Ok(v.clone()),
        SqlValue::Number(v, _) => Ok(v.clone()),
        SqlValue::Boolean(v) => Ok(v.to_string()),
        SqlValue::Null => Ok("null".to_string()),
        _ => Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Unsupported literal value",
        )),
    }
}

fn parse_sql_order_by(
    order_by: Option<&sqlparser::ast::OrderBy>,
) -> Result<Vec<SortColumn>, AppError> {
    let Some(order_by) = order_by else {
        return Ok(Vec::new());
    };

    order_by
        .exprs
        .iter()
        .map(|expr| {
            let field_path = parse_sql_column_expr(&expr.expr)?;
            Ok(SortColumn {
                field_path,
                descending: expr.asc == Some(false),
            })
        })
        .collect()
}

fn parse_sql_limit(expr: Option<Expr>) -> Result<Option<usize>, AppError> {
    expr.map(|value| parse_sql_usize_literal(&value, "LIMIT"))
        .transpose()
}

fn parse_sql_offset(offset: Option<sqlparser::ast::Offset>) -> Result<Option<usize>, AppError> {
    offset
        .map(|offset| parse_sql_usize_literal(&offset.value, "OFFSET"))
        .transpose()
}

fn parse_sql_usize_literal(expr: &Expr, clause: &str) -> Result<usize, AppError> {
    let value = parse_sql_literal(expr)?;
    let parsed = value.parse::<usize>().map_err(|_| {
        AppError::new(
            StatusCode::BAD_REQUEST,
            format!("{clause} must be a non-negative integer"),
        )
    })?;

    if parsed == 0 && clause == "LIMIT" {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "LIMIT must be greater than 0",
        ));
    }

    Ok(parsed)
}

fn sql_like_to_filter(pattern: &str) -> Result<(FilterOperator, String), AppError> {
    let starts = pattern.starts_with('%');
    let ends = pattern.ends_with('%');
    let core = pattern.trim_matches('%').to_string();

    if pattern.matches('%').count() > usize::from(starts) + usize::from(ends) {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "LIKE only supports prefix/suffix wildcards",
        ));
    }

    if starts && ends {
        return Ok((FilterOperator::Contains, core));
    }
    if starts {
        return Ok((FilterOperator::EndsWith, core));
    }
    if ends {
        return Ok((FilterOperator::StartsWith, core));
    }

    Ok((FilterOperator::Eq, core))
}

fn apply_column_selection(
    rows: Vec<Value>,
    selected_columns: Option<Vec<String>>,
) -> Result<Vec<Value>, AppError> {
    let Some(selected_columns) = selected_columns else {
        return Ok(rows);
    };

    rows.into_iter()
        .map(|row| {
            let object = row.as_object().ok_or_else(|| {
                AppError::new(StatusCode::BAD_REQUEST, "Resource row is not a JSON object")
            })?;

            let mut projected = serde_json::Map::new();
            for column in &selected_columns {
                let value = get_value_at_path(&Value::Object(object.clone()), column)
                    .cloned()
                    .unwrap_or(Value::Null);
                projected.insert(column.clone(), value);
            }

            Ok(Value::Object(projected))
        })
        .collect()
}

fn parse_positive_usize(key: &str, value: &str) -> Result<usize, AppError> {
    let parsed = value.parse::<usize>().map_err(|_| {
        AppError::new(
            StatusCode::BAD_REQUEST,
            format!("Invalid value for '{key}': '{value}'"),
        )
    })?;

    if parsed == 0 {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            format!("'{key}' must be greater than 0"),
        ));
    }

    Ok(parsed)
}

fn parse_filter_key(key: &str) -> Result<(String, FilterOperator), AppError> {
    let Some((field_path, operator)) = key.split_once(':') else {
        return Ok((key.to_string(), FilterOperator::Eq));
    };

    let operator = match operator {
        "eq" => FilterOperator::Eq,
        "ne" => FilterOperator::Ne,
        "lt" => FilterOperator::Lt,
        "lte" => FilterOperator::Lte,
        "gt" => FilterOperator::Gt,
        "gte" => FilterOperator::Gte,
        "in" => FilterOperator::In,
        "contains" => FilterOperator::Contains,
        "startsWith" => FilterOperator::StartsWith,
        "endsWith" => FilterOperator::EndsWith,
        _ => {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                format!("Unsupported filter operator '{operator}' in '{key}'"),
            ));
        }
    };

    if field_path.is_empty() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            format!("Invalid filter key '{key}'"),
        ));
    }

    Ok((field_path.to_string(), operator))
}

fn filter_collection_data(
    data: Value,
    filters: &[FilterCondition],
    table: Option<&TableSchema>,
) -> Result<Value, AppError> {
    let items = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    let filtered = items
        .iter()
        .filter(|item| item_matches_filters(item, filters, table))
        .cloned()
        .collect::<Vec<_>>();

    Ok(Value::Array(filtered))
}

fn sort_collection_data(data: Value, sort_columns: &[SortColumn]) -> Result<Value, AppError> {
    let items = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    let mut sorted = items.to_vec();
    sorted.sort_by(|a, b| compare_items_by_columns(a, b, sort_columns));
    Ok(Value::Array(sorted))
}

fn compare_items_by_columns(left: &Value, right: &Value, sort_columns: &[SortColumn]) -> Ordering {
    for column in sort_columns {
        let left_value = get_value_at_path(left, &column.field_path);
        let right_value = get_value_at_path(right, &column.field_path);
        let mut cmp = compare_optional_values(left_value, right_value);
        if column.descending {
            cmp = cmp.reverse();
        }
        if cmp != Ordering::Equal {
            return cmp;
        }
    }

    Ordering::Equal
}

fn compare_optional_values(left: Option<&Value>, right: Option<&Value>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => compare_json_values(left, right),
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => Ordering::Equal,
    }
}

fn compare_json_values(left: &Value, right: &Value) -> Ordering {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => left
            .as_f64()
            .zip(right.as_f64())
            .and_then(|(l, r)| l.partial_cmp(&r))
            .unwrap_or(Ordering::Equal),
        (Value::Bool(left), Value::Bool(right)) => left.cmp(right),
        (Value::String(left), Value::String(right)) => left.cmp(right),
        (Value::Null, Value::Null) => Ordering::Equal,
        _ => value_to_filter_string(left).cmp(&value_to_filter_string(right)),
    }
}

fn item_matches_filters(
    item: &Value,
    filters: &[FilterCondition],
    table: Option<&TableSchema>,
) -> bool {
    filters.iter().all(|condition| {
        let actual = get_value_at_path(item, &condition.field_path).unwrap_or(&Value::Null);
        let column = table.and_then(|t| t.columns.get(&condition.field_path));
        matches_filter(actual, condition, column)
    })
}

fn matches_filter(
    actual: &Value,
    condition: &FilterCondition,
    column: Option<&ColumnSchema>,
) -> bool {
    match condition.operator {
        FilterOperator::IsNull => actual.is_null(),
        FilterOperator::IsNotNull => !actual.is_null(),
        FilterOperator::Eq => compare_with_expected(actual, &condition.value, column)
            .is_some_and(|cmp| cmp == Ordering::Equal),
        FilterOperator::Ne => compare_with_expected(actual, &condition.value, column)
            .is_some_and(|cmp| cmp != Ordering::Equal),
        FilterOperator::Lt => compare_with_expected(actual, &condition.value, column)
            .is_some_and(|cmp| cmp == Ordering::Less),
        FilterOperator::Lte => compare_with_expected(actual, &condition.value, column)
            .is_some_and(|cmp| cmp == Ordering::Less || cmp == Ordering::Equal),
        FilterOperator::Gt => compare_with_expected(actual, &condition.value, column)
            .is_some_and(|cmp| cmp == Ordering::Greater),
        FilterOperator::Gte => compare_with_expected(actual, &condition.value, column)
            .is_some_and(|cmp| cmp == Ordering::Greater || cmp == Ordering::Equal),
        FilterOperator::In => condition
            .value
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .any(|v| {
                compare_with_expected(actual, v, column).is_some_and(|cmp| cmp == Ordering::Equal)
            }),
        FilterOperator::Contains => actual.as_str().is_some_and(|text| {
            text.to_lowercase()
                .contains(&condition.value.to_lowercase())
        }),
        FilterOperator::StartsWith => actual.as_str().is_some_and(|text| {
            text.to_lowercase()
                .starts_with(&condition.value.to_lowercase())
        }),
        FilterOperator::EndsWith => actual.as_str().is_some_and(|text| {
            text.to_lowercase()
                .ends_with(&condition.value.to_lowercase())
        }),
    }
}

fn compare_with_expected(
    actual: &Value,
    expected: &str,
    column: Option<&ColumnSchema>,
) -> Option<Ordering> {
    let left = coerce_actual_value(actual, column)?;
    let right = coerce_expected_value(expected, column)?;
    compare_comparable_values(&left, &right)
}

fn coerce_actual_value(actual: &Value, column: Option<&ColumnSchema>) -> Option<ComparableValue> {
    if actual.is_null() {
        return Some(ComparableValue::Null);
    }

    if let Some(column) = column {
        return coerce_actual_for_column(actual, column);
    }

    if let Some(number) = actual.as_f64() {
        return Some(ComparableValue::Number(number));
    }
    if let Some(boolean) = actual.as_bool() {
        return Some(ComparableValue::Bool(boolean));
    }
    if let Some(text) = actual.as_str() {
        if let Ok(number) = text.parse::<f64>() {
            return Some(ComparableValue::Number(number));
        }
        if let Ok(boolean) = text.parse::<bool>() {
            return Some(ComparableValue::Bool(boolean));
        }
        return Some(ComparableValue::String(text.to_string()));
    }

    Some(ComparableValue::String(value_to_filter_string(actual)))
}

fn coerce_actual_for_column(actual: &Value, column: &ColumnSchema) -> Option<ComparableValue> {
    match column.column_type {
        ColumnType::Integer | ColumnType::Float => {
            if let Some(number) = actual.as_f64() {
                Some(ComparableValue::Number(number))
            } else {
                actual
                    .as_str()
                    .and_then(|text| text.parse::<f64>().ok())
                    .map(ComparableValue::Number)
            }
        }
        ColumnType::Boolean => {
            if let Some(boolean) = actual.as_bool() {
                Some(ComparableValue::Bool(boolean))
            } else {
                actual
                    .as_str()
                    .and_then(|text| text.parse::<bool>().ok())
                    .map(ComparableValue::Bool)
            }
        }
        ColumnType::String => Some(ComparableValue::String(value_to_filter_string(actual))),
        ColumnType::Json => Some(ComparableValue::String(value_to_filter_string(actual))),
    }
}

fn coerce_expected_value(expected: &str, column: Option<&ColumnSchema>) -> Option<ComparableValue> {
    if expected.eq_ignore_ascii_case("null") {
        return Some(ComparableValue::Null);
    }

    if let Some(column) = column {
        return match column.column_type {
            ColumnType::Integer | ColumnType::Float => {
                expected.parse::<f64>().ok().map(ComparableValue::Number)
            }
            ColumnType::Boolean => expected.parse::<bool>().ok().map(ComparableValue::Bool),
            ColumnType::String | ColumnType::Json => {
                Some(ComparableValue::String(expected.to_string()))
            }
        };
    }

    if let Ok(number) = expected.parse::<f64>() {
        return Some(ComparableValue::Number(number));
    }
    if let Ok(boolean) = expected.parse::<bool>() {
        return Some(ComparableValue::Bool(boolean));
    }

    Some(ComparableValue::String(expected.to_string()))
}

fn compare_comparable_values(left: &ComparableValue, right: &ComparableValue) -> Option<Ordering> {
    match (left, right) {
        (ComparableValue::Null, ComparableValue::Null) => Some(Ordering::Equal),
        (ComparableValue::Number(left), ComparableValue::Number(right)) => left.partial_cmp(right),
        (ComparableValue::Bool(left), ComparableValue::Bool(right)) => Some(left.cmp(right)),
        (ComparableValue::String(left), ComparableValue::String(right)) => Some(left.cmp(right)),
        _ => None,
    }
}

fn get_value_at_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;

    for segment in path.split('.') {
        let object = current.as_object()?;
        current = object.get(segment)?;
    }

    Some(current)
}

fn embed_collection_data(
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

    for embed in embeds {
        let fk = table.foreign_keys.get(embed).ok_or_else(|| {
            AppError::new(
                StatusCode::BAD_REQUEST,
                format!("Cannot embed '{embed}' for resource '{resource}'"),
            )
        })?;

        let target_resource = load_resource(state, &fk.target_table)?;
        let target_items = target_resource.as_array().ok_or_else(|| {
            AppError::new(
                StatusCode::BAD_REQUEST,
                format!(
                    "Embedded resource '{}' is not a JSON array",
                    fk.target_table
                ),
            )
        })?;

        let mut lookup = HashMap::new();
        for item in target_items {
            if let Some((_, key)) = item
                .as_object()
                .and_then(|object| object.get(&fk.target_column).map(|key| (object, key)))
            {
                lookup.insert(value_to_filter_string(key), item.clone());
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
            let replacement = lookup.get(&key).cloned().unwrap_or(Value::Null);
            object.insert(embed.clone(), replacement);
        }
    }

    Ok(Value::Array(embedded_items))
}

fn paginate_collection_data(data: Value, pagination: Pagination) -> Result<Value, AppError> {
    let items = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    let total_items = items.len();
    let pages = if total_items == 0 {
        1
    } else {
        total_items.div_ceil(pagination.per_page)
    };
    let page = pagination.page.min(pages.max(1));
    let start = (page - 1) * pagination.per_page;
    let end = (start + pagination.per_page).min(total_items);
    let data = if start < total_items {
        items[start..end].to_vec()
    } else {
        Vec::new()
    };

    Ok(serde_json::json!({
        "first": 1,
        "prev": if page > 1 { Some(page - 1) } else { None::<usize> },
        "next": if page < pages { Some(page + 1) } else { None::<usize> },
        "last": pages,
        "pages": pages,
        "items": total_items,
        "data": data,
    }))
}

fn value_to_filter_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(boolean) => boolean.to_string(),
        Value::Null => "null".to_string(),
        _ => value.to_string(),
    }
}

async fn create_item(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Json(mut payload): Json<Value>,
) -> Result<impl IntoResponse, AppError> {
    let _guard = state.write_lock_for_resource(&resource).await;

    let mut data = load_resource(&state, &resource)?;
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
    write_resource(&state, &resource, &data)?;

    Ok((StatusCode::CREATED, Json(created)))
}

async fn get_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.read_lock_for_resource(&resource).await;
    let data = load_resource(&state, &resource)?;
    validate_resource_data(&state, &resource, &data)?;
    let array = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    let item = find_item(array, &id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?;

    Ok(Json(item.clone()))
}

async fn replace_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
    Json(mut payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.write_lock_for_resource(&resource).await;
    let mut data = load_resource(&state, &resource)?;
    let array = data
        .as_array_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    let object = payload
        .as_object_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Payload must be a JSON object"))?;
    object.insert(
        "id".to_string(),
        coerce_id_value(&id, state.schema_table(&resource)?),
    );

    let replacement = Value::Object(object.clone());
    let position = find_item_index(array, &id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?;
    array[position] = replacement.clone();

    validate_resource_data(&state, &resource, &data)?;
    write_resource(&state, &resource, &data)?;
    Ok(Json(replacement))
}

async fn patch_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.write_lock_for_resource(&resource).await;
    let mut data = load_resource(&state, &resource)?;
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
    write_resource(&state, &resource, &data)?;
    Ok(Json(updated))
}

async fn delete_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
) -> Result<StatusCode, AppError> {
    let _guard = state.write_lock_for_resource(&resource).await;
    let mut data = load_resource(&state, &resource)?;
    validate_resource_data(&state, &resource, &data)?;
    let array = data
        .as_array_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    let index = find_item_index(array, &id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?;
    array.remove(index);

    validate_resource_data(&state, &resource, &data)?;
    write_resource(&state, &resource, &data)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn replace_resource_object(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.write_lock_for_resource(&resource).await;
    let mut data = load_resource(&state, &resource)?;
    if !data.is_object() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Resource is not a JSON object",
        ));
    }

    if !payload.is_object() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Payload must be a JSON object",
        ));
    }

    data = payload;
    write_resource(&state, &resource, &data)?;
    Ok(Json(data))
}

async fn patch_resource_object(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.write_lock_for_resource(&resource).await;
    let mut data = load_resource(&state, &resource)?;
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
    write_resource(&state, &resource, &data)?;
    Ok(Json(updated))
}

impl AppState {
    fn schema_table(&self, resource: &str) -> Result<Option<&TableSchema>, AppError> {
        let Some(schema) = self.schema.as_ref() else {
            return Ok(None);
        };

        schema.tables.get(resource).map(Some).ok_or_else(|| {
            AppError::new(
                StatusCode::BAD_REQUEST,
                format!("Resource '{resource}' is not defined in schema"),
            )
        })
    }

    fn resource_names_sorted(&self) -> Result<Vec<String>, AppError> {
        Ok(self
            .resources
            .read()
            .map_err(|_| {
                AppError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Resource cache lock poisoned",
                )
            })?
            .iter()
            .cloned()
            .collect())
    }

    fn embed_lock_resources(
        &self,
        resource: &str,
        embeds: &[String],
    ) -> Result<Vec<String>, AppError> {
        let mut resources = BTreeSet::new();
        resources.insert(resource.to_string());

        if !embeds.is_empty() {
            let table = self.schema_table(resource)?.ok_or_else(|| {
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

    async fn read_lock_for_resource(&self, resource: &str) -> OwnedRwLockReadGuard<()> {
        self.resource_lock(resource).await.read_owned().await
    }

    async fn write_lock_for_resource(&self, resource: &str) -> OwnedRwLockWriteGuard<()> {
        self.resource_lock(resource).await.write_owned().await
    }

    async fn read_locks_for_resources(
        &self,
        resources: &[String],
    ) -> Vec<OwnedRwLockReadGuard<()>> {
        let mut guards = Vec::with_capacity(resources.len());
        for resource in resources {
            guards.push(self.read_lock_for_resource(resource).await);
        }
        guards
    }

    async fn resource_lock(&self, resource: &str) -> Arc<RwLock<()>> {
        if let Some(lock) = self.resource_locks.read().await.get(resource).cloned() {
            return lock;
        }

        let mut locks = self.resource_locks.write().await;
        locks
            .entry(resource.to_string())
            .or_insert_with(|| Arc::new(RwLock::new(())))
            .clone()
    }
}

fn validate_resource_data(state: &AppState, resource: &str, data: &Value) -> Result<(), AppError> {
    let Some(table) = state.schema_table(resource)? else {
        return Ok(());
    };

    let array = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    for (index, item) in array.iter().enumerate() {
        let object = item.as_object().ok_or_else(|| {
            AppError::new(
                StatusCode::BAD_REQUEST,
                format!("Row {index} in resource '{resource}' is not an object"),
            )
        })?;

        for key in object.keys() {
            if !table.columns.contains_key(key) {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    format!("Row {index} in resource '{resource}' contains unknown column '{key}'"),
                ));
            }
        }

        for (column_name, column) in &table.columns {
            match object.get(column_name) {
                Some(Value::Null) if !column.nullable => {
                    return Err(AppError::new(
                        StatusCode::BAD_REQUEST,
                        format!(
                            "Row {index} in resource '{resource}' has null for non-null column '{column_name}'"
                        ),
                    ));
                }
                Some(value) if !value_matches_type(value, &column.column_type) => {
                    return Err(AppError::new(
                        StatusCode::BAD_REQUEST,
                        format!(
                            "Row {index} in resource '{resource}' has invalid type for '{column_name}'"
                        ),
                    ));
                }
                None if !column.nullable => {
                    return Err(AppError::new(
                        StatusCode::BAD_REQUEST,
                        format!(
                            "Row {index} in resource '{resource}' is missing non-null column '{column_name}'"
                        ),
                    ));
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn value_matches_type(value: &Value, column_type: &ColumnType) -> bool {
    if value.is_null() {
        return true;
    }

    match column_type {
        ColumnType::Integer => value.as_i64().is_some() || value.as_u64().is_some(),
        ColumnType::Float => value.is_number(),
        ColumnType::Boolean => value.is_boolean(),
        ColumnType::String => value.is_string(),
        ColumnType::Json => true,
    }
}

fn maybe_fill_missing_id(
    item: &mut serde_json::Map<String, Value>,
    array: &[Value],
    table: Option<&TableSchema>,
) -> Result<(), AppError> {
    if item.contains_key("id") {
        return Ok(());
    }

    let id_column = table.and_then(|table| table.columns.get("id"));
    if let Some(column) = id_column {
        if matches!(column.column_type, ColumnType::Integer | ColumnType::Float) {
            item.insert("id".to_string(), Value::from(next_numeric_id(array)));
            return Ok(());
        }

        if !column.nullable {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                "Payload is missing required non-numeric id column",
            ));
        }

        return Ok(());
    }

    item.insert("id".to_string(), Value::from(next_numeric_id(array)));
    Ok(())
}

fn load_resource(state: &AppState, resource: &str) -> Result<Value, AppError> {
    let file = resource_file_path(&state.folder, resource)?;
    if !file.exists() {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            format!("Resource '{resource}' not found"),
        ));
    }

    let metadata = fs::metadata(&file)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let current_metadata = CachedMetadata {
        modified: metadata.modified().map_err(|e| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read file metadata: {e}"),
            )
        })?,
        len: metadata.len(),
    };

    if let Ok(cache) = state.resource_cache.read()
        && let Some(cached) = cache.get(resource)
        && cached.metadata == current_metadata
    {
        return Ok(cached.value.clone());
    }

    let raw = fs::read_to_string(&file)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let value = serde_json::from_str::<Value>(&raw).map_err(|e| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Invalid JSON: {e}"),
        )
    })?;

    if let Ok(mut cache) = state.resource_cache.write() {
        cache.insert(
            resource.to_string(),
            CachedResource {
                value: value.clone(),
                metadata: current_metadata,
            },
        );
    }

    Ok(value)
}

fn resource_exists(state: &AppState, resource: &str) -> Result<bool, AppError> {
    state
        .resources
        .read()
        .map(|resources| resources.contains(resource))
        .map_err(|_| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Resource cache lock poisoned",
            )
        })
}

fn request_query_hash(path: &str, query: Option<&str>) -> Option<String> {
    if path != "/sql" && path != "/export.sql" {
        return None;
    }

    let query = query?;
    let sql = query.split('&').find_map(|pair| {
        pair.split_once('=')
            .and_then(|(k, v)| (k == "q").then_some(v))
    })?;

    Some(stable_hash(sql))
}

fn stable_hash(value: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn write_resource(state: &AppState, resource: &str, value: &Value) -> Result<(), AppError> {
    let file = resource_file_path(&state.folder, resource)?;
    let content = serde_json::to_string_pretty(value)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    fs::write(&file, format!("{content}\n"))
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let metadata = fs::metadata(&file)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let modified = metadata.modified().map_err(|e| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read file metadata: {e}"),
        )
    })?;

    if let Ok(mut cache) = state.resource_cache.write() {
        cache.insert(
            resource.to_string(),
            CachedResource {
                value: value.clone(),
                metadata: CachedMetadata {
                    modified,
                    len: metadata.len(),
                },
            },
        );
    }

    Ok(())
}

fn resource_file_path(folder: &Path, resource: &str) -> Result<PathBuf, AppError> {
    if !is_valid_resource_name(resource) {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Resource name must only contain letters, numbers, underscore, and dash",
        ));
    }

    Ok(folder.join(format!("{resource}.json")))
}

fn is_valid_resource_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn find_item<'a>(items: &'a [Value], id: &str) -> Option<&'a Value> {
    items.iter().find(|item| id_matches(item, id))
}

fn find_item_index(items: &[Value], id: &str) -> Option<usize> {
    items.iter().position(|item| id_matches(item, id))
}

fn id_matches(item: &Value, expected: &str) -> bool {
    item.as_object()
        .and_then(|obj| obj.get("id"))
        .is_some_and(|id| match id {
            Value::Number(n) => n.to_string() == expected,
            Value::String(s) => s == expected,
            _ => false,
        })
}

fn next_numeric_id(items: &[Value]) -> i64 {
    items
        .iter()
        .filter_map(|item| item.as_object().and_then(|obj| obj.get("id")))
        .filter_map(|id| id.as_i64())
        .max()
        .map_or(1, |max| max + 1)
}

fn coerce_id_value(id: &str, table: Option<&TableSchema>) -> Value {
    match table.and_then(|table| table.columns.get("id")) {
        Some(column) if matches!(column.column_type, ColumnType::String) => {
            Value::String(id.to_string())
        }
        _ => id
            .parse::<i64>()
            .map_or_else(|_| Value::String(id.to_string()), Value::from),
    }
}

fn scan_resources(folder: &Path) -> Result<BTreeSet<String>, std::io::Error> {
    let mut resources = BTreeSet::new();
    let entries = fs::read_dir(folder)?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };

        if is_valid_resource_name(stem) {
            resources.insert(stem.to_owned());
        }
    }

    Ok(resources)
}

fn start_resource_watcher(
    folder: Arc<PathBuf>,
    resources: Arc<StdRwLock<BTreeSet<String>>>,
    resource_cache: Arc<StdRwLock<HashMap<String, CachedResource>>>,
) {
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();

        let mut watcher = match RecommendedWatcher::new(
            move |result| {
                let _ = tx.send(result);
            },
            Config::default(),
        ) {
            Ok(watcher) => watcher,
            Err(err) => {
                tracing::error!("Failed to create filesystem watcher: {err}");
                return;
            }
        };

        if let Err(err) = watcher.watch(&folder, RecursiveMode::NonRecursive) {
            tracing::error!("Failed to watch folder {}: {err}", folder.display());
            return;
        }

        for event in rx {
            match event {
                Ok(event) => match scan_resources(&folder) {
                    Ok(new_resources) => {
                        if let Ok(mut cache) = resources.write() {
                            *cache = new_resources;
                        }

                        if let Ok(mut cache) = resource_cache.write() {
                            for path in &event.paths {
                                let is_json =
                                    path.extension().and_then(|ext| ext.to_str()) == Some("json");
                                if !is_json {
                                    continue;
                                }

                                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                                    continue;
                                };

                                if !is_valid_resource_name(stem) {
                                    continue;
                                }

                                if path.exists() {
                                    match fs::read_to_string(path)
                                        .ok()
                                        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                                    {
                                        Some(value) => {
                                            if let Ok(metadata) = fs::metadata(path) {
                                                if let Ok(modified) = metadata.modified() {
                                                    cache.insert(
                                                        stem.to_string(),
                                                        CachedResource {
                                                            value,
                                                            metadata: CachedMetadata {
                                                                modified,
                                                                len: metadata.len(),
                                                            },
                                                        },
                                                    );
                                                } else {
                                                    cache.remove(stem);
                                                }
                                            } else {
                                                cache.remove(stem);
                                            }
                                        }
                                        None => {
                                            cache.remove(stem);
                                        }
                                    }
                                } else {
                                    cache.remove(stem);
                                }
                            }
                        }
                    }
                    Err(err) => tracing::error!(
                        "Failed to refresh resources for folder {}: {err}",
                        folder.display()
                    ),
                },
                Err(err) => tracing::warn!("File watch event error: {err}"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::parse_dbml_schema;

    #[test]
    fn validates_resource_names() {
        assert!(is_valid_resource_name("users"));
        assert!(is_valid_resource_name("blog_posts-2025"));
        assert!(!is_valid_resource_name(""));
        assert!(!is_valid_resource_name("../evil"));
        assert!(!is_valid_resource_name("with space"));
    }

    #[test]
    fn finds_next_numeric_id() {
        let items = serde_json::json!([
            {"id": 1, "name": "a"},
            {"id": 5, "name": "b"},
            {"id": "abc", "name": "c"}
        ]);

        assert_eq!(next_numeric_id(items.as_array().expect("array")), 6);
    }

    #[test]
    fn writes_and_reads_resource_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let value = serde_json::json!([{"id": 1, "name": "example"}]);
        let state = AppState {
            folder: Arc::new(temp.path().to_path_buf()),
            resources: Arc::new(StdRwLock::new(BTreeSet::new())),
            resource_cache: Arc::new(StdRwLock::new(HashMap::new())),
            resource_locks: Arc::new(RwLock::new(HashMap::new())),
            schema: Arc::new(None),
            request_log: None,
        };

        write_resource(&state, "users", &value).expect("write resource");
        let loaded = load_resource(&state, "users").expect("load resource");

        assert_eq!(value, loaded);
    }

    #[test]
    fn scans_only_valid_json_resource_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(temp.path().join("users.json"), "[]").expect("write users");
        fs::write(temp.path().join("posts.json"), "[]").expect("write posts");
        fs::write(temp.path().join("notes.txt"), "hello").expect("write txt");
        fs::write(temp.path().join("bad name.json"), "[]").expect("write invalid");

        let resources = scan_resources(temp.path()).expect("scan resources");

        assert_eq!(
            resources.into_iter().collect::<Vec<_>>(),
            vec!["posts".to_string(), "users".to_string()]
        );
    }

    #[test]
    fn validates_rows_against_schema() {
        let schema = parse_dbml_schema(
            r#"
            Table users {
              id int [pk]
              name varchar [not null]
              active bool
            }
            "#,
        )
        .expect("parse schema");

        let state = AppState {
            folder: Arc::new(PathBuf::from(".")),
            resources: Arc::new(StdRwLock::new(BTreeSet::new())),
            resource_cache: Arc::new(StdRwLock::new(HashMap::new())),
            resource_locks: Arc::new(RwLock::new(HashMap::new())),
            schema: Arc::new(Some(schema)),
            request_log: None,
        };

        let ok = serde_json::json!([{"id": 1, "name": "Ada", "active": true}]);
        assert!(validate_resource_data(&state, "users", &ok).is_ok());

        let wrong_type = serde_json::json!([{"id": "oops", "name": "Ada"}]);
        assert!(validate_resource_data(&state, "users", &wrong_type).is_err());

        let unknown_col = serde_json::json!([{"id": 1, "name": "Ada", "role": "admin"}]);
        assert!(validate_resource_data(&state, "users", &unknown_col).is_err());
    }

    #[test]
    fn filters_collection_items_with_multiple_query_params() {
        let data = serde_json::json!([
            {"id": 1, "role": "admin", "active": true},
            {"id": 2, "role": "admin", "active": false},
            {"id": 3, "role": "member", "active": true}
        ]);

        let filtered = filter_collection_data(
            data,
            &[
                FilterCondition {
                    field_path: "role".to_string(),
                    operator: FilterOperator::Eq,
                    value: "admin".to_string(),
                },
                FilterCondition {
                    field_path: "active".to_string(),
                    operator: FilterOperator::Eq,
                    value: "true".to_string(),
                },
            ],
            None,
        )
        .expect("filter collection");

        assert_eq!(
            filtered,
            serde_json::json!([{"id": 1, "role": "admin", "active": true}])
        );
    }

    #[test]
    fn sorts_collection_items_by_one_or_more_columns() {
        let data = serde_json::json!([
            {"id": 2, "role": "admin", "name": "Zed"},
            {"id": 1, "role": "member", "name": "Ada"},
            {"id": 3, "role": "admin", "name": "Bob"}
        ]);

        let sorted = sort_collection_data(
            data.clone(),
            &[SortColumn {
                field_path: "id".to_string(),
                descending: false,
            }],
        )
        .expect("sort by id");
        assert_eq!(
            sorted,
            serde_json::json!([
                {"id": 1, "role": "member", "name": "Ada"},
                {"id": 2, "role": "admin", "name": "Zed"},
                {"id": 3, "role": "admin", "name": "Bob"}
            ])
        );

        let sorted_multi = sort_collection_data(
            data,
            &[
                SortColumn {
                    field_path: "role".to_string(),
                    descending: false,
                },
                SortColumn {
                    field_path: "name".to_string(),
                    descending: false,
                },
            ],
        )
        .expect("sort by role and name");
        assert_eq!(
            sorted_multi,
            serde_json::json!([
                {"id": 3, "role": "admin", "name": "Bob"},
                {"id": 2, "role": "admin", "name": "Zed"},
                {"id": 1, "role": "member", "name": "Ada"}
            ])
        );
    }

    #[test]
    fn supports_advanced_filter_operators_and_pagination() {
        let data = serde_json::json!([
            {"id": 1, "title": "Hello World", "views": 150, "author": {"name": "Typicode"}},
            {"id": 2, "title": "Other post", "views": 80, "author": {"name": "Alice"}},
            {"id": 3, "title": "hello rust", "views": 200, "author": {"name": "Typicode"}}
        ]);

        let filtered = filter_collection_data(
            data.clone(),
            &[
                FilterCondition {
                    field_path: "views".to_string(),
                    operator: FilterOperator::Gt,
                    value: "100".to_string(),
                },
                FilterCondition {
                    field_path: "title".to_string(),
                    operator: FilterOperator::Contains,
                    value: "hello".to_string(),
                },
                FilterCondition {
                    field_path: "author.name".to_string(),
                    operator: FilterOperator::Eq,
                    value: "Typicode".to_string(),
                },
            ],
            None,
        )
        .expect("filter collection");

        assert_eq!(
            filtered,
            serde_json::json!([
                {"id": 1, "title": "Hello World", "views": 150, "author": {"name": "Typicode"}},
                {"id": 3, "title": "hello rust", "views": 200, "author": {"name": "Typicode"}}
            ])
        );

        let paged = paginate_collection_data(
            data,
            Pagination {
                page: 2,
                per_page: 2,
            },
        )
        .expect("paginate collection");
        assert_eq!(paged["items"], 3);
        assert_eq!(paged["pages"], 2);
        assert_eq!(paged["prev"], 1);
        assert_eq!(paged["next"], serde_json::Value::Null);
        assert_eq!(paged["data"].as_array().expect("array").len(), 1);
    }

    #[test]
    fn splits_query_params_into_filters_and_sort_columns() {
        let parsed = parse_collection_query_params(vec![
            ("role".to_string(), "admin".to_string()),
            ("_sort".to_string(), "-role,name".to_string()),
            ("active".to_string(), "true".to_string()),
            ("sort".to_string(), "id".to_string()),
            ("_page".to_string(), "2".to_string()),
            ("_per_page".to_string(), "25".to_string()),
            ("embed".to_string(), "author_id,team_id".to_string()),
        ])
        .expect("parse query params");

        assert_eq!(parsed.filters.len(), 2);
        assert_eq!(parsed.filters[0].field_path, "role");
        assert!(matches!(parsed.filters[0].operator, FilterOperator::Eq));
        assert_eq!(parsed.sort_columns.len(), 3);
        assert_eq!(parsed.sort_columns[0].field_path, "role");
        assert!(parsed.sort_columns[0].descending);
        assert_eq!(parsed.pagination.unwrap().page, 2);
        assert_eq!(parsed.embeds, vec!["author_id", "team_id"]);
    }

    #[test]
    fn rejects_invalid_query_filter_and_pagination_values() {
        let invalid_operator =
            parse_collection_query_params(vec![("role:unknown".to_string(), "admin".to_string())])
                .expect_err("invalid operator should fail");
        assert_eq!(invalid_operator.status, StatusCode::BAD_REQUEST);

        let invalid_page =
            parse_collection_query_params(vec![("page".to_string(), "0".to_string())])
                .expect_err("page 0 should fail");
        assert_eq!(invalid_page.status, StatusCode::BAD_REQUEST);

        let invalid_per_page =
            parse_collection_query_params(vec![("per_page".to_string(), "abc".to_string())])
                .expect_err("non numeric per_page should fail");
        assert_eq!(invalid_per_page.status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn exports_sql_from_schema_and_object_resource() {
        let schema = parse_dbml_schema(
            r#"
            Table users {
              id int [pk]
              name varchar [not null]
              active bool
            }
            "#,
        )
        .expect("parse schema");

        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(
            temp.path().join("users.json"),
            r#"[
                {"id": 1, "name": "Ada", "active": true},
                {"id": 2, "name": "Turing", "active": false}
            ]"#,
        )
        .expect("write users");
        fs::write(
            temp.path().join("settings.json"),
            r#"{"theme": "dark", "enabled": true}"#,
        )
        .expect("write settings");

        let mut resources = BTreeSet::new();
        resources.insert("settings".to_string());
        resources.insert("users".to_string());

        let state = AppState {
            folder: Arc::new(temp.path().to_path_buf()),
            resources: Arc::new(StdRwLock::new(resources)),
            resource_cache: Arc::new(StdRwLock::new(HashMap::new())),
            resource_locks: Arc::new(RwLock::new(HashMap::new())),
            schema: Arc::new(Some(schema)),
            request_log: None,
        };

        let sql = build_sql_export(&state, SqlExportDialect::Postgres).expect("build sql");

        assert!(sql.contains(r#"CREATE TABLE "users""#));
        assert!(sql.contains(r#""name" TEXT NOT NULL"#));
        assert!(sql.contains(r#"INSERT INTO "users""#));
        assert!(sql.contains(r#"CREATE TABLE "settings""#));
        assert!(sql.contains(r#""id" INTEGER NOT NULL"#));
        assert!(sql.contains(r#"INSERT INTO "settings""#));
        assert!(sql.contains("'dark'"));
        assert!(sql.contains("TRUE"));
    }

    #[test]
    fn infers_columns_and_escapes_values_for_sql_export() {
        let rows = vec![
            BTreeMap::from([
                ("id".to_string(), Value::from(1)),
                ("name".to_string(), Value::from("O'Hara")),
                ("meta".to_string(), serde_json::json!({"a": 1})),
            ]),
            BTreeMap::from([
                ("id".to_string(), Value::from(2.5)),
                ("name".to_string(), Value::Null),
                ("meta".to_string(), Value::Null),
            ]),
        ];

        let state = AppState {
            folder: Arc::new(PathBuf::from(".")),
            resources: Arc::new(StdRwLock::new(BTreeSet::new())),
            resource_cache: Arc::new(StdRwLock::new(HashMap::new())),
            resource_locks: Arc::new(RwLock::new(HashMap::new())),
            schema: Arc::new(None),
            request_log: None,
        };

        let columns = resolve_export_columns(&state, "events", &rows).expect("infer columns");
        let create_sql =
            build_create_table_statement("events", &columns, SqlExportDialect::Postgres);
        let insert_sql = build_insert_statement(
            "events",
            &columns,
            rows.first().cloned().expect("row"),
            SqlExportDialect::Postgres,
        );

        assert!(create_sql.contains(r#""id" REAL"#));
        assert!(create_sql.contains(r#""name" TEXT"#));
        assert!(create_sql.contains(r#""meta" JSONB"#));
        assert!(insert_sql.contains("'O''Hara'"));
        assert!(insert_sql.contains("::jsonb"));
    }

    #[test]
    fn paginates_past_last_page_by_clamping_to_last_page() {
        let data = serde_json::json!([
            {"id": 1, "name": "a"},
            {"id": 2, "name": "b"},
            {"id": 3, "name": "c"}
        ]);

        let paged = paginate_collection_data(
            data,
            Pagination {
                page: 5,
                per_page: 2,
            },
        )
        .expect("paginate collection");

        assert_eq!(paged["last"], 2);
        assert_eq!(paged["prev"], 1);
        assert_eq!(paged["next"], serde_json::Value::Null);

        let ids = paged["data"]
            .as_array()
            .expect("array response")
            .iter()
            .map(|item| item["id"].as_i64().expect("numeric id"))
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![3]);
    }
}
