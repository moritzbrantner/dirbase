use std::cmp::Ordering;

use axum::http::StatusCode;
use serde_json::Value;

use crate::error::AppError;

use super::{
    evaluator::{get_value_at_path, value_to_filter_string},
    types::SortColumn,
};

pub fn sort_collection_data(data: Value, sort_columns: &[SortColumn]) -> Result<Value, AppError> {
    let items = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    let mut sorted = items.iter().collect::<Vec<_>>();
    sort_collection_refs(sorted.as_mut_slice(), sort_columns);
    Ok(Value::Array(sorted.into_iter().cloned().collect()))
}

pub fn sort_collection_refs(items: &mut [&Value], sort_columns: &[SortColumn]) {
    items.sort_by(|a, b| compare_items_by_columns(a, b, sort_columns));
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
