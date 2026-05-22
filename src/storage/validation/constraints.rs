use serde_json::Value;

use crate::schema::{ColumnSchema, ColumnType};

pub(super) fn validate_column_constraints(
    value: &Value,
    column: &ColumnSchema,
) -> Result<(), &'static str> {
    if let Some(enum_values) = &column.enum_values {
        let Some(text) = value.as_str() else {
            return Err("enum");
        };
        if !enum_values.iter().any(|allowed| allowed == text) {
            return Err("enum");
        }
    }

    if let Some(text) = value_as_constraint_string(value, &column.column_type) {
        let length = text.chars().count();
        if column.min_length.is_some_and(|min| length < min) {
            return Err("min_length");
        }
        if column.max_length.is_some_and(|max| length > max) {
            return Err("max_length");
        }
        if let Some(pattern) = &column.pattern
            && !regex::Regex::new(pattern).map(|regex| regex.is_match(text)).unwrap_or(false)
        {
            return Err("pattern");
        }
    }

    if let Some(actual) = numeric_value(value, &column.column_type) {
        if let Some(min) = &column.min
            && actual < min.as_f64().unwrap_or(f64::NEG_INFINITY)
        {
            return Err("min");
        }
        if let Some(max) = &column.max
            && actual > max.as_f64().unwrap_or(f64::INFINITY)
        {
            return Err("max");
        }
    }
    if let Some(actual) = date_value(value, &column.column_type) {
        if let Some(min) = &column.min
            && let Some(min) = min.as_str().and_then(parse_date_bound)
            && actual < min
        {
            return Err("min");
        }
        if let Some(max) = &column.max
            && let Some(max) = max.as_str().and_then(parse_date_bound)
            && actual > max
        {
            return Err("max");
        }
    }
    if let Some(actual) = datetime_value(value, &column.column_type) {
        if let Some(min) = &column.min
            && let Some(min) = min.as_str().and_then(parse_datetime_bound)
            && actual < min
        {
            return Err("min");
        }
        if let Some(max) = &column.max
            && let Some(max) = max.as_str().and_then(parse_datetime_bound)
            && actual > max
        {
            return Err("max");
        }
    }

    Ok(())
}

fn value_as_constraint_string<'a>(value: &'a Value, column_type: &ColumnType) -> Option<&'a str> {
    match column_type {
        ColumnType::String
        | ColumnType::Date
        | ColumnType::DateTime
        | ColumnType::Uuid
        | ColumnType::BigInteger
        | ColumnType::Decimal => value.as_str(),
        _ => None,
    }
}

fn numeric_value(value: &Value, column_type: &ColumnType) -> Option<f64> {
    match column_type {
        ColumnType::Integer | ColumnType::Float | ColumnType::BigInteger | ColumnType::Decimal => {
            value.as_f64().or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
        }
        _ => None,
    }
}

fn date_value(value: &Value, column_type: &ColumnType) -> Option<chrono::NaiveDate> {
    if !matches!(column_type, ColumnType::Date) {
        return None;
    }
    value.as_str().and_then(parse_date_bound)
}

fn datetime_value(
    value: &Value,
    column_type: &ColumnType,
) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    if !matches!(column_type, ColumnType::DateTime) {
        return None;
    }
    value.as_str().and_then(parse_datetime_bound)
}

fn parse_date_bound(text: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
}

fn parse_datetime_bound(text: &str) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    chrono::DateTime::parse_from_rfc3339(text).ok()
}
