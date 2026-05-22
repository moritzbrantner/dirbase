use axum::{
    Json,
    extract::{Query, State},
    http::header::CONTENT_TYPE,
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::Value;

use crate::{app::AppState, error::AppError};

use super::{executor::run_sql_query, export::build_sql_export, types::SqlExportDialect};

#[derive(Deserialize)]
pub struct SqlGetParams {
    pub q: String,
}
#[derive(Deserialize)]
pub struct SqlPostBody {
    pub query: String,
}
#[derive(Deserialize)]
pub struct SqlExportParams {
    pub dialect: Option<String>,
}

pub async fn sql_query(
    State(state): State<AppState>,
    Query(params): Query<SqlGetParams>,
) -> Result<Json<Value>, AppError> {
    run_sql_query(state, params.q).await
}
pub async fn sql_query_post(
    State(state): State<AppState>,
    Json(payload): Json<SqlPostBody>,
) -> Result<Json<Value>, AppError> {
    run_sql_query(state, payload.query).await
}

pub async fn export_sql(
    State(state): State<AppState>,
    Query(params): Query<SqlExportParams>,
) -> Result<impl IntoResponse, AppError> {
    let dialect = SqlExportDialect::parse(params.dialect.as_deref())?;
    let resource_names = state.resource_names_sorted().await;
    let _guards = state.read_locks_for_resources(&resource_names).await;
    let sql = build_sql_export(&state, dialect).await?;
    Ok(([(CONTENT_TYPE, "text/sql; charset=utf-8")], sql))
}
