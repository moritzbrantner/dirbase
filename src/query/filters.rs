use std::cmp::Ordering;

use axum::http::StatusCode;
use serde_json::Value;

use crate::{
    error::AppError,
    schema::{ColumnSchema, ColumnType, TableSchema},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOperator {
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
pub struct FilterCondition {
    pub field_path: String,
    pub operator: FilterOperator,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct SortColumn {
    pub field_path: String,
    pub descending: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Pagination {
    pub page: usize,
    pub per_page: usize,
}

#[derive(Debug, Default)]
pub struct ParsedCollectionQuery {
    pub filters: Vec<FilterCondition>,
    pub sort_columns: Vec<SortColumn>,
    pub pagination: Option<Pagination>,
    pub embeds: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum ComparableValue {
    Null,
    Number(f64),
    Bool(bool),
    String(String),
}

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
        filters.push(FilterCondition { field_path, operator, value });
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
        _ => None,
    }
}

pub fn filter_collection_data(
    data: Value,
    filters: &[FilterCondition],
    table: Option<&TableSchema>,
) -> Result<Value, AppError> {
    let items = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    Ok(Value::Array(
        items.iter().filter(|item| item_matches_filters(item, filters, table)).cloned().collect(),
    ))
}

pub fn sort_collection_data(data: Value, sort_columns: &[SortColumn]) -> Result<Value, AppError> {
    let items = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    let mut sorted = items.to_vec();
    sorted.sort_by(|a, b| compare_items_by_columns(a, b, sort_columns));
    Ok(Value::Array(sorted))
}

pub fn paginate_collection_data(data: Value, pagination: Pagination) -> Result<Value, AppError> {
    let items = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    let total_items = items.len();
    let pages = if total_items == 0 { 1 } else { total_items.div_ceil(pagination.per_page) };
    let page = pagination.page.max(1).min(pages.max(1));
    let start = (page - 1) * pagination.per_page;
    let end = (start + pagination.per_page).min(total_items);
    let data = if start < total_items { items[start..end].to_vec() } else { Vec::new() };

    Ok(serde_json::json!({
        "first": 1,
        "prev": if page > 1 { Some(page - 1) } else { None::<usize> },
        "next": if page < pages { Some(page + 1) } else { None::<usize> },
        "last": pages,
        "page": page,
        "pages": pages,
        "items": total_items,
        "data": data,
    }))
}

pub fn get_value_at_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        let object = current.as_object()?;
        current = object.get(segment)?;
    }
    Some(current)
}

fn compare_items_by_columns(left: &Value, right: &Value, sort_columns: &[SortColumn]) -> Ordering {
    for column in sort_columns {
        let mut cmp = compare_optional_values(
            get_value_at_path(left, &column.field_path),
            get_value_at_path(right, &column.field_path),
        );
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
        FilterOperator::In => {
            condition.value.split(',').map(str::trim).filter(|v| !v.is_empty()).any(|v| {
                compare_with_expected(actual, v, column).is_some_and(|cmp| cmp == Ordering::Equal)
            })
        }
        FilterOperator::Contains => actual
            .as_str()
            .is_some_and(|text| text.to_lowercase().contains(&condition.value.to_lowercase())),
        FilterOperator::StartsWith => actual
            .as_str()
            .is_some_and(|text| text.to_lowercase().starts_with(&condition.value.to_lowercase())),
        FilterOperator::EndsWith => actual
            .as_str()
            .is_some_and(|text| text.to_lowercase().ends_with(&condition.value.to_lowercase())),
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
        ColumnType::String | ColumnType::Json => {
            Some(ComparableValue::String(value_to_filter_string(actual)))
        }
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

pub fn value_to_filter_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "null".to_string(),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_where_like_json_server_cases() {
        let parsed = parse_collection_query_params(vec![
            ("views:gt".to_string(), "100".to_string()),
            ("title:eq".to_string(), "a".to_string()),
            ("views_lt".to_string(), "300".to_string()),
            ("first_name_eq".to_string(), "Alice".to_string()),
            ("author.first_name_ne".to_string(), "Bob".to_string()),
            ("title".to_string(), "hello".to_string()),
            ("id:in".to_string(), "1,3".to_string()),
            ("title:contains".to_string(), "ell".to_string()),
            ("title:startsWith".to_string(), "he".to_string()),
            ("title:endsWith".to_string(), "lo".to_string()),
        ])
        .expect("parse");

        let by_field = |name: &str| parsed.filters.iter().find(|f| f.field_path == name).unwrap();
        let view_operators = parsed
            .filters
            .iter()
            .filter(|f| f.field_path == "views")
            .map(|f| f.operator)
            .collect::<Vec<_>>();
        assert!(view_operators.contains(&FilterOperator::Gt));
        assert!(view_operators.contains(&FilterOperator::Lt));
        assert_eq!(by_field("title").operator, FilterOperator::Eq);
        assert_eq!(by_field("first_name").operator, FilterOperator::Eq);
        assert_eq!(by_field("author.first_name").operator, FilterOperator::Ne);
        assert_eq!(by_field("id").operator, FilterOperator::In);
    }

    #[test]
    fn ignores_unknown_underscore_suffix_as_plain_field() {
        let parsed = parse_collection_query_params(vec![
            ("views_foo".to_string(), "100".to_string()),
            ("title_eq".to_string(), "a".to_string()),
        ])
        .expect("parse");

        assert_eq!(parsed.filters[0].field_path, "views_foo");
        assert_eq!(parsed.filters[0].operator, FilterOperator::Eq);
        assert_eq!(parsed.filters[1].field_path, "title");
        assert_eq!(parsed.filters[1].operator, FilterOperator::Eq);
    }

    #[test]
    fn matches_where_like_operators() {
        let obj = json!({"a": 10, "b": 20, "c": "x", "nested": {"a": 10, "b": 20}});

        let cases = [
            (vec![("a:eq", "10")], true),
            (vec![("a:eq", "11")], false),
            (vec![("c:ne", "y")], true),
            (vec![("c:ne", "x")], false),
            (vec![("a:lt", "11")], true),
            (vec![("a:lt", "10")], false),
            (vec![("a:lte", "10")], true),
            (vec![("a:lte", "9")], false),
            (vec![("b:gt", "19")], true),
            (vec![("b:gt", "20")], false),
            (vec![("b:gte", "20")], true),
            (vec![("b:gte", "21")], false),
            (vec![("nested.a:eq", "10")], true),
            (vec![("nested.b:lt", "20")], false),
            (vec![("a:in", "10,11")], true),
            (vec![("a:in", "1,2")], false),
            (vec![("c:contains", "X")], true),
            (vec![("c:startsWith", "X")], true),
            (vec![("c:endsWith", "X")], true),
        ];

        for (filters, expected) in cases {
            let parsed = parse_collection_query_params(
                filters.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>(),
            )
            .expect("parse");
            let matches = filter_collection_data(json!([obj.clone()]), &parsed.filters, None)
                .expect("filter")
                .as_array()
                .expect("array")
                .len()
                == 1;
            assert_eq!(matches, expected, "filters: {filters:?}");
        }
    }

    #[test]
    fn parses_filter_queries_in_json_server_style() {
        let cases: Vec<(&str, Vec<(&str, FilterOperator, &str)>)> = vec![
            (
                "views:gt=100&title:eq=a",
                vec![("views", FilterOperator::Gt, "100"), ("title", FilterOperator::Eq, "a")],
            ),
            ("title=hello", vec![("title", FilterOperator::Eq, "hello")]),
            (
                "author.name:lt=c&author.id:ne=2",
                vec![
                    ("author.name", FilterOperator::Lt, "c"),
                    ("author.id", FilterOperator::Ne, "2"),
                ],
            ),
            (
                "views:gt=100&views:lt=300",
                vec![("views", FilterOperator::Gt, "100"), ("views", FilterOperator::Lt, "300")],
            ),
            (
                "views_gt=100&title_eq=a",
                vec![("views", FilterOperator::Gt, "100"), ("title", FilterOperator::Eq, "a")],
            ),
            (
                "first_name_eq=Alice&author.first_name_ne=Bob",
                vec![
                    ("first_name", FilterOperator::Eq, "Alice"),
                    ("author.first_name", FilterOperator::Ne, "Bob"),
                ],
            ),
            (
                "id:in=1,3&title:contains=ello&title:startsWith=hel&title:endsWith=rld",
                vec![
                    ("id", FilterOperator::In, "1,3"),
                    ("title", FilterOperator::Contains, "ello"),
                    ("title", FilterOperator::StartsWith, "hel"),
                    ("title", FilterOperator::EndsWith, "rld"),
                ],
            ),
        ];

        for (query, expected) in cases {
            let parsed = parse_collection_query_params(parse_query(query)).expect("parse");
            let actual = parsed
                .filters
                .iter()
                .map(|f| (f.field_path.as_str(), f.operator, f.value.as_str()))
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "query: {query}");
        }
    }

    #[test]
    fn matches_filters_in_json_server_style() {
        let obj = json!({"a": 10, "b": 20, "c": "x", "nested": {"a": 10, "b": 20}});

        let cases: Vec<(&str, bool)> = vec![
            ("a:eq=10", true),
            ("a:eq=11", false),
            ("c:ne=y", true),
            ("c:ne=x", false),
            ("a:lt=11", true),
            ("a:lt=10", false),
            ("a:lte=10", true),
            ("a:lte=9", false),
            ("b:gt=19", true),
            ("b:gt=20", false),
            ("b:gte=20", true),
            ("b:gte=21", false),
            ("a:gt=0&b:lt=30", true),
            ("a:gt=10&b:lt=30", false),
            ("nested.a:eq=10", true),
            ("nested.b:lt=20", false),
            ("a:in=10,11", true),
            ("a:in=1,2", false),
            ("c:in=x,y", true),
            ("c:in=y,z", false),
            ("a:in=10,11&a:gt=9", true),
            ("a:in=10,11&a:gt=10", false),
            ("c:contains=x", true),
            ("c:contains=X", true),
            ("c:contains=z", false),
            ("a:contains=1", false),
            ("c:startsWith=x", true),
            ("c:startsWith=X", true),
            ("c:startsWith=z", false),
            ("a:startsWith=1", false),
            ("c:endsWith=x", true),
            ("c:endsWith=X", true),
            ("c:endsWith=z", false),
            ("a:endsWith=1", false),
        ];

        for (query, expected) in cases {
            let parsed = parse_collection_query_params(parse_query(query)).expect("parse");
            let matches = filter_collection_data(json!([obj.clone()]), &parsed.filters, None)
                .expect("filter")
                .as_array()
                .expect("array")
                .len()
                == 1;
            assert_eq!(matches, expected, "query: {query}");
        }
    }

    fn parse_query(query: &str) -> Vec<(String, String)> {
        query
            .split('&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| {
                let (key, value) = pair.split_once('=').expect("query pair with equals");
                (key.to_string(), value.to_string())
            })
            .collect()
    }
    #[test]
    fn paginates_like_json_server_boundaries() {
        let p1 =
            paginate_collection_data(json!([1, 2, 3, 4, 5]), Pagination { page: 1, per_page: 2 })
                .expect("paginate");
        assert_eq!(p1["first"], 1);
        assert_eq!(p1["prev"], Value::Null);
        assert_eq!(p1["next"], 2);
        assert_eq!(p1["last"], 3);
        assert_eq!(p1["pages"], 3);
        assert_eq!(p1["items"], 5);
        assert_eq!(p1["data"], json!([1, 2]));

        let p2 =
            paginate_collection_data(json!([1, 2, 3, 4, 5]), Pagination { page: 2, per_page: 2 })
                .expect("paginate");
        assert_eq!(p2["prev"], 1);
        assert_eq!(p2["next"], 3);
        assert_eq!(p2["data"], json!([3, 4]));

        let plast =
            paginate_collection_data(json!([1, 2, 3, 4, 5]), Pagination { page: 9, per_page: 2 })
                .expect("paginate");
        assert_eq!(plast["prev"], 2);
        assert_eq!(plast["next"], Value::Null);
        assert_eq!(plast["data"], json!([5]));

        let p0 = paginate_collection_data(json!([1, 2, 3]), Pagination { page: 0, per_page: 2 })
            .expect("paginate");
        assert_eq!(p0["page"], 1);
        assert_eq!(p0["data"], json!([1, 2]));
    }
}
