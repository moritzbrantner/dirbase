use serde_json::Value;

use crate::schema::{ColumnType, DeclaredTableSchema};

pub(super) fn declared_schema_expects_object(table: &DeclaredTableSchema) -> bool {
    matches!(table.kind, Some(crate::schema::TableKind::Object))
}

pub(super) fn value_matches_type(value: &Value, column_type: &ColumnType) -> bool {
    if value.is_null() {
        return true;
    }
    match column_type {
        ColumnType::Integer => value.as_i64().is_some() || value.as_u64().is_some(),
        ColumnType::Float => value.is_number(),
        ColumnType::Boolean => value.is_boolean(),
        ColumnType::String => value.is_string(),
        ColumnType::Json => true,
        ColumnType::Date => value
            .as_str()
            .is_some_and(|text| chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok()),
        ColumnType::DateTime => {
            value.as_str().is_some_and(|text| chrono::DateTime::parse_from_rfc3339(text).is_ok())
        }
        ColumnType::Uuid => value.as_str().is_some_and(|text| uuid::Uuid::parse_str(text).is_ok()),
        ColumnType::BigInteger => value_is_big_integer(value),
        ColumnType::Decimal => value_is_decimal(value),
    }
}

fn value_is_big_integer(value: &Value) -> bool {
    value.as_i64().is_some()
        || value.as_u64().is_some()
        || value.as_str().is_some_and(is_big_integer_literal)
}

fn value_is_decimal(value: &Value) -> bool {
    value.is_number() || value.as_str().is_some_and(is_decimal_literal)
}

fn is_big_integer_literal(text: &str) -> bool {
    let rest = text.strip_prefix(['-', '+']).unwrap_or(text);
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

fn is_decimal_literal(text: &str) -> bool {
    let rest = text.strip_prefix(['-', '+']).unwrap_or(text);
    if rest.is_empty() {
        return false;
    }
    let mut parts = rest.split('.');
    let first = parts.next().unwrap_or_default();
    let second = parts.next();
    if parts.next().is_some() {
        return false;
    }
    let first_ok = !first.is_empty() && first.chars().all(|c| c.is_ascii_digit());
    match second {
        Some(fraction) => {
            first_ok && !fraction.is_empty() && fraction.chars().all(|c| c.is_ascii_digit())
        }
        None => first_ok,
    }
}
