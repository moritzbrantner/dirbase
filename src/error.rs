use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

pub(crate) const ERROR_CODE_INVALID_SQL: &str = "invalid_sql";
pub(crate) const ERROR_CODE_LIMIT_EXCEEDED: &str = "limit_exceeded";
pub(crate) const ERROR_CODE_UNAUTHORIZED: &str = "unauthorized";
pub(crate) const ERROR_CODE_UNKNOWN_TABLE: &str = "unknown_table";
pub(crate) const ERROR_CODE_UNSUPPORTED_FEATURE: &str = "unsupported_feature";

#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub message: String,
    pub code: Option<&'static str>,
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'static str>,
}

impl AppError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self { status, message: message.into(), code: None }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }

    pub fn payload_too_large(message: impl Into<String>) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, message)
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, message)
    }

    pub fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.status, Json(ErrorBody { error: self.message, code: self.code })).into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::{body::to_bytes, response::IntoResponse};

    use super::*;

    #[tokio::test]
    async fn app_error_serializes_error_body_and_optional_code() {
        let response = AppError::bad_request("invalid request").into_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.expect("body bytes");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json body");
        assert_eq!(payload, serde_json::json!({"error": "invalid request"}));

        let response = AppError::payload_too_large("too large")
            .with_code(ERROR_CODE_LIMIT_EXCEEDED)
            .into_response();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = to_bytes(response.into_body(), usize::MAX).await.expect("body bytes");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("json body");
        assert_eq!(payload, serde_json::json!({"error": "too large", "code": "limit_exceeded"}));
    }
}
