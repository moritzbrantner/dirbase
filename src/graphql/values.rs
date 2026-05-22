use async_graphql::{
    Error as GraphqlError, Value as GraphqlValue,
    dynamic::{FieldValue, TypeRef},
};
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::{
    error::AppError,
    schema::{ColumnSchema, ColumnType},
};

use super::types::{GraphqlObjectValue, GraphqlPageValue, ScalarKind};

pub(crate) fn parent_object_value<'a>(
    ctx: &'a async_graphql::dynamic::ResolverContext<'a>,
) -> Result<&'a GraphqlObjectValue, GraphqlError> {
    ctx.parent_value
        .try_downcast_ref::<GraphqlObjectValue>()
        .map_err(|err| GraphqlError::new(err.message))
}

pub(crate) fn parent_page_value<'a>(
    ctx: &'a async_graphql::dynamic::ResolverContext<'a>,
) -> Result<&'a GraphqlPageValue, GraphqlError> {
    ctx.parent_value
        .try_downcast_ref::<GraphqlPageValue>()
        .map_err(|err| GraphqlError::new(err.message))
}

pub(crate) fn typed_object_value(
    _type_name: &str,
    object: JsonMap<String, JsonValue>,
) -> FieldValue<'static> {
    FieldValue::owned_any(GraphqlObjectValue { object })
}

pub(crate) fn json_to_graphql_value(value: JsonValue) -> Result<GraphqlValue, GraphqlError> {
    GraphqlValue::from_json(value)
        .map_err(|err| GraphqlError::new(format!("Failed to convert JSON value: {err}")))
}

pub(crate) fn graphql_argument_to_lookup_string(
    value: &GraphqlValue,
) -> Result<String, GraphqlError> {
    match value {
        GraphqlValue::String(text) => Ok(text.clone()),
        GraphqlValue::Number(number) => Ok(number.to_string()),
        GraphqlValue::Boolean(value) => Ok(value.to_string()),
        _ => Err(GraphqlError::new("GraphQL id argument must be a scalar value")),
    }
}

pub(crate) fn app_error_to_graphql(error: AppError) -> GraphqlError {
    GraphqlError::new(error.message)
}

pub(crate) fn scalar_kind_from_column(column: &ColumnSchema) -> ScalarKind {
    match column.column_type {
        ColumnType::Integer => ScalarKind::Int,
        ColumnType::Float => ScalarKind::Float,
        ColumnType::Boolean => ScalarKind::Boolean,
        ColumnType::String
        | ColumnType::Date
        | ColumnType::DateTime
        | ColumnType::Uuid
        | ColumnType::BigInteger
        | ColumnType::Decimal => ScalarKind::String,
        ColumnType::Json => ScalarKind::Json,
    }
}

pub(crate) fn scalar_kind_from_json_value(value: &JsonValue) -> ScalarKind {
    if value.is_i64() || value.is_u64() {
        return ScalarKind::Int;
    }
    if value.is_number() {
        return ScalarKind::Float;
    }
    if value.is_boolean() {
        return ScalarKind::Boolean;
    }
    if value.is_string() {
        return ScalarKind::String;
    }
    ScalarKind::Json
}

pub(crate) fn scalar_type_ref(kind: ScalarKind, nullable: bool) -> TypeRef {
    let type_name = match kind {
        ScalarKind::Int => TypeRef::INT,
        ScalarKind::Float => TypeRef::FLOAT,
        ScalarKind::Boolean => TypeRef::BOOLEAN,
        ScalarKind::String => TypeRef::STRING,
        ScalarKind::Json => "JSON",
    };
    named_type_ref(type_name, nullable)
}

pub(crate) fn named_type_ref(type_name: &str, nullable: bool) -> TypeRef {
    if nullable { TypeRef::named(type_name) } else { TypeRef::named_nn(type_name) }
}
