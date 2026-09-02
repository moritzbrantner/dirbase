use std::collections::HashMap;

use axum::{Json, http::StatusCode};
use serde_json::{Map, Value};
use sqlparser::{ast::Statement, dialect::GenericDialect, parser::Parser as SqlParser};

use crate::{
    app::AppState,
    error::AppError,
    query::filters::{
        FilterCondition, SortColumn, filter_collection_data, get_value_at_path,
        sort_collection_data,
    },
    storage::{load_resource, validate_resource_data},
};

use super::{
    parser::parse_sql_query,
    types::{ParsedSqlJoin, ParsedSqlProjection, ParsedSqlQuery},
};

pub(crate) async fn run_sql_query(state: AppState, query: String) -> Result<Json<Value>, AppError> {
    let parsed = parse_sql_query(&query, &state).await?;
    let lock_resources = sql_lock_resources(&parsed);
    let _guards = state.read_locks_for_resources(&lock_resources).await;
    let rows = materialize_sql_rows(&state, &parsed).await?;
    let scanned_rows = rows.len();
    if scanned_rows > state.config.max_sql_scan_rows {
        return Err(AppError::payload_too_large(format!(
            "Query exceeds scan guard: {scanned_rows} rows scanned (max {})",
            state.config.max_sql_scan_rows
        ))
        .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE));
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
        let offset = parse_exact_sql_offset(&query)?;
        sorted
            .as_array()
            .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?
            .iter()
            .skip(offset)
            .take(pagination.per_page)
            .cloned()
            .collect()
    } else {
        sorted
            .as_array()
            .cloned()
            .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?
    };

    let rows = apply_column_selection(paginated_rows, parsed.selected_columns)?;
    let row_count = rows.len();
    if row_count > state.config.max_sql_selected_rows {
        return Err(AppError::bad_request(
            format!(
                "Query returned {row_count} rows; maximum allowed is {}. Use LIMIT to reduce the result set",
                state.config.max_sql_selected_rows
            ),
        )
        .with_code(crate::error::ERROR_CODE_UNSUPPORTED_FEATURE));
    }

    Ok(Json(
        serde_json::json!({ "dialect": "generic", "query": query, "row_count": row_count, "rows": rows }),
    ))
}

fn parse_exact_sql_offset(query: &str) -> Result<usize, AppError> {
    // `parse_sql_query` has already validated the statement and its LIMIT/OFFSET
    // literals. Re-reading the AST here preserves the exact SQL row offset
    // instead of rounding it down into page-number pagination.
    let statements = SqlParser::parse_sql(&GenericDialect {}, query).map_err(|err| {
        AppError::new(StatusCode::BAD_REQUEST, format!("Invalid SQL query: {err}"))
            .with_code(crate::error::ERROR_CODE_INVALID_SQL)
    })?;
    let Some(Statement::Query(query)) = statements.into_iter().next() else {
        return Ok(0);
    };
    let Some(offset) = query.offset else {
        return Ok(0);
    };
    offset.value.to_string().parse::<usize>().map_err(|_| {
        AppError::new(StatusCode::BAD_REQUEST, "OFFSET must be a non-negative integer")
            .with_code(crate::error::ERROR_CODE_INVALID_SQL)
    })
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
        let lookup: HashMap<String, Vec<Map<String, Value>>> =
            target_rows.iter().fold(HashMap::new(), |mut acc, row| {
                if let Some(value) = row.get(&join.right_column) {
                    acc.entry(value_to_lookup_key(value)).or_default().push(row.clone());
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

pub(crate) fn apply_column_selection(
    rows: Vec<Value>,
    selected_columns: Option<Vec<ParsedSqlProjection>>,
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
                    column.output.clone(),
                    get_value_at_path(&Value::Object(object.clone()), &column.source)
                        .cloned()
                        .unwrap_or(Value::Null),
                );
            }
            Ok(Value::Object(projected))
        })
        .collect()
}

pub(crate) fn validate_sql_query_fields(
    state: &AppState,
    resource: &str,
    resource_alias: &str,
    joins: &[ParsedSqlJoin],
    selected_columns: Option<&[ParsedSqlProjection]>,
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
                &column.source,
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
