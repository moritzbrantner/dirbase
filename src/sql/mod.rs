use std::collections::BTreeMap;

use axum::{
    Json,
    extract::{Query, State},
    http::{StatusCode, header::CONTENT_TYPE},
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::Value;
use sqlparser::{
    ast::{BinaryOperator, Expr, Select, SelectItem, SetExpr, Statement, Value as SqlValue},
    dialect::GenericDialect,
    parser::Parser as SqlParser,
};

use crate::{
    app::AppState,
    error::AppError,
    query::filters::{
        FilterCondition, FilterOperator, Pagination, SortColumn, filter_collection_data,
        get_value_at_path, paginate_collection_data, sort_collection_data,
    },
    schema::{ColumnSchema, ColumnType},
    storage::{load_resource, resource_exists, validate_resource_data, validate_sql_identifier},
};

const MAX_SQL_QUERY_LENGTH: usize = 16_384;
const MAX_SQL_SELECTED_ROWS: usize = 1_000;
const MAX_SQL_SCANNED_ROWS: usize = 50_000;

#[derive(Deserialize)]
pub struct SqlGetParams {
    pub q: String,
}
#[derive(Deserialize)]
pub struct SqlPostBody {
    pub query: String,
}
#[derive(Deserialize)]
pub struct SqlExportParams {
    pub dialect: Option<String>,
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

pub async fn sql_query(
    State(state): State<AppState>,
    Query(params): Query<SqlGetParams>,
) -> Result<Json<Value>, AppError> {
    run_sql_query(state, params.q).await
}
pub async fn sql_query_post(
    State(state): State<AppState>,
    Json(payload): Json<SqlPostBody>,
) -> Result<Json<Value>, AppError> {
    run_sql_query(state, payload.query).await
}

pub async fn export_sql(
    State(state): State<AppState>,
    Query(params): Query<SqlExportParams>,
) -> Result<impl IntoResponse, AppError> {
    let dialect = SqlExportDialect::parse(params.dialect.as_deref())?;
    let resource_names = state.resource_names_sorted()?;
    let _guards = state.read_locks_for_resources(&resource_names).await;
    let sql = build_sql_export(&state, dialect)?;
    Ok(([(CONTENT_TYPE, "text/sql; charset=utf-8")], sql))
}

async fn run_sql_query(state: AppState, query: String) -> Result<Json<Value>, AppError> {
    let parsed = parse_sql_query(&query, &state)?;
    let _guard = state.read_lock_for_resource(&parsed.resource).await;
    let data = load_resource(&state, &parsed.resource)?;
    let data = data.as_ref().clone();
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
        paginate_collection_data(sorted, pagination)?
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
        return Err(AppError::new(StatusCode::BAD_REQUEST, format!("Query returned {row_count} rows; maximum allowed is {MAX_SQL_SELECTED_ROWS}. Use LIMIT to reduce the result set")).with_code("unsupported_feature"));
    }

    Ok(Json(
        serde_json::json!({ "dialect": "generic", "query": query, "row_count": row_count, "rows": rows }),
    ))
}

fn parse_sql_query(query: &str, state: &AppState) -> Result<ParsedSqlQuery, AppError> {
    if query.len() > MAX_SQL_QUERY_LENGTH {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            format!("SQL query length exceeds {MAX_SQL_QUERY_LENGTH} characters"),
        )
        .with_code("invalid_sql"));
    }
    let statements = SqlParser::parse_sql(&GenericDialect {}, query).map_err(|err| {
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
    match statement {
        Statement::Query(query_box) => {
            let offset = query_box
                .offset
                .map(|o| parse_sql_usize_literal(&o.value, "OFFSET"))
                .transpose()?;
            let limit = query_box
                .limit
                .map(|e| parse_sql_usize_literal(&e, "LIMIT"))
                .transpose()?;
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
            if matches!(pagination.as_ref(), Some(p) if p.per_page > MAX_SQL_SELECTED_ROWS) {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    format!("LIMIT exceeds max selected rows ({MAX_SQL_SELECTED_ROWS})"),
                )
                .with_code("unsupported_feature"));
            }
            let sort_columns = parse_sql_order_by(query_box.order_by.as_ref())?;
            match *query_box.body {
                SetExpr::Select(select) => {
                    parse_sql_select(*select, sort_columns, pagination, state)
                }
                _ => Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    "Only SELECT queries are supported",
                )
                .with_code("unsupported_feature")),
            }
        }
        _ => Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Only SELECT statements are supported",
        )
        .with_code("unsupported_feature")),
    }
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
        sqlparser::ast::TableFactor::Table { name, .. } => name
            .0
            .last()
            .ok_or_else(|| {
                AppError::new(StatusCode::BAD_REQUEST, "Missing table/resource name")
                    .with_code("invalid_sql")
            })?
            .value
            .clone(),
        _ => {
            return Err(
                AppError::new(StatusCode::BAD_REQUEST, "Unsupported FROM clause")
                    .with_code("unsupported_feature"),
            );
        }
    };

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
                columns.push(identifier.value.clone());
            }
            SelectItem::UnnamedExpr(Expr::CompoundIdentifier(parts)) => {
                let column = parts.last().ok_or_else(|| {
                    AppError::new(StatusCode::BAD_REQUEST, "Invalid column reference")
                })?;
                validate_sql_identifier(&column.value, "column")?;
                columns.push(column.value.clone());
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
                BinaryOperator::Eq => FilterOperator::Eq,
                BinaryOperator::NotEq => FilterOperator::Ne,
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
            if matches!(operator, FilterOperator::Eq | FilterOperator::Ne)
                && value.eq_ignore_ascii_case("null")
            {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    if matches!(operator, FilterOperator::Eq) {
                        "Use IS NULL instead of = NULL"
                    } else {
                        "Use IS NOT NULL instead of != NULL"
                    },
                ));
            }
            Ok(vec![FilterCondition {
                field_path,
                operator,
                value,
            }])
        }
        Expr::IsNull(expr) => Ok(vec![FilterCondition {
            field_path: parse_sql_column_expr(expr)?,
            operator: FilterOperator::IsNull,
            value: String::new(),
        }]),
        Expr::IsNotNull(expr) => Ok(vec![FilterCondition {
            field_path: parse_sql_column_expr(expr)?,
            operator: FilterOperator::IsNotNull,
            value: String::new(),
        }]),
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
            Ok(format!("-{}", parse_sql_literal(expr)?))
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
            Ok(SortColumn {
                field_path: parse_sql_column_expr(&expr.expr)?,
                descending: expr.asc == Some(false),
            })
        })
        .collect()
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
                projected.insert(
                    column.clone(),
                    get_value_at_path(&Value::Object(object.clone()), column)
                        .cloned()
                        .unwrap_or(Value::Null),
                );
            }
            Ok(Value::Object(projected))
        })
        .collect()
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
        append_table_export(&mut chunks, state, &resource, data.as_ref(), dialect)?;
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
    chunks.push(format!("-- Resource: {resource}\n"));
    chunks.push(build_create_table_statement(resource, &columns, dialect));
    for row in rows {
        chunks.push(build_insert_statement(resource, &columns, row, dialect));
    }
    chunks.push("\n".to_string());
    Ok(())
}

fn normalize_resource_rows(data: &Value) -> Result<Vec<BTreeMap<String, Value>>, AppError> {
    match data {
        Value::Array(items) => items
            .iter()
            .map(|item| {
                item.as_object()
                    .ok_or_else(|| {
                        AppError::new(
                            StatusCode::BAD_REQUEST,
                            "Array resource row is not an object",
                        )
                    })
                    .map(|o| {
                        o.iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect::<BTreeMap<_, _>>()
                    })
            })
            .collect(),
        Value::Object(object) => {
            let mut row = BTreeMap::new();
            row.insert("id".to_string(), Value::from(1));
            for (k, v) in object {
                row.insert(k.clone(), v.clone());
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
    if let Some(table) = state
        .schema
        .as_ref()
        .as_ref()
        .and_then(|schema| schema.tables.get(resource))
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
            if let Some(it) = inferred_type {
                if entry.column_type != it {
                    entry.column_type = ColumnType::String;
                    if matches!(entry.column_type, ColumnType::String)
                        && (matches!(it, ColumnType::Json)
                            || matches!(entry.column_type, ColumnType::Json))
                    {
                        entry.column_type = ColumnType::Json;
                    }
                }
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
            format!(
                "  \"{}\" {}{}",
                name,
                dialect.type_name(&schema.column_type),
                if schema.nullable { "" } else { " NOT NULL" }
            )
        })
        .collect::<Vec<_>>()
        .join(",\n");
    format!("CREATE TABLE \"{}\" (\n{}\n);\n", resource, defs)
}

fn build_insert_statement(
    resource: &str,
    columns: &[(String, ColumnSchema)],
    row: BTreeMap<String, Value>,
    dialect: SqlExportDialect,
) -> String {
    let col_sql = columns
        .iter()
        .map(|(name, _)| format!("\"{}\"", name))
        .collect::<Vec<_>>()
        .join(", ");
    let val_sql = columns
        .iter()
        .map(|(name, schema)| serialize_sql_value(row.get(name), &schema.column_type, dialect))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "INSERT INTO \"{}\" ({}) VALUES ({});\n",
        resource, col_sql, val_sql
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

fn quote_sql_string(input: &str) -> String {
    format!("'{}'", input.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::{BTreeSet, HashMap},
        path::PathBuf,
        sync::{Arc, RwLock as StdRwLock},
    };
    use tokio::sync::RwLock;

    #[test]
    fn parses_select_projection() {
        let state = AppState {
            folder: Arc::new(PathBuf::from(".")),
            resources: Arc::new(StdRwLock::new(BTreeSet::from(["users".to_string()]))),
            resource_cache: Arc::new(StdRwLock::new(HashMap::new())),
            resource_locks: Arc::new(RwLock::new(HashMap::new())),
            schema: Arc::new(None),
            request_log: None,
        };
        let parsed = parse_sql_query("SELECT id FROM users", &state).expect("parse");
        assert_eq!(parsed.resource, "users");
        assert_eq!(parsed.selected_columns.expect("columns"), vec!["id"]);
    }
}
