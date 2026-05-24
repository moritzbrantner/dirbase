use std::collections::HashMap;

use axum::http::StatusCode;
use sqlparser::{
    ast::{
        BinaryOperator, Expr, Ident, Join, JoinConstraint, JoinOperator, Select, SelectItem,
        SetExpr, Statement, TableFactor, Value as SqlValue,
    },
    dialect::GenericDialect,
    parser::Parser as SqlParser,
};

use crate::{
    app::AppState,
    error::AppError,
    query::filters::{FilterCondition, FilterOperator, Pagination, SortColumn},
    storage::{resource_exists, validate_sql_identifier},
};

use super::{
    executor::validate_sql_query_fields,
    types::{MAX_SQL_QUERY_LENGTH, ParsedSqlJoin, ParsedSqlProjection, ParsedSqlQuery},
};

pub(crate) async fn parse_sql_query(
    query: &str,
    state: &AppState,
) -> Result<ParsedSqlQuery, AppError> {
    if query.len() > MAX_SQL_QUERY_LENGTH {
        return Err(AppError::bad_request(format!(
            "SQL query length exceeds {MAX_SQL_QUERY_LENGTH} characters"
        ))
        .with_code(crate::error::ERROR_CODE_INVALID_SQL));
    }
    let statements = SqlParser::parse_sql(&GenericDialect {}, query).map_err(|err| {
        AppError::new(StatusCode::BAD_REQUEST, format!("Invalid SQL query: {err}"))
            .with_code(crate::error::ERROR_CODE_INVALID_SQL)
    })?;
    if statements.len() != 1 {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Only a single SQL statement is supported",
        )
        .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE));
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
                        .with_code(crate::error::ERROR_CODE_INVALID_SQL));
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
                .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE));
            }
            let sort_columns = parse_sql_order_by(query_box.order_by.as_ref())?;
            match *query_box.body {
                SetExpr::Select(select) => {
                    parse_sql_select(*select, sort_columns, pagination, state).await
                }
                _ => {
                    Err(AppError::new(StatusCode::BAD_REQUEST, "Only SELECT queries are supported")
                        .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE))
                }
            }
        }
        _ => Err(AppError::new(StatusCode::BAD_REQUEST, "Only SELECT statements are supported")
            .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE)),
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
            .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE));
    }
    if select.having.is_some() {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "HAVING is not supported")
            .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE));
    }
    if select.distinct.is_some() {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "DISTINCT is not supported")
            .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE));
    }
    if select.from.len() != 1 {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Exactly one table/resource in FROM is required",
        )
        .with_code(crate::error::ERROR_CODE_INVALID_SQL));
    }
    let from = &select.from[0];
    let (resource, resource_alias) = parse_sql_table_factor(&from.relation)?;
    validate_sql_query_identifier(&resource, "resource")?;
    validate_sql_query_identifier(&resource_alias, "resource alias")?;
    if !resource_exists(state, &resource).await? {
        return Err(AppError::not_found(format!("Unknown table/resource '{resource}'"))
            .with_code(crate::error::ERROR_CODE_UNKNOWN_TABLE));
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
            for part in &name.0 {
                validate_sql_ast_identifier(part, "resource")?;
            }
            let resource = name
                .0
                .last()
                .ok_or_else(|| {
                    AppError::new(StatusCode::BAD_REQUEST, "Missing table/resource name")
                        .with_code(crate::error::ERROR_CODE_INVALID_SQL)
                })?
                .value
                .clone();
            let alias = if let Some(alias) = alias {
                validate_sql_ast_identifier(&alias.name, "resource alias")?;
                alias.name.value.clone()
            } else {
                resource.clone()
            };
            Ok((resource, alias))
        }
        _ => Err(AppError::new(StatusCode::BAD_REQUEST, "Unsupported FROM clause")
            .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE)),
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
        validate_sql_query_identifier(&resource, "resource")?;
        validate_sql_query_identifier(&alias, "resource alias")?;
        if !resource_exists(state, &resource).await? {
            return Err(AppError::new(
                StatusCode::NOT_FOUND,
                format!("Unknown table/resource '{resource}'"),
            )
            .with_code(crate::error::ERROR_CODE_UNKNOWN_TABLE));
        }
        if aliases.contains_key(&alias) {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                format!("Duplicate table alias '{alias}'"),
            )
            .with_code(crate::error::ERROR_CODE_INVALID_SQL));
        }
        let (left_alias, left_column, right_alias, right_column) = match &join.join_operator {
            JoinOperator::Inner(JoinConstraint::On(expr)) => parse_sql_join_on(expr)?,
            JoinOperator::Inner(_) => {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    "INNER JOIN requires an ON clause",
                )
                .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE));
            }
            _ => {
                return Err(AppError::new(StatusCode::BAD_REQUEST, "Only INNER JOIN is supported")
                    .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE));
            }
        };
        if right_alias != alias && left_alias != alias {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                "JOIN ON clause must reference the joined table alias",
            )
            .with_code(crate::error::ERROR_CODE_INVALID_SQL));
        }
        let existing_alias = if left_alias == alias { &right_alias } else { &left_alias };
        if !aliases.contains_key(existing_alias) {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                format!("JOIN references unknown alias '{existing_alias}'"),
            )
            .with_code(crate::error::ERROR_CODE_INVALID_SQL));
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
            .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE));
    };
    if *op != BinaryOperator::Eq {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "JOIN ON only supports equality")
            .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE));
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
            validate_sql_ast_identifier(prefix, "resource alias")?;
            validate_sql_ast_identifier(column, "column")?;
            Ok((prefix.value.clone(), column.value.clone()))
        }
        _ => Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "JOIN columns must use qualified references like table.column",
        )
        .with_code(crate::error::ERROR_CODE_INVALID_SQL)),
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
    .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE))
}

fn parse_sql_projection(
    projection: &[SelectItem],
) -> Result<Option<Vec<ParsedSqlProjection>>, AppError> {
    if projection.len() == 1 && matches!(projection[0], SelectItem::Wildcard(_)) {
        return Ok(None);
    }
    let mut columns = Vec::new();
    for item in projection {
        match item {
            SelectItem::UnnamedExpr(Expr::Identifier(identifier)) => {
                validate_sql_ast_identifier(identifier, "column")?;
                columns.push(ParsedSqlProjection {
                    source: identifier.value.clone(),
                    output: identifier.value.clone(),
                });
            }
            SelectItem::UnnamedExpr(Expr::CompoundIdentifier(parts)) => {
                let source = parse_sql_compound_column(parts)?;
                columns.push(ParsedSqlProjection { output: source.clone(), source });
            }
            SelectItem::ExprWithAlias { expr, alias } => {
                validate_sql_ast_identifier(alias, "column alias")?;
                columns.push(ParsedSqlProjection {
                    source: parse_sql_column_expr(expr)?,
                    output: alias.value.clone(),
                });
            }
            _ => {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    "Unsupported SELECT projection",
                )
                .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE));
            }
        }
    }
    Ok(Some(columns))
}

fn parse_sql_compound_column(parts: &[Ident]) -> Result<String, AppError> {
    let column = parts
        .last()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Invalid column reference"))?;
    for part in parts {
        validate_sql_ast_identifier(part, "column")?;
    }
    validate_sql_ast_identifier(column, "column")?;
    Ok(parts.iter().map(|part| part.value.clone()).collect::<Vec<_>>().join("."))
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
        Expr::InList { expr, list, negated } => {
            if *negated {
                return Err(AppError::new(StatusCode::BAD_REQUEST, "NOT IN is not supported")
                    .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE));
            }
            if list.is_empty() {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    "IN requires at least one value",
                )
                .with_code(crate::error::ERROR_CODE_INVALID_SQL));
            }
            let field_path = parse_sql_column_expr(expr)?;
            let values =
                list.iter().map(parse_sql_literal).collect::<Result<Vec<_>, _>>()?.join(",");
            Ok(vec![FilterCondition::new(field_path, FilterOperator::In, values)])
        }
        Expr::Between { expr, negated, low, high } => {
            if *negated {
                return Err(AppError::new(StatusCode::BAD_REQUEST, "NOT BETWEEN is not supported")
                    .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE));
            }
            let field_path = parse_sql_column_expr(expr)?;
            Ok(vec![
                FilterCondition::new(
                    field_path.clone(),
                    FilterOperator::Gte,
                    parse_sql_literal(low)?,
                ),
                FilterCondition::new(field_path, FilterOperator::Lte, parse_sql_literal(high)?),
            ])
        }
        Expr::Like { negated, any, expr, pattern, escape_char } => {
            parse_sql_like(expr, pattern, *negated, *any, escape_char.as_deref())
        }
        Expr::ILike { negated, any, expr, pattern, escape_char } => {
            parse_sql_like(expr, pattern, *negated, *any, escape_char.as_deref())
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

fn parse_sql_like(
    expr: &Expr,
    pattern: &Expr,
    negated: bool,
    any: bool,
    escape_char: Option<&str>,
) -> Result<Vec<FilterCondition>, AppError> {
    if negated {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "NOT LIKE is not supported")
            .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE));
    }
    if any {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "LIKE ANY is not supported")
            .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE));
    }
    if escape_char.is_some() {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "LIKE ESCAPE is not supported")
            .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE));
    }

    let field_path = parse_sql_column_expr(expr)?;
    let pattern = parse_sql_literal(pattern)?;
    let (operator, value) = sql_like_pattern_to_filter(&pattern)?;
    Ok(vec![FilterCondition::new(field_path, operator, value)])
}

fn sql_like_pattern_to_filter(pattern: &str) -> Result<(FilterOperator, String), AppError> {
    if pattern.contains('_') {
        return Err(AppError::new(StatusCode::BAD_REQUEST, "LIKE '_' wildcards are not supported")
            .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE));
    }

    let leading = pattern.starts_with('%');
    let trailing = pattern.ends_with('%');
    let inner_start = usize::from(leading);
    let inner_end = pattern.len().saturating_sub(usize::from(trailing));
    let value = pattern[inner_start..inner_end].to_string();

    if value.contains('%') {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "LIKE only supports '%' at the beginning or end of the pattern",
        )
        .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE));
    }

    let operator = match (leading, trailing) {
        (true, true) => FilterOperator::Contains,
        (false, true) => FilterOperator::StartsWith,
        (true, false) => FilterOperator::EndsWith,
        (false, false) => FilterOperator::Eq,
    };
    Ok((operator, value))
}

fn parse_sql_column_expr(expr: &Expr) -> Result<String, AppError> {
    match expr {
        Expr::Identifier(identifier) => {
            validate_sql_ast_identifier(identifier, "column")?;
            Ok(identifier.value.clone())
        }
        Expr::CompoundIdentifier(parts) => parse_sql_compound_column(parts),
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

fn validate_sql_ast_identifier(identifier: &Ident, kind: &str) -> Result<(), AppError> {
    if identifier.quote_style.is_some() {
        return Err(AppError::bad_request(format!("Quoted {kind} identifiers are not supported"))
            .with_code(crate::error::ERROR_CODE_INVALID_SQL));
    }
    validate_sql_query_identifier(&identifier.value, kind)
}

fn validate_sql_query_identifier(identifier: &str, kind: &str) -> Result<(), AppError> {
    validate_sql_identifier(identifier, kind)
        .map_err(|err| err.with_code(crate::error::ERROR_CODE_INVALID_SQL))
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
                response_format: crate::app::ResponseFormat::Json,
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
        assert_eq!(
            parsed.selected_columns.expect("columns"),
            vec![ParsedSqlProjection { source: "id".to_string(), output: "id".to_string() }]
        );
    }
}
