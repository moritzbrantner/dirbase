use std::{
    cmp::Ordering,
    collections::{BTreeSet, HashMap},
    fs,
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use clap::{CommandFactory, Parser};
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::Mutex;

mod schema;

use schema::{ColumnType, Schema, TableSchema, load_schema};

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
    resources: Arc<RwLock<BTreeSet<String>>>,
    io_lock: Arc<Mutex<()>>,
    schema: Arc<Option<Schema>>,
    request_log: Option<Arc<StdMutex<fs::File>>>,
}

#[derive(Debug)]
struct AppError {
    status: StatusCode,
    message: String,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

impl AppError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                error: self.message,
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
        resources: Arc::new(RwLock::new(initial_resources)),
        io_lock: Arc::new(Mutex::new(())),
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

    start_resource_watcher(state.folder.clone(), state.resources.clone());

    let app_state = state.clone();
    let app = if cli.readonly {
        Router::new()
            .route("/", get(list_resources))
            .route("/{resource}", get(get_collection))
            .route("/{resource}/{id}", get(get_item))
            .with_state(app_state)
    } else {
        Router::new()
            .route("/", get(list_resources))
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
    let response = next.run(request).await;
    let status = response.status();

    if let Some(log_file) = &state.request_log {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let line = format!("{timestamp} {method} {path} {}\n", status.as_u16());
        if let Ok(mut file) = log_file.lock() {
            let _ = file.write_all(line.as_bytes());
        }
    }

    response
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

async fn get_collection(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Query(query_params): Query<Vec<(String, String)>>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.io_lock.lock().await;
    let data = load_resource(&state.folder, &resource)?;
    validate_resource_data(&state, &resource, &data)?;
    let parsed = parse_collection_query_params(query_params)?;

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
        filter_collection_data(data, &parsed.filters)?
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
}

#[derive(Debug, Clone)]
struct FilterCondition {
    field_path: String,
    operator: FilterOperator,
    value: String,
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

fn filter_collection_data(data: Value, filters: &[FilterCondition]) -> Result<Value, AppError> {
    let items = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    let filtered = items
        .iter()
        .filter(|item| item_matches_filters(item, filters))
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

fn item_matches_filters(item: &Value, filters: &[FilterCondition]) -> bool {
    filters.iter().all(|condition| {
        let Some(actual) = get_value_at_path(item, &condition.field_path) else {
            return false;
        };

        matches_filter(actual, condition)
    })
}

fn matches_filter(actual: &Value, condition: &FilterCondition) -> bool {
    match condition.operator {
        FilterOperator::Eq => value_to_filter_string(actual) == condition.value,
        FilterOperator::Ne => value_to_filter_string(actual) != condition.value,
        FilterOperator::Lt => {
            compare_with_expected(actual, &condition.value).is_some_and(|cmp| cmp == Ordering::Less)
        }
        FilterOperator::Lte => compare_with_expected(actual, &condition.value)
            .is_some_and(|cmp| cmp == Ordering::Less || cmp == Ordering::Equal),
        FilterOperator::Gt => compare_with_expected(actual, &condition.value)
            .is_some_and(|cmp| cmp == Ordering::Greater),
        FilterOperator::Gte => compare_with_expected(actual, &condition.value)
            .is_some_and(|cmp| cmp == Ordering::Greater || cmp == Ordering::Equal),
        FilterOperator::In => condition
            .value
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .any(|v| value_to_filter_string(actual) == v),
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

fn compare_with_expected(actual: &Value, expected: &str) -> Option<Ordering> {
    if let Some(actual_num) = actual.as_f64() {
        let expected_num = expected.parse::<f64>().ok()?;
        return actual_num.partial_cmp(&expected_num);
    }

    if let Some(actual_bool) = actual.as_bool() {
        let expected_bool = expected.parse::<bool>().ok()?;
        return Some(actual_bool.cmp(&expected_bool));
    }

    Some(value_to_filter_string(actual).as_str().cmp(expected))
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

        let target_resource = load_resource(&state.folder, &fk.target_table)?;
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
    let _guard = state.io_lock.lock().await;

    let mut data = load_resource(&state.folder, &resource)?;
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
    write_resource(&state.folder, &resource, &data)?;

    Ok((StatusCode::CREATED, Json(created)))
}

async fn get_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.io_lock.lock().await;
    let data = load_resource(&state.folder, &resource)?;
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
    let _guard = state.io_lock.lock().await;
    let mut data = load_resource(&state.folder, &resource)?;
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
    write_resource(&state.folder, &resource, &data)?;
    Ok(Json(replacement))
}

async fn patch_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.io_lock.lock().await;
    let mut data = load_resource(&state.folder, &resource)?;
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
    write_resource(&state.folder, &resource, &data)?;
    Ok(Json(updated))
}

async fn delete_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
) -> Result<StatusCode, AppError> {
    let _guard = state.io_lock.lock().await;
    let mut data = load_resource(&state.folder, &resource)?;
    validate_resource_data(&state, &resource, &data)?;
    let array = data
        .as_array_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    let index = find_item_index(array, &id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?;
    array.remove(index);

    validate_resource_data(&state, &resource, &data)?;
    write_resource(&state.folder, &resource, &data)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn replace_resource_object(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.io_lock.lock().await;
    let mut data = load_resource(&state.folder, &resource)?;
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
    write_resource(&state.folder, &resource, &data)?;
    Ok(Json(data))
}

async fn patch_resource_object(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.io_lock.lock().await;
    let mut data = load_resource(&state.folder, &resource)?;
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
    write_resource(&state.folder, &resource, &data)?;
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

fn load_resource(folder: &Path, resource: &str) -> Result<Value, AppError> {
    let file = resource_file_path(folder, resource)?;
    if !file.exists() {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            format!("Resource '{resource}' not found"),
        ));
    }

    let raw = fs::read_to_string(&file)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    serde_json::from_str::<Value>(&raw).map_err(|e| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Invalid JSON: {e}"),
        )
    })
}

fn write_resource(folder: &Path, resource: &str, value: &Value) -> Result<(), AppError> {
    let file = resource_file_path(folder, resource)?;
    let content = serde_json::to_string_pretty(value)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    fs::write(file, format!("{content}\n"))
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
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

fn start_resource_watcher(folder: Arc<PathBuf>, resources: Arc<RwLock<BTreeSet<String>>>) {
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
                Ok(_) => match scan_resources(&folder) {
                    Ok(new_resources) => {
                        if let Ok(mut cache) = resources.write() {
                            *cache = new_resources;
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

        write_resource(temp.path(), "users", &value).expect("write resource");
        let loaded = load_resource(temp.path(), "users").expect("load resource");

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
            resources: Arc::new(RwLock::new(BTreeSet::new())),
            io_lock: Arc::new(Mutex::new(())),
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
