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
        )?;
    }

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
    )
}

fn validate_declared_columns<'a>(
    table: &DeclaredTableSchema,
    lookup: impl Fn(&str) -> Option<&'a Value>,
    missing_message: impl Fn(&str) -> String,
    null_message: impl Fn(&str) -> String,
    type_message: impl Fn(&str) -> String,
) -> Result<(), AppError> {
    for (column_name, column) in &table.columns {
        match lookup(column_name) {
            Some(Value::Null) if !column.nullable => {
                return Err(AppError::new(StatusCode::BAD_REQUEST, null_message(column_name)));
            }
            Some(value) if !value_matches_type(value, &column.column_type) => {
                return Err(AppError::new(StatusCode::BAD_REQUEST, type_message(column_name)));
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
            AppConfig, AppState, DataSource, GraphqlStore, HealthState, MetricsStore, SchemaStore,
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
        (name.to_string(), ColumnSchema { column_type, nullable })
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
