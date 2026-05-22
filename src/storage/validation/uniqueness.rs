use axum::http::StatusCode;
use serde_json::Value;

use crate::{error::AppError, schema::DeclaredTableSchema};

pub(super) fn validate_unique_constraints(
    resource: &str,
    array: &[Value],
    table: &DeclaredTableSchema,
) -> Result<(), AppError> {
    for constraint in &table.unique {
        let mut seen = std::collections::BTreeSet::new();
        for row in array {
            let Some(object) = row.as_object() else {
                continue;
            };
            let mut parts = Vec::with_capacity(constraint.len());
            let mut skip = false;
            for column_name in constraint {
                match object.get(column_name) {
                    Some(value) if !value.is_null() => {
                        parts.push(unique_value_key(value));
                    }
                    _ => {
                        skip = true;
                        break;
                    }
                }
            }
            if skip {
                continue;
            }
            let key = parts.join("\x1f");
            if !seen.insert(key) {
                return Err(AppError::new(
                    StatusCode::BAD_REQUEST,
                    format!(
                        "Resource '{resource}' violates unique constraint on '{}'",
                        constraint.join(", ")
                    ),
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn validate_primary_key_uniqueness(
    resource: &str,
    array: &[Value],
    table: &DeclaredTableSchema,
) -> Result<(), AppError> {
    let Some(primary_key) = table.primary_key.as_deref() else {
        return Ok(());
    };
    let mut seen = std::collections::BTreeSet::new();
    for row in array {
        let Some(value) = row.as_object().and_then(|object| object.get(primary_key)) else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        let key = unique_value_key(value);
        if !seen.insert(key) {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                format!("Resource '{resource}' has duplicate primary key '{primary_key}'"),
            ));
        }
    }
    Ok(())
}

fn unique_value_key(value: &Value) -> String {
    match value {
        Value::String(value) => format!("s:{value}"),
        Value::Number(value) => format!("n:{value}"),
        Value::Bool(value) => format!("b:{value}"),
        Value::Null => "null".to_string(),
        Value::Array(_) | Value::Object(_) => {
            format!("j:{}", serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()))
        }
    }
}
