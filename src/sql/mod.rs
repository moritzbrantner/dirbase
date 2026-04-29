use std::collections::{BTreeMap, HashMap};

use axum::{
    Json,
    extract::{Query, State},
    http::{StatusCode, header::CONTENT_TYPE},
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::{Map, Value};
use sqlparser::{
    ast::{
        BinaryOperator, Expr, Join, JoinConstraint, JoinOperator, Select, SelectItem, SetExpr,
        Statement, TableFactor, Value as SqlValue,
    },
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
            (Self::Postgres, ColumnType::BigInteger) => "BIGINT",
            (Self::Sqlite, ColumnType::BigInteger) => "INTEGER",
            (_, ColumnType::Float) => "REAL",
            (Self::Postgres, ColumnType::Decimal) => "NUMERIC",
            (Self::Sqlite, ColumnType::Decimal) => "TEXT",
            (_, ColumnType::Boolean) => "BOOLEAN",
            (Self::Sqlite, ColumnType::Json) => "TEXT",
            (Self::Postgres, ColumnType::Json) => "JSONB",
            (Self::Postgres, ColumnType::Date) => "DATE",
            (Self::Postgres, ColumnType::DateTime) => "TIMESTAMPTZ",
            (Self::Postgres, ColumnType::Uuid) => "UUID",
            (
                _,
                ColumnType::String | ColumnType::Date | ColumnType::DateTime | ColumnType::Uuid,
            ) => "TEXT",
        }
    }
}

#[derive(Debug)]
struct ParsedSqlQuery {
    resource: String,
    resource_alias: String,
    selected_columns: Option<Vec<String>>,
    filters: Vec<FilterCondition>,
    sort_columns: Vec<SortColumn>,
    pagination: Option<Pagination>,
    joins: Vec<ParsedSqlJoin>,
}

#[derive(Debug, Clone)]
struct ParsedSqlJoin {
    resource: String,
    alias: String,
    left_alias: String,
    left_column: String,
    right_column: String,
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
    let resource_names = state.resource_names_sorted().await;
    let _guards = state.read_locks_for_resources(&resource_names).await;
    let sql = build_sql_export(&state, dialect).await?;
    Ok(([(CONTENT_TYPE, "text/sql; charset=utf-8")], sql))
}

async fn run_sql_query(state: AppState, query: String) -> Result<Json<Value>, AppError> {
    let parsed = parse_sql_query(&query, &state).await?;
    let lock_resources = sql_lock_resources(&parsed);
    let _guards = state.read_locks_for_resources(&lock_resources).await;
    let rows = materialize_sql_rows(&state, &parsed).await?;
    let scanned_rows = rows.len();
    if scanned_rows > state.config.max_sql_scan_rows {
        return Err(AppError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "Query exceeds scan guard: {scanned_rows} rows scanned (max {})",
                state.config.max_sql_scan_rows
            ),
        )
        .with_code("unsupported_feature"));
    }

    let filtered = if parsed.filters.is_empty() {
        Value::Array(rows.clone())
    } else {
        filter_collection_data(Value::Array(rows.clone()), &parsed.filters, None)?
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
                AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "Invalid pagination payload")
            })?
    } else {
        sorted
            .as_array()
            .cloned()
            .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?
    };

    let rows = apply_column_selection(paginated_rows, parsed.selected_columns)?;
    let row_count = rows.len();
    if row_count > state.config.max_sql_selected_rows {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            format!(
                "Query returned {row_count} rows; maximum allowed is {}. Use LIMIT to reduce the result set",
                state.config.max_sql_selected_rows
            ),
        )
        .with_code("unsupported_feature"));
    }

    Ok(Json(
        serde_json::json!({ "dialect": "generic", "query": query, "row_count": row_count, "rows": rows }),
    ))
}

fn sql_lock_resources(parsed: &ParsedSqlQuery) -> Vec<String> {
    let mut resources = vec![parsed.resource.clone()];
    for join in &parsed.joins {
        if !resources.contains(&join.resource) {
            resources.push(join.resource.clone());
        }
    }
    resources
}

async fn materialize_sql_rows(
    state: &AppState,
    parsed: &ParsedSqlQuery,
) -> Result<Vec<Value>, AppError> {
    let base = load_resource(state, &parsed.resource).await?;
    let base_value = base.as_ref().clone();
    validate_resource_data(state, &parsed.resource, &base_value)?;
    let base_rows = base_value
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    let mut joined_rows = base_rows
        .iter()
        .map(|row| {
            let object = row.as_object().ok_or_else(|| {
                AppError::new(StatusCode::BAD_REQUEST, "Resource row is not a JSON object")
            })?;
            Ok(build_base_sql_row(&parsed.resource, &parsed.resource_alias, object.clone()))
        })
        .collect::<Result<Vec<_>, AppError>>()?;

    if parsed.joins.is_empty() {
        return Ok(joined_rows);
    }

    let mut join_data = HashMap::<String, Vec<Map<String, Value>>>::new();
    for join in &parsed.joins {
        if join_data.contains_key(&join.alias) {
            continue;
        }
        let resource = load_resource(state, &join.resource).await?;
        let data = resource.as_ref().clone();
        validate_resource_data(state, &join.resource, &data)?;
        let rows = data
            .as_array()
            .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?
            .iter()
            .map(|row| {
                row.as_object().cloned().ok_or_else(|| {
                    AppError::new(StatusCode::BAD_REQUEST, "Resource row is not a JSON object")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        join_data.insert(join.alias.clone(), rows);
    }

    for join in &parsed.joins {
        let Some(target_rows) = join_data.get(&join.alias) else {
            continue;
        };
        let lookup = target_rows.iter().fold(HashMap::new(), |mut acc, row| {
            if let Some(value) = row.get(&join.right_column) {
                acc.entry(value_to_lookup_key(value)).or_insert_with(Vec::new).push(row.clone());
            }
            acc
        });
        let mut next_rows = Vec::new();
        for row in &joined_rows {
            let Some(actual) =
                get_value_at_path(row, &format!("{}.{}", join.left_alias, join.left_column))
            else {
                continue;
            };
            if let Some(matches) = lookup.get(&value_to_lookup_key(actual)) {
                for matched in matches {
                    next_rows.push(extend_joined_row(row, &join.alias, matched.clone()));
                }
            }
        }
        joined_rows = next_rows;
    }

    Ok(joined_rows)
}

fn build_base_sql_row(resource: &str, alias: &str, object: Map<String, Value>) -> Value {
    let mut root = object.clone();
    root.insert(resource.to_string(), Value::Object(object.clone()));
    if alias != resource {
        root.insert(alias.to_string(), Value::Object(object));
    }
    Value::Object(root)
}

fn extend_joined_row(row: &Value, alias: &str, object: Map<String, Value>) -> Value {
    let mut root = row.as_object().cloned().unwrap_or_default();
    root.insert(alias.to_string(), Value::Object(object));
    Value::Object(root)
}

fn value_to_lookup_key(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(boolean) => boolean.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()),
    }
}

async fn parse_sql_query(query: &str, state: &AppState) -> Result<ParsedSqlQuery, AppError> {
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
            let limit =
                query_box.limit.map(|e| parse_sql_usize_literal(&e, "LIMIT")).transpose()?;
            let pagination = match (limit, offset) {
                (None, None) => None,
                (Some(per_page), Some(offset)) => {
                    Some(Pagination { page: (offset / per_page) + 1, per_page })
                }
                (Some(per_page), None) => Some(Pagination { page: 1, per_page }),
                (None, Some(_)) => {
                    return Err(AppError::new(StatusCode::BAD_REQUEST, "OFFSET requires LIMIT")
                        .with_code("invalid_sql"));
                }
            };
            if matches!(pagination.as_ref(), Some(p) if p.per_page > state.config.max_sql_selected_rows)
            {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "LIMIT exceeds max selected rows ({})",
                        state.config.max_sql_selected_rows
                    ),
                )
                .with_code("unsupported_feature"));
            }
            let sort_columns = parse_sql_order_by(query_box.order_by.as_ref())?;
            match *query_box.body {
                SetExpr::Select(select) => {
                    parse_sql_select(*select, sort_columns, pagination, state).await
                }
                _ => {
                    Err(AppError::new(StatusCode::BAD_REQUEST, "Only SELECT queries are supported")
                        .with_code("unsupported_feature"))
                }
            }
        }
        _ => Err(AppError::new(StatusCode::BAD_REQUEST, "Only SELECT statements are supported")
            .with_code("unsupported_feature")),
    }
}

async fn parse_sql_select(
    select: Select,
    sort_columns: Vec<SortColumn>,
    pagination: Option<Pagination>,
    state: &AppState,
) -> Result<ParsedSqlQuery, AppError> {
    if !matches!(select.group_by, sqlparser::ast::GroupByExpr::Expressions(ref exprs, _) if exprs.is_empty())
    {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "GROUP BY is not supported")
            .with_code("unsupported_feature"));
    }
    if select.having.is_some() {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "HAVING is not supported")
            .with_code("unsupported_feature"));
    }
    if select.distinct.is_some() {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "DISTINCT is not supported")
            .with_code("unsupported_feature"));
    }
    if select.from.len() != 1 {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Exactly one table/resource in FROM is required",
        )
        .with_code("invalid_sql"));
    }
    let from = &select.from[0];
    let (resource, resource_alias) = parse_sql_table_factor(&from.relation)?;
    validate_sql_identifier(&resource, "resource")?;
    validate_sql_identifier(&resource_alias, "resource alias")?;
    if !resource_exists(state, &resource).await? {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            format!("Unknown table/resource '{resource}'"),
        )
        .with_code("unknown_table"));
    }
    let joins = parse_sql_joins(&resource, &resource_alias, &from.joins, state).await?;

    let selected_columns = parse_sql_projection(&select.projection)?;
    let filters = if let Some(selection) = select.selection {
        parse_sql_where(&selection)?
    } else {
        Vec::new()
    };
    validate_sql_query_fields(
        state,
        &resource,
        &resource_alias,
        &joins,
        selected_columns.as_deref(),
        &filters,
        &sort_columns,
    )?;
    Ok(ParsedSqlQuery {
        resource,
        resource_alias,
        selected_columns,
        filters,
        sort_columns,
        pagination,
        joins,
    })
}

fn parse_sql_table_factor(relation: &TableFactor) -> Result<(String, String), AppError> {
    match relation {
        TableFactor::Table { name, alias, .. } => {
            let resource = name
                .0
                .last()
                .ok_or_else(|| {
                    AppError::new(StatusCode::BAD_REQUEST, "Missing table/resource name")
                        .with_code("invalid_sql")
                })?
                .value
                .clone();
            let alias = alias
                .as_ref()
                .map(|alias| alias.name.value.clone())
                .unwrap_or_else(|| resource.clone());
            Ok((resource, alias))
        }
        _ => Err(AppError::new(StatusCode::BAD_REQUEST, "Unsupported FROM clause")
            .with_code("unsupported_feature")),
    }
}

async fn parse_sql_joins(
    base_resource: &str,
    base_alias: &str,
    joins: &[Join],
    state: &AppState,
) -> Result<Vec<ParsedSqlJoin>, AppError> {
    let mut parsed = Vec::new();
    let mut aliases = HashMap::from([(base_alias.to_string(), base_resource.to_string())]);
    for join in joins {
        let (resource, alias) = parse_sql_table_factor(&join.relation)?;
        validate_sql_identifier(&resource, "resource")?;
        validate_sql_identifier(&alias, "resource alias")?;
        if !resource_exists(state, &resource).await? {
            return Err(AppError::new(
                StatusCode::NOT_FOUND,
                format!("Unknown table/resource '{resource}'"),
            )
            .with_code("unknown_table"));
        }
        if aliases.contains_key(&alias) {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                format!("Duplicate table alias '{alias}'"),
            )
            .with_code("invalid_sql"));
        }
        let (left_alias, left_column, right_alias, right_column) = match &join.join_operator {
            JoinOperator::Inner(JoinConstraint::On(expr)) => parse_sql_join_on(expr)?,
            JoinOperator::Inner(_) => {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    "INNER JOIN requires an ON clause",
                )
                .with_code("unsupported_feature"));
            }
            _ => {
                return Err(AppError::new(StatusCode::BAD_REQUEST, "Only INNER JOIN is supported")
                    .with_code("unsupported_feature"));
            }
        };
        if right_alias != alias && left_alias != alias {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                "JOIN ON clause must reference the joined table alias",
            )
            .with_code("invalid_sql"));
        }
        let existing_alias = if left_alias == alias { &right_alias } else { &left_alias };
        if !aliases.contains_key(existing_alias) {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                format!("JOIN references unknown alias '{existing_alias}'"),
            )
            .with_code("invalid_sql"));
        }
        let existing_resource = aliases.get(existing_alias).expect("existing alias");
        validate_join_relation(
            state,
            existing_resource,
            &resource,
            if left_alias == alias { &right_column } else { &left_column },
            if left_alias == alias { &left_column } else { &right_column },
        )?;
        parsed.push(ParsedSqlJoin {
            resource: resource.clone(),
            alias: alias.clone(),
            left_alias: if left_alias == alias { right_alias.clone() } else { left_alias.clone() },
            left_column: if left_alias == alias {
                right_column.clone()
            } else {
                left_column.clone()
            },
            right_column: if right_alias == alias {
                right_column.clone()
            } else {
                left_column.clone()
            },
        });
        aliases.insert(alias, resource);
    }
    Ok(parsed)
}

fn parse_sql_join_on(expr: &Expr) -> Result<(String, String, String, String), AppError> {
    let Expr::BinaryOp { left, op, right } = expr else {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "JOIN ON must be a simple equality")
            .with_code("unsupported_feature"));
    };
    if *op != BinaryOperator::Eq {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "JOIN ON only supports equality")
            .with_code("unsupported_feature"));
    }
    let left = parse_sql_qualified_column_expr(left)?;
    let right = parse_sql_qualified_column_expr(right)?;
    Ok((left.0, left.1, right.0, right.1))
}

fn parse_sql_qualified_column_expr(expr: &Expr) -> Result<(String, String), AppError> {
    match expr {
        Expr::CompoundIdentifier(parts) if parts.len() >= 2 => {
            let prefix = parts.first().ok_or_else(|| {
                AppError::new(StatusCode::BAD_REQUEST, "Invalid column reference")
            })?;
            let column = parts.last().ok_or_else(|| {
                AppError::new(StatusCode::BAD_REQUEST, "Invalid column reference")
            })?;
            validate_sql_identifier(&prefix.value, "resource alias")?;
            validate_sql_identifier(&column.value, "column")?;
            Ok((prefix.value.clone(), column.value.clone()))
        }
        _ => Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "JOIN columns must use qualified references like table.column",
        )
        .with_code("invalid_sql")),
    }
}

fn validate_join_relation(
    state: &AppState,
    left_resource: &str,
    right_resource: &str,
    left_column: &str,
    right_column: &str,
) -> Result<(), AppError> {
    let left = state.schema_table(left_resource);
    let right = state.schema_table(right_resource);
    let left_matches = left
        .as_ref()
        .and_then(|table| table.foreign_keys.get(left_column))
        .is_some_and(|fk| fk.target_table == right_resource && fk.target_column == right_column);
    let right_matches = right
        .as_ref()
        .and_then(|table| table.foreign_keys.get(right_column))
        .is_some_and(|fk| fk.target_table == left_resource && fk.target_column == left_column);
    if left_matches || right_matches {
        return Ok(());
    }
    Err(AppError::new(
        StatusCode::BAD_REQUEST,
        format!(
            "JOIN between '{left_resource}.{left_column}' and '{right_resource}.{right_column}' is not backed by schema metadata"
        ),
    )
    .with_code("unsupported_feature"))
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
                for part in parts {
                    validate_sql_identifier(&part.value, "column")?;
                }
                validate_sql_identifier(&column.value, "column")?;
                columns.push(
                    parts.iter().map(|part| part.value.clone()).collect::<Vec<_>>().join("."),
                );
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
            Ok(vec![FilterCondition::new(field_path, operator, value)])
        }
        Expr::IsNull(expr) => Ok(vec![FilterCondition::new(
            parse_sql_column_expr(expr)?,
            FilterOperator::IsNull,
            String::new(),
        )]),
        Expr::IsNotNull(expr) => Ok(vec![FilterCondition::new(
            parse_sql_column_expr(expr)?,
            FilterOperator::IsNotNull,
            String::new(),
        )]),
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
            for part in parts {
                validate_sql_identifier(&part.value, "column")?;
            }
            validate_sql_identifier(&column.value, "column")?;
            Ok(parts.iter().map(|part| part.value.clone()).collect::<Vec<_>>().join("."))
        }
        _ => Err(AppError::new(StatusCode::BAD_REQUEST, "Expected a column identifier")),
    }
}

fn parse_sql_literal(expr: &Expr) -> Result<String, AppError> {
    match expr {
        Expr::Value(value) => parse_sql_value(value),
        Expr::UnaryOp { op, expr } if op.to_string() == "-" => {
            Ok(format!("-{}", parse_sql_literal(expr)?))
        }
        _ => Err(AppError::new(StatusCode::BAD_REQUEST, "Expected a literal value")),
    }
}
fn parse_sql_value(value: &SqlValue) -> Result<String, AppError> {
    match value {
        SqlValue::SingleQuotedString(v) | SqlValue::DoubleQuotedString(v) => Ok(v.clone()),
        SqlValue::Number(v, _) => Ok(v.clone()),
        SqlValue::Boolean(v) => Ok(v.to_string()),
        SqlValue::Null => Ok("null".to_string()),
        _ => Err(AppError::new(StatusCode::BAD_REQUEST, "Unsupported literal value")),
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
        AppError::new(StatusCode::BAD_REQUEST, format!("{clause} must be a non-negative integer"))
    })?;
    if parsed == 0 && clause == "LIMIT" {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "LIMIT must be greater than 0"));
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

async fn build_sql_export(state: &AppState, dialect: SqlExportDialect) -> Result<String, AppError> {
    let resources = state.resources.read().await.iter().cloned().collect::<Vec<_>>();
    let mut chunks = vec![format!(
        "-- SQL export generated by dirbase\n-- dialect: {}\n-- object resources exported as single-row tables with synthetic id=1\n\n",
        match dialect {
            SqlExportDialect::Postgres => "postgres",
            SqlExportDialect::Sqlite => "sqlite",
        }
    )];
    for resource in resources {
        let data = load_resource(state, &resource).await?;
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
                        o.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<BTreeMap<_, _>>()
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
    let schema = state.schema_snapshot();
    if let Some(table) = schema.tables.get(resource) {
        return Ok(table
            .columns
            .iter()
            .map(|(name, schema)| (name.clone(), schema.clone()))
            .collect());
    }
    let mut inferred = BTreeMap::<String, ColumnSchema>::new();
    for row in rows {
        for (key, value) in row {
            let inferred_type = ColumnType::infer_json(value);
            let entry = inferred.entry(key.clone()).or_insert(ColumnSchema {
                column_type: inferred_type.clone().unwrap_or(ColumnType::String),
                nullable: false,
                enum_values: None,
                min: None,
                max: None,
                min_length: None,
                max_length: None,
                pattern: None,
            });
            if let Some(it) = inferred_type
                && entry.column_type != it
            {
                let entry_is_json = matches!(entry.column_type, ColumnType::Json);
                entry.column_type = if matches!(it, ColumnType::Json) || entry_is_json {
                    ColumnType::Json
                } else {
                    ColumnType::String
                };
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
    let col_sql =
        columns.iter().map(|(name, _)| format!("\"{}\"", name)).collect::<Vec<_>>().join(", ");
    let val_sql = columns
        .iter()
        .map(|(name, schema)| serialize_sql_value(row.get(name), &schema.column_type, dialect))
        .collect::<Vec<_>>()
        .join(", ");
    format!("INSERT INTO \"{}\" ({}) VALUES ({});\n", resource, col_sql, val_sql)
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
    resource_alias: &str,
    joins: &[ParsedSqlJoin],
    selected_columns: Option<&[String]>,
    filters: &[FilterCondition],
    sort_columns: &[SortColumn],
) -> Result<(), AppError> {
    if let Some(selected_columns) = selected_columns {
        for column in selected_columns {
            validate_sql_field(
                state,
                resource,
                resource_alias,
                joins,
                column,
                "SELECT projection",
            )?;
        }
    }

    for filter in filters {
        validate_sql_field(
            state,
            resource,
            resource_alias,
            joins,
            &filter.field_path,
            "WHERE clause",
        )?;
    }

    for sort in sort_columns {
        validate_sql_field(
            state,
            resource,
            resource_alias,
            joins,
            &sort.field_path,
            "ORDER BY clause",
        )?;
    }

    Ok(())
}

fn validate_sql_field(
    state: &AppState,
    resource: &str,
    resource_alias: &str,
    joins: &[ParsedSqlJoin],
    field: &str,
    context: &str,
) -> Result<(), AppError> {
    let (target_resource, column_name) =
        resolve_sql_field_target(resource, resource_alias, joins, field)?;
    let Some(table) = state.schema_table(&target_resource) else {
        return Ok(());
    };
    if !table.columns.is_empty() && !table.columns.contains_key(&column_name) {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            format!("Unknown column '{field}' in {context}"),
        ));
    }
    Ok(())
}

fn resolve_sql_field_target(
    resource: &str,
    resource_alias: &str,
    joins: &[ParsedSqlJoin],
    field: &str,
) -> Result<(String, String), AppError> {
    if let Some((prefix, column)) = field.split_once('.') {
        let mut aliases = HashMap::from([(resource_alias.to_string(), resource.to_string())]);
        aliases.insert(resource.to_string(), resource.to_string());
        for join in joins {
            aliases.insert(join.alias.clone(), join.resource.clone());
            aliases.insert(join.resource.clone(), join.resource.clone());
        }
        let Some(target_resource) = aliases.get(prefix) else {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                format!("Unknown table alias '{prefix}' in column reference '{field}'"),
            ));
        };
        return Ok((target_resource.clone(), column.to_string()));
    }
    Ok((resource.to_string(), field.to_string()))
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
        sync::Arc,
    };
    use tokio::sync::RwLock;

    #[test]
    fn parses_select_projection() {
        let state = AppState {
            data_source: Arc::new(crate::app::DataSource::Folder(PathBuf::from("."))),
            config: Arc::new(crate::app::AppConfig {
                readonly: false,
                enable_log: false,
                auth_token: None,
                cors_origin: None,
                max_body_bytes: 1024 * 1024,
                max_per_page: 100,
                max_sql_scan_rows: 50_000,
                max_sql_selected_rows: 1_000,
            }),
            resources: Arc::new(RwLock::new(BTreeSet::from(["users".to_string()]))),
            resource_cache: Arc::new(RwLock::new(HashMap::new())),
            resource_locks: Arc::new(RwLock::new(HashMap::new())),
            schema_store: Arc::new(std::sync::RwLock::new(crate::app::SchemaStore::default())),
            graphql_store: Arc::new(RwLock::new(crate::app::GraphqlStore::default())),
            metrics: Arc::new(crate::app::MetricsStore::default()),
            health: Arc::new(crate::app::HealthState::new(true, None)),
            event_bus: tokio::sync::broadcast::channel(16).0,
        };
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let parsed =
            runtime.block_on(parse_sql_query("SELECT id FROM users", &state)).expect("parse");
        assert_eq!(parsed.resource, "users");
        assert_eq!(parsed.selected_columns.expect("columns"), vec!["id"]);
    }
}
