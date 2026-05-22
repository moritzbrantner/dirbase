use std::cmp::Ordering;

use axum::http::StatusCode;
use serde_json::Value;

use crate::{
    error::AppError,
    schema::{ColumnSchema, ColumnType, TableSchema},
};

use super::types::{ComparableValue, FilterCondition, FilterOperator};

pub fn filter_collection_data(
    data: Value,
    filters: &[FilterCondition],
    table: Option<&TableSchema>,
) -> Result<Value, AppError> {
    let items = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    Ok(Value::Array(filter_collection_refs(items, filters, table).into_iter().cloned().collect()))
}

pub fn filter_collection_refs<'a>(
    items: &'a [Value],
    filters: &[FilterCondition],
    table: Option<&TableSchema>,
) -> Vec<&'a Value> {
    items.iter().filter(|item| item_matches_filters(item, filters, table)).collect()
}

pub fn get_value_at_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        let object = current.as_object()?;
        current = object.get(segment)?;
    }
    Some(current)
}

fn item_matches_filters(
    item: &Value,
    filters: &[FilterCondition],
    table: Option<&TableSchema>,
) -> bool {
    filters.iter().all(|condition| {
        let actual = get_value_at_segments(item, &condition.field_segments).unwrap_or(&Value::Null);
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
        FilterOperator::Eq => compare_with_prepared(actual, condition, column)
            .is_some_and(|cmp| cmp == Ordering::Equal),
        FilterOperator::Ne => compare_with_prepared(actual, condition, column)
            .is_some_and(|cmp| cmp != Ordering::Equal),
        FilterOperator::Lt => compare_with_prepared(actual, condition, column)
            .is_some_and(|cmp| cmp == Ordering::Less),
        FilterOperator::Lte => compare_with_prepared(actual, condition, column)
            .is_some_and(|cmp| cmp == Ordering::Less || cmp == Ordering::Equal),
        FilterOperator::Gt => compare_with_prepared(actual, condition, column)
            .is_some_and(|cmp| cmp == Ordering::Greater),
        FilterOperator::Gte => compare_with_prepared(actual, condition, column)
            .is_some_and(|cmp| cmp == Ordering::Greater || cmp == Ordering::Equal),
        FilterOperator::In => condition.prepared_in_values.iter().any(|prepared| {
            compare_with_prepared_value(actual, prepared, &condition.value, column)
                .is_some_and(|cmp| cmp == Ordering::Equal)
        }),
        FilterOperator::Contains => {
            actual.as_str().is_some_and(|text| text.to_lowercase().contains(&condition.value_lower))
        }
        FilterOperator::StartsWith => actual
            .as_str()
            .is_some_and(|text| text.to_lowercase().starts_with(&condition.value_lower)),
        FilterOperator::EndsWith => actual
            .as_str()
            .is_some_and(|text| text.to_lowercase().ends_with(&condition.value_lower)),
    }
}
fn compare_with_prepared(
    actual: &Value,
    condition: &FilterCondition,
    column: Option<&ColumnSchema>,
) -> Option<Ordering> {
    compare_with_prepared_value(actual, &condition.prepared_value, &condition.value, column)
}
fn compare_with_prepared_value(
    actual: &Value,
    prepared: &ComparableValue,
    raw_expected: &str,
    column: Option<&ColumnSchema>,
) -> Option<Ordering> {
    let left = coerce_actual_value(actual, column)?;
    if column.is_some() {
        let right = coerce_expected_value(raw_expected, column)?;
        return compare_comparable_values(&left, &right);
    }
    compare_comparable_values(&left, prepared)
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
        ColumnType::Integer | ColumnType::Float | ColumnType::BigInteger | ColumnType::Decimal => {
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
        ColumnType::String
        | ColumnType::Json
        | ColumnType::Date
        | ColumnType::DateTime
        | ColumnType::Uuid => Some(ComparableValue::String(value_to_filter_string(actual))),
    }
}
fn coerce_expected_value(expected: &str, column: Option<&ColumnSchema>) -> Option<ComparableValue> {
    if expected.eq_ignore_ascii_case("null") {
        return Some(ComparableValue::Null);
    }
    if let Some(column) = column {
        return match column.column_type {
            ColumnType::Integer
            | ColumnType::Float
            | ColumnType::BigInteger
            | ColumnType::Decimal => expected.parse::<f64>().ok().map(ComparableValue::Number),
            ColumnType::Boolean => expected.parse::<bool>().ok().map(ComparableValue::Bool),
            ColumnType::String
            | ColumnType::Json
            | ColumnType::Date
            | ColumnType::DateTime
            | ColumnType::Uuid => Some(ComparableValue::String(expected.to_string())),
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

fn get_value_at_segments<'a>(value: &'a Value, segments: &[String]) -> Option<&'a Value> {
    let mut current = value;
    for segment in segments {
        let object = current.as_object()?;
        current = object.get(segment.as_str())?;
    }
    Some(current)
}

pub fn value_to_filter_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "null".to_string(),
        _ => value.to_string(),
    }
}
