use axum::http::StatusCode;

use crate::error::AppError;

use super::types::{
    FilterCondition, FilterOperator, Pagination, ParsedCollectionQuery, SortColumn,
};

pub fn parse_collection_query_params(
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
                        sort_columns
                            .push(SortColumn { field_path: field_path.to_string(), descending });
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
        filters.push(FilterCondition::new(field_path, operator, value));
    }

    let pagination = match (page, per_page) {
        (None, None) => None,
        (Some(page), Some(per_page)) => Some(Pagination { page, per_page }),
        (Some(page), None) => Some(Pagination { page, per_page: 10 }),
        (None, Some(per_page)) => Some(Pagination { page: 1, per_page }),
    };

    Ok(ParsedCollectionQuery { filters, sort_columns, pagination, embeds })
}

fn parse_positive_usize(key: &str, value: &str) -> Result<usize, AppError> {
    let parsed = value.parse::<usize>().map_err(|_| {
        AppError::new(StatusCode::BAD_REQUEST, format!("Invalid value for '{key}': '{value}'"))
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
    if let Some((field_path, operator)) = key.split_once(':') {
        let operator = parse_operator(operator).ok_or_else(|| {
            AppError::new(
                StatusCode::BAD_REQUEST,
                format!("Unsupported filter operator '{operator}' in '{key}'"),
            )
        })?;

        if field_path.is_empty() {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                format!("Invalid filter key '{key}'"),
            ));
        }
        return Ok((field_path.to_string(), operator));
    }

    if let Some((field_path, operator)) = key.rsplit_once('_')
        && !field_path.is_empty()
        && let Some(operator) = parse_operator(operator)
    {
        return Ok((field_path.to_string(), operator));
    }

    if key.is_empty() {
        return Err(AppError::new(StatusCode::BAD_REQUEST, format!("Invalid filter key '{key}'")));
    }

    Ok((key.to_string(), FilterOperator::Eq))
}

fn parse_operator(operator: &str) -> Option<FilterOperator> {
    match operator {
        "eq" => Some(FilterOperator::Eq),
        "ne" => Some(FilterOperator::Ne),
        "lt" => Some(FilterOperator::Lt),
        "lte" => Some(FilterOperator::Lte),
        "gt" => Some(FilterOperator::Gt),
        "gte" => Some(FilterOperator::Gte),
        "in" => Some(FilterOperator::In),
        "contains" => Some(FilterOperator::Contains),
        "startsWith" => Some(FilterOperator::StartsWith),
        "endsWith" => Some(FilterOperator::EndsWith),
        "isNull" | "is_null" => Some(FilterOperator::IsNull),
        "isNotNull" | "is_not_null" => Some(FilterOperator::IsNotNull),
        _ => None,
    }
}
