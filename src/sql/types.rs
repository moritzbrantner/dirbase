use axum::http::StatusCode;

use crate::{
    error::AppError,
    query::filters::{FilterCondition, Pagination, SortColumn},
    schema::ColumnType,
};

pub(crate) const MAX_SQL_QUERY_LENGTH: usize = 16_384;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SqlExportDialect {
    Postgres,
    Sqlite,
}

impl SqlExportDialect {
    pub(crate) fn parse(value: Option<&str>) -> Result<Self, AppError> {
        match value.unwrap_or("postgres").to_ascii_lowercase().as_str() {
            "postgres" | "postgresql" => Ok(Self::Postgres),
            "sqlite" => Ok(Self::Sqlite),
            other => Err(AppError::new(
                StatusCode::BAD_REQUEST,
                format!("Unsupported SQL dialect '{other}'. Expected 'postgres' or 'sqlite'"),
            )),
        }
    }
    pub(crate) fn type_name(self, column_type: &ColumnType) -> &'static str {
        match (self, column_type) {
            (_, ColumnType::Integer) => "INTEGER",
            (Self::Postgres, ColumnType::BigInteger) => "BIGINT",
            (Self::Sqlite, ColumnType::BigInteger) => "INTEGER",
            (_, ColumnType::Float) => "REAL",
            (Self::Postgres, ColumnType::Decimal) => "NUMERIC",
            (Self::Sqlite, ColumnType::Decimal) => "TEXT",
            (_, ColumnType::Boolean) => "BOOLEAN",
            (Self::Sqlite, ColumnType::Json) => "TEXT",
            (Self::Postgres, ColumnType::Json) => "JSONB",
            (Self::Postgres, ColumnType::Date) => "DATE",
            (Self::Postgres, ColumnType::DateTime) => "TIMESTAMPTZ",
            (Self::Postgres, ColumnType::Uuid) => "UUID",
            (
                _,
                ColumnType::String | ColumnType::Date | ColumnType::DateTime | ColumnType::Uuid,
            ) => "TEXT",
        }
    }
}

#[derive(Debug)]
pub(crate) struct ParsedSqlQuery {
    pub(crate) resource: String,
    pub(crate) resource_alias: String,
    pub(crate) selected_columns: Option<Vec<ParsedSqlProjection>>,
    pub(crate) filters: Vec<FilterCondition>,
    pub(crate) sort_columns: Vec<SortColumn>,
    pub(crate) pagination: Option<Pagination>,
    pub(crate) joins: Vec<ParsedSqlJoin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedSqlProjection {
    pub(crate) source: String,
    pub(crate) output: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedSqlJoin {
    pub(crate) resource: String,
    pub(crate) alias: String,
    pub(crate) left_alias: String,
    pub(crate) left_column: String,
    pub(crate) right_column: String,
}
