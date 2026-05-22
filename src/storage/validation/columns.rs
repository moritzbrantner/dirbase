use axum::http::StatusCode;
use serde_json::Value;

use crate::{error::AppError, schema::DeclaredTableSchema};

use super::{constraints::validate_column_constraints, types::value_matches_type};

pub(super) fn validate_declared_columns<'a>(
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
