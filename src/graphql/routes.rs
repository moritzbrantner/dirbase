use async_graphql::{
    Request as GraphqlRequest,
    http::{GraphiQLSource, parse_query_string},
};
use async_graphql_axum::GraphQLResponse;
use axum::{
    Json,
    extract::State,
    http::{
        HeaderMap, StatusCode, Uri,
        header::{ACCEPT, CONTENT_TYPE},
    },
    response::{Html, IntoResponse, Response},
};
use serde_json::json;

use crate::app::AppState;

use super::types::GraphqlRelationCache;

pub async fn graphql_get(State(state): State<AppState>, headers: HeaderMap, uri: Uri) -> Response {
    let raw_query = uri.query().unwrap_or_default();
    if raw_query.len() > state.config.max_query_bytes {
        return graphql_error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "GraphQL query exceeds configured max of {} bytes",
                state.config.max_query_bytes
            ),
        );
    }
    if raw_query.is_empty() && request_prefers_html(&headers) {
        return Html(GraphiQLSource::build().endpoint("/graphql").finish()).into_response();
    }

    let request = match parse_query_string(raw_query) {
        Ok(request) if !request.query.trim().is_empty() => request,
        Ok(_) => return graphql_error_response(StatusCode::BAD_REQUEST, "Missing GraphQL query"),
        Err(err) => return graphql_error_response(StatusCode::BAD_REQUEST, err.to_string()),
    };

    execute_graphql_request(&state, request).await
}

pub async fn graphql_post(
    State(state): State<AppState>,
    Json(request): Json<GraphqlRequest>,
) -> Response {
    if request.query.len() > state.config.max_query_bytes {
        return graphql_error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "GraphQL query exceeds configured max of {} bytes",
                state.config.max_query_bytes
            ),
        );
    }
    execute_graphql_request(&state, request).await
}
pub(crate) fn request_prefers_html(headers: &HeaderMap) -> bool {
    headers
        .get(ACCEPT)
        .and_then(|value| value.to_str().ok())
        .map(|accept| {
            accept.split(',').map(str::trim).any(|value| {
                value.starts_with("text/html") || value.starts_with("application/xhtml+xml")
            })
        })
        .unwrap_or(false)
}

pub(crate) async fn execute_graphql_request(state: &AppState, request: GraphqlRequest) -> Response {
    let schema = match state.graphql_schema().await {
        Ok(schema) => schema,
        Err(error) => {
            return graphql_error_response(StatusCode::INTERNAL_SERVER_ERROR, error);
        }
    };

    let request = request.data(GraphqlRelationCache::default());
    GraphQLResponse::from(schema.execute(request).await).into_response()
}

pub(crate) fn graphql_error_response(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        [(CONTENT_TYPE, "application/graphql-response+json")],
        Json(json!({
            "errors": [{"message": message.into()}]
        })),
    )
        .into_response()
}
