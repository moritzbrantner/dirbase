use axum::http::StatusCode;
use serde_json::Value;

use crate::{
    app::AppState,
    error::AppError,
    schema::{ColumnType, DeclaredTableSchema, is_valid_identifier},
};

pub fn validate_resource_data(
    state: &AppState,
    resource: &str,
    data: &Value,
) -> Result<(), AppError> {
    let Some(table) = state.validation_schema_table(resource) else {
        return Ok(());
    };
    match data {
        Value::Array(array) => validate_array_resource_data(resource, array, &table),
        Value::Object(object) => validate_object_resource_data(resource, object, &table),
        _ if declared_schema_expects_object(&table) => Err(AppError::new(
            StatusCode::BAD_REQUEST,
            format!("Resource '{resource}' is not a JSON object"),
        )),
        _ => Err(AppError::new(
            StatusCode::BAD_REQUEST,
            format!("Resource '{resource}' is not a JSON array"),
        )),
    }
}

pub fn validate_sql_identifier(identifier: &str, kind: &str) -> Result<(), AppError> {
    if !is_valid_identifier(identifier) {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            format!("Invalid {kind} identifier '{identifier}'"),
        ));
    }
    Ok(())
}

fn validate_array_resource_data(
    resource: &str,
    array: &[Value],
    table: &DeclaredTableSchema,
) -> Result<(), AppError> {
    for (index, item) in array.iter().enumerate() {
        let object = item.as_object().ok_or_else(|| {
            AppError::new(
                StatusCode::BAD_REQUEST,
                format!("Row {index} in resource '{resource}' is not an object"),
            )
        })?;
        validate_declared_columns(
            table,
            |column_name| object.get(column_name),
            |column_name| {
                format!(
                    "Row {index} in resource '{resource}' is missing non-null column '{column_name}'"
                )
            },
            |column_name| {
                format!(
                    "Row {index} in resource '{resource}' has null for non-null column '{column_name}'"
                )
            },
            |column_name| {
                format!("Row {index} in resource '{resource}' has invalid type for '{column_name}'")
            },
            |column_name| {
                format!(
                    "Row {index} in resource '{resource}' has value outside enum for '{column_name}'"
                )
            },
            |column_name, constraint| {
                format!(
                    "Row {index} in resource '{resource}' violates {constraint} for '{column_name}'"
                )
            },
        )?;
    }

    validate_unique_constraints(resource, array, table)?;
    validate_primary_key_uniqueness(resource, array, table)?;
    Ok(())
}

fn validate_object_resource_data(
    resource: &str,
    object: &serde_json::Map<String, Value>,
    table: &DeclaredTableSchema,
) -> Result<(), AppError> {
    validate_declared_columns(
        table,
        |column_name| object.get(column_name),
        |column_name| format!("Resource '{resource}' is missing non-null column '{column_name}'"),
        |column_name| format!("Resource '{resource}' has null for non-null column '{column_name}'"),
        |column_name| format!("Resource '{resource}' has invalid type for '{column_name}'"),
        |column_name| format!("Resource '{resource}' has value outside enum for '{column_name}'"),
        |column_name, constraint| {
            format!("Resource '{resource}' violates {constraint} for '{column_name}'")
        },
    )
}

fn validate_declared_columns<'a>(
    table: &DeclaredTableSchema,
    lookup: impl Fn(&str) -> Option<&'a Value>,
    missing_message: impl Fn(&str) -> String,
    null_message: impl Fn(&str) -> String,
    type_message: impl Fn(&str) -> String,
    enum_message: impl Fn(&str) -> String,
    constraint_message: impl Fn(&str, &str) -> String,
) -> Result<(), AppError> {
    for (column_name, column) in &table.columns {
        match lookup(column_name) {
            Some(Value::Null) if !column.nullable => {
                return Err(AppError::new(StatusCode::BAD_REQUEST, null_message(column_name)));
            }
            Some(value) if !value_matches_type(value, &column.column_type) => {
                return Err(AppError::new(StatusCode::BAD_REQUEST, type_message(column_name)));
            }
            Some(value) if !value.is_null() => {
                validate_column_constraints(value, column).map_err(|constraint| {
                    AppError::new(
                        StatusCode::BAD_REQUEST,
                        if constraint == "enum" {
                            enum_message(column_name)
                        } else {
                            constraint_message(column_name, constraint)
                        },
                    )
                })?;
            }
            None if !column.nullable => {
                return Err(AppError::new(StatusCode::BAD_REQUEST, missing_message(column_name)));
            }
            _ => {}
        }
    }

    Ok(())
}

fn declared_schema_expects_object(table: &DeclaredTableSchema) -> bool {
    matches!(table.kind, Some(crate::schema::TableKind::Object))
}

fn value_matches_type(value: &Value, column_type: &ColumnType) -> bool {
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

fn validate_column_constraints(
    value: &Value,
    column: &crate::schema::ColumnSchema,
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

fn validate_unique_constraints(
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

fn validate_primary_key_uniqueness(
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

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet, HashMap},
        path::PathBuf,
        sync::Arc,
    };

    use axum::http::StatusCode;
    use serde_json::json;
    use tokio::sync::RwLock;

    use super::*;
    use crate::{
        app::{
            AppConfig, AppState, DataSource, GraphqlStore, HealthState, MetricsStore,
            ResponseFormat, SchemaStore,
        },
        schema::{ColumnSchema, DeclaredSchema, Schema, TableKind},
    };

    fn test_state_with_declared_schema(
        resource: &str,
        declared_table_schema: DeclaredTableSchema,
    ) -> AppState {
        let mut tables = BTreeMap::new();
        tables.insert(resource.to_string(), declared_table_schema);

        AppState {
            data_source: Arc::new(DataSource::Folder(PathBuf::from("."))),
            config: Arc::new(AppConfig {
                readonly: false,
                enable_log: false,
                response_format: ResponseFormat::Json,
                auth_token: None,
                cors_origin: None,
                max_body_bytes: 1024 * 1024,
                max_per_page: 100,
                max_sql_scan_rows: 50_000,
                max_sql_selected_rows: 1_000,
            }),
            resources: Arc::new(RwLock::new(BTreeSet::from([resource.to_string()]))),
            resource_cache: Arc::new(RwLock::new(HashMap::new())),
            resource_locks: Arc::new(RwLock::new(HashMap::new())),
            schema_store: Arc::new(std::sync::RwLock::new(
                SchemaStore::new(Some(DeclaredSchema { tables }), Schema::default())
                    .expect("valid schema store"),
            )),
            graphql_store: Arc::new(RwLock::new(GraphqlStore::default())),
            metrics: Arc::new(MetricsStore::default()),
            health: Arc::new(HealthState::new(true, None)),
            event_bus: tokio::sync::broadcast::channel(16).0,
        }
    }

    fn column(name: &str, column_type: ColumnType, nullable: bool) -> (String, ColumnSchema) {
        (name.to_string(), ColumnSchema::new(column_type, nullable))
    }

    #[test]
    fn validate_resource_data_accepts_array_rows_matching_declared_schema() {
        let state = test_state_with_declared_schema(
            "users",
            DeclaredTableSchema {
                columns: BTreeMap::from([
                    column("id", ColumnType::Integer, false),
                    column("name", ColumnType::String, false),
                    column("active", ColumnType::Boolean, true),
                ]),
                ..DeclaredTableSchema::default()
            },
        );

        let result = validate_resource_data(
            &state,
            "users",
            &json!([
                {"id": 1, "name": "Ada", "active": true},
                {"id": 2, "name": "Grace", "active": null}
            ]),
        );

        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn validate_resource_data_accepts_extended_scalar_types() {
        let state = test_state_with_declared_schema(
            "events",
            DeclaredTableSchema {
                columns: BTreeMap::from([
                    column("id", ColumnType::Uuid, false),
                    column("starts_on", ColumnType::Date, false),
                    column("starts_at", ColumnType::DateTime, false),
                    column("counter", ColumnType::BigInteger, false),
                    column("amount", ColumnType::Decimal, false),
                ]),
                ..DeclaredTableSchema::default()
            },
        );

        let result = validate_resource_data(
            &state,
            "events",
            &json!([
                {
                    "id": "550e8400-e29b-41d4-a716-446655440000",
                    "starts_on": "2026-04-29",
                    "starts_at": "2026-04-29T12:30:00Z",
                    "counter": "9223372036854775808",
                    "amount": "12345.67"
                }
            ]),
        );

        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn validate_resource_data_rejects_invalid_extended_scalar_types() {
        let state = test_state_with_declared_schema(
            "events",
            DeclaredTableSchema {
                columns: BTreeMap::from([column("id", ColumnType::Uuid, false)]),
                ..DeclaredTableSchema::default()
            },
        );

        let err = validate_resource_data(&state, "events", &json!([{"id": "not-a-uuid"}]))
            .expect_err("invalid uuid");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("invalid type for 'id'"));
    }

    #[test]
    fn validate_resource_data_rejects_invalid_extended_scalar_literals() {
        for (column_type, value) in [
            (ColumnType::Date, json!("2026-02-30")),
            (ColumnType::DateTime, json!("2026-04-29 12:30:00")),
            (ColumnType::BigInteger, json!("12.5")),
            (ColumnType::Decimal, json!("12.")),
        ] {
            let state = test_state_with_declared_schema(
                "events",
                DeclaredTableSchema {
                    columns: BTreeMap::from([column("value", column_type, false)]),
                    ..DeclaredTableSchema::default()
                },
            );

            let err = validate_resource_data(&state, "events", &json!([{ "value": value }]))
                .expect_err("invalid extended scalar");
            assert_eq!(err.status, StatusCode::BAD_REQUEST);
            assert!(err.message.contains("invalid type for 'value'"), "{}", err.message);
        }
    }

    #[test]
    fn validate_resource_data_enforces_enum_and_scalar_constraints() {
        let mut status = ColumnSchema::new(ColumnType::String, false);
        status.enum_values = Some(vec!["draft".to_string(), "published".to_string()]);
        let mut slug = ColumnSchema::new(ColumnType::String, false);
        slug.min_length = Some(3);
        slug.max_length = Some(12);
        slug.pattern = Some("^[a-z0-9-]+$".to_string());
        let mut score = ColumnSchema::new(ColumnType::Integer, false);
        score.min = Some(Value::from(1));
        score.max = Some(Value::from(5));
        let mut published_on = ColumnSchema::new(ColumnType::Date, false);
        published_on.min = Some(Value::from("2026-01-01"));
        published_on.max = Some(Value::from("2026-12-31"));

        let state = test_state_with_declared_schema(
            "posts",
            DeclaredTableSchema {
                columns: BTreeMap::from([
                    ("status".to_string(), status),
                    ("slug".to_string(), slug),
                    ("score".to_string(), score),
                    ("published_on".to_string(), published_on),
                ]),
                ..DeclaredTableSchema::default()
            },
        );

        let ok = validate_resource_data(
            &state,
            "posts",
            &json!([{"status": "draft", "slug": "hello-1", "score": 3, "published_on": "2026-04-29"}]),
        );
        assert!(ok.is_ok(), "{ok:?}");

        let err = validate_resource_data(
            &state,
            "posts",
            &json!([{"status": "archived", "slug": "hello-1", "score": 3, "published_on": "2026-04-29"}]),
        )
        .expect_err("enum");
        assert!(err.message.contains("value outside enum"));

        let err = validate_resource_data(
            &state,
            "posts",
            &json!([{"status": "draft", "slug": "NOPE", "score": 3, "published_on": "2026-04-29"}]),
        )
        .expect_err("pattern");
        assert!(err.message.contains("violates pattern"));

        let err = validate_resource_data(
            &state,
            "posts",
            &json!([{"status": "draft", "slug": "hello-1", "score": 6, "published_on": "2026-04-29"}]),
        )
        .expect_err("max");
        assert!(err.message.contains("violates max"));

        let err = validate_resource_data(
            &state,
            "posts",
            &json!([{"status": "draft", "slug": "hello-1", "score": 3, "published_on": "2027-01-01"}]),
        )
        .expect_err("date max");
        assert!(err.message.contains("violates max"));
    }

    #[test]
    fn validate_resource_data_enforces_unique_and_primary_key_constraints() {
        let state = test_state_with_declared_schema(
            "users",
            DeclaredTableSchema {
                primary_key: Some("id".to_string()),
                columns: BTreeMap::from([
                    column("id", ColumnType::Integer, false),
                    column("email", ColumnType::String, true),
                ]),
                unique: vec![vec!["email".to_string()]],
                ..DeclaredTableSchema::default()
            },
        );

        let duplicate_email = validate_resource_data(
            &state,
            "users",
            &json!([
                {"id": 1, "email": "ada@example.com"},
                {"id": 2, "email": "ada@example.com"}
            ]),
        )
        .expect_err("unique");
        assert!(duplicate_email.message.contains("violates unique constraint on 'email'"));

        let duplicate_id = validate_resource_data(
            &state,
            "users",
            &json!([
                {"id": 1, "email": "ada@example.com"},
                {"id": 1, "email": "grace@example.com"}
            ]),
        )
        .expect_err("primary key");
        assert!(duplicate_id.message.contains("duplicate primary key 'id'"));

        let nulls = validate_resource_data(
            &state,
            "users",
            &json!([
                {"id": 1, "email": null},
                {"id": 2}
            ]),
        );
        assert!(nulls.is_ok(), "{nulls:?}");
    }

    #[test]
    fn validate_resource_data_rejects_array_row_missing_non_null_column() {
        let state = test_state_with_declared_schema(
            "users",
            DeclaredTableSchema {
                columns: BTreeMap::from([column("name", ColumnType::String, false)]),
                ..DeclaredTableSchema::default()
            },
        );

        let err = validate_resource_data(&state, "users", &json!([{}])).expect_err("missing field");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(
            err.message.contains("Row 0 in resource 'users' is missing non-null column 'name'")
        );
    }

    #[test]
    fn validate_resource_data_rejects_array_row_null_non_null_column() {
        let state = test_state_with_declared_schema(
            "users",
            DeclaredTableSchema {
                columns: BTreeMap::from([column("name", ColumnType::String, false)]),
                ..DeclaredTableSchema::default()
            },
        );

        let err =
            validate_resource_data(&state, "users", &json!([{"name": null}])).expect_err("null");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(
            err.message.contains("Row 0 in resource 'users' has null for non-null column 'name'")
        );
    }

    #[test]
    fn validate_resource_data_rejects_array_row_wrong_integer_type() {
        let state = test_state_with_declared_schema(
            "users",
            DeclaredTableSchema {
                columns: BTreeMap::from([column("id", ColumnType::Integer, false)]),
                ..DeclaredTableSchema::default()
            },
        );

        let err = validate_resource_data(&state, "users", &json!([{"id": "1"}])).expect_err("type");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Row 0 in resource 'users' has invalid type for 'id'"));
    }

    #[test]
    fn validate_resource_data_rejects_array_row_wrong_float_type() {
        let state = test_state_with_declared_schema(
            "prices",
            DeclaredTableSchema {
                columns: BTreeMap::from([column("amount", ColumnType::Float, false)]),
                ..DeclaredTableSchema::default()
            },
        );

        let err = validate_resource_data(&state, "prices", &json!([{"amount": "12.5"}]))
            .expect_err("type");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Row 0 in resource 'prices' has invalid type for 'amount'"));
    }

    #[test]
    fn validate_resource_data_rejects_array_row_wrong_boolean_type() {
        let state = test_state_with_declared_schema(
            "users",
            DeclaredTableSchema {
                columns: BTreeMap::from([column("active", ColumnType::Boolean, false)]),
                ..DeclaredTableSchema::default()
            },
        );

        let err = validate_resource_data(&state, "users", &json!([{"active": "true"}]))
            .expect_err("type");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Row 0 in resource 'users' has invalid type for 'active'"));
    }

    #[test]
    fn validate_resource_data_rejects_array_row_wrong_string_type() {
        let state = test_state_with_declared_schema(
            "users",
            DeclaredTableSchema {
                columns: BTreeMap::from([column("name", ColumnType::String, false)]),
                ..DeclaredTableSchema::default()
            },
        );

        let err = validate_resource_data(&state, "users", &json!([{"name": 7}])).expect_err("type");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Row 0 in resource 'users' has invalid type for 'name'"));
    }

    #[test]
    fn validate_resource_data_accepts_extra_undeclared_columns() {
        let state = test_state_with_declared_schema(
            "users",
            DeclaredTableSchema {
                columns: BTreeMap::from([column("id", ColumnType::Integer, false)]),
                ..DeclaredTableSchema::default()
            },
        );

        let result =
            validate_resource_data(&state, "users", &json!([{"id": 1, "name": "Ada", "age": 37}]));
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn validate_resource_data_accepts_object_matching_declared_schema() {
        let state = test_state_with_declared_schema(
            "profile",
            DeclaredTableSchema {
                kind: Some(TableKind::Object),
                columns: BTreeMap::from([
                    column("name", ColumnType::String, false),
                    column("age", ColumnType::Integer, true),
                ]),
                ..DeclaredTableSchema::default()
            },
        );

        let result = validate_resource_data(&state, "profile", &json!({"name": "Ada", "age": 37}));
        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn validate_resource_data_rejects_object_missing_non_null_column() {
        let state = test_state_with_declared_schema(
            "profile",
            DeclaredTableSchema {
                kind: Some(TableKind::Object),
                columns: BTreeMap::from([column("name", ColumnType::String, false)]),
                ..DeclaredTableSchema::default()
            },
        );

        let err = validate_resource_data(&state, "profile", &json!({})).expect_err("missing field");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Resource 'profile' is missing non-null column 'name'"));
    }

    #[test]
    fn validate_resource_data_rejects_object_wrong_type() {
        let state = test_state_with_declared_schema(
            "profile",
            DeclaredTableSchema {
                kind: Some(TableKind::Object),
                columns: BTreeMap::from([column("age", ColumnType::Integer, false)]),
                ..DeclaredTableSchema::default()
            },
        );

        let err =
            validate_resource_data(&state, "profile", &json!({"age": "old"})).expect_err("type");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Resource 'profile' has invalid type for 'age'"));
    }

    #[test]
    fn validate_resource_data_rejects_scalar_when_object_expected() {
        let state = test_state_with_declared_schema(
            "profile",
            DeclaredTableSchema { kind: Some(TableKind::Object), ..DeclaredTableSchema::default() },
        );

        let err = validate_resource_data(&state, "profile", &json!(1)).expect_err("object");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Resource 'profile' is not a JSON object"));
    }

    #[test]
    fn validate_resource_data_rejects_scalar_when_array_expected() {
        let state = test_state_with_declared_schema("users", DeclaredTableSchema::default());

        let err = validate_resource_data(&state, "users", &json!(1)).expect_err("array");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Resource 'users' is not a JSON array"));
    }

    #[test]
    fn validate_sql_identifier_accepts_valid_identifier() {
        assert!(validate_sql_identifier("user_profiles-2024", "table").is_ok());
    }

    #[test]
    fn validate_sql_identifier_rejects_invalid_identifier() {
        let err = validate_sql_identifier("user profiles", "table").expect_err("invalid id");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Invalid table identifier 'user profiles'"));
    }
}
