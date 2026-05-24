use std::{
    hash::{Hash, Hasher},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    body::Body,
    extract::State,
    http::{
        Method, Request, StatusCode,
        header::{
            ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
            ACCESS_CONTROL_ALLOW_ORIGIN, AUTHORIZATION, HeaderName, HeaderValue, ORIGIN,
            REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
        },
    },
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::{app::AppState, error::AppError};

pub async fn log_requests_middleware(
    State(_state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let query_hash = request_query_hash(request.uri().path(), request.uri().query());
    let response = next.run(request).await;
    let status = response.status();

    let timestamp =
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default();
    let suffix = query_hash.map(|hash| format!(" query_hash={hash}")).unwrap_or_default();
    let line = format!("{timestamp} {method} {path} {}{suffix}", status.as_u16());
    tracing::info!(target: "dirbase::request", "{line}");
    response
}

pub async fn metrics_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    state.metrics.record_request();
    let response = next.run(request).await;
    state.metrics.record_response(
        response.status().is_client_error() || response.status().is_server_error(),
    );
    response
}

pub async fn auth_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if request.method() == Method::OPTIONS {
        return next.run(request).await;
    }
    let path = request.uri().path();
    if path == "/healthz" || (matches!(path, "/readyz" | "/metrics") && !ops_auth_enabled(&state)) {
        return next.run(request).await;
    }
    let Some(expected) = state.config.auth_token.as_deref() else {
        return next.run(request).await;
    };
    let authorized = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| token == expected);
    if authorized {
        return next.run(request).await;
    }
    state.metrics.record_auth_failure();
    AppError::unauthorized("Missing or invalid bearer token")
        .with_code(crate::error::ERROR_CODE_UNAUTHORIZED)
        .into_response()
}

fn ops_auth_enabled(state: &AppState) -> bool {
    state.config.protect_ops && state.config.auth_token.is_some()
}

pub async fn security_headers_middleware(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(X_FRAME_OPTIONS, HeaderValue::from_static("sameorigin"));
    headers.insert(
        HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    response
}

pub async fn cors_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let origin =
        request.headers().get(ORIGIN).and_then(|value| value.to_str().ok()).map(str::to_string);
    let allow_origin = state
        .config
        .cors_origin
        .as_deref()
        .zip(origin.as_deref())
        .and_then(|(expected, actual)| (expected == actual).then_some(actual.to_string()));

    if request.method() == Method::OPTIONS {
        let mut response = StatusCode::NO_CONTENT.into_response();
        if let Some(origin) = allow_origin {
            let headers = response.headers_mut();
            headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin.parse().expect("valid origin"));
            headers.insert(
                ACCESS_CONTROL_ALLOW_METHODS,
                "GET,POST,PUT,PATCH,DELETE,OPTIONS".parse().expect("allow methods"),
            );
            headers.insert(
                ACCESS_CONTROL_ALLOW_HEADERS,
                "content-type,authorization".parse().expect("allow headers"),
            );
        }
        return response;
    }

    let mut response = next.run(request).await;
    if let Some(origin) = allow_origin {
        let headers = response.headers_mut();
        headers.insert(ACCESS_CONTROL_ALLOW_ORIGIN, origin.parse().expect("valid origin"));
        headers.insert(
            ACCESS_CONTROL_ALLOW_METHODS,
            "GET,POST,PUT,PATCH,DELETE,OPTIONS".parse().expect("allow methods"),
        );
        headers.insert(
            ACCESS_CONTROL_ALLOW_HEADERS,
            "content-type,authorization".parse().expect("allow headers"),
        );
    }
    response
}

fn request_query_hash(path: &str, query: Option<&str>) -> Option<String> {
    if path != "/sql" && path != "/export.sql" {
        return None;
    }
    let query = query?;
    let sql = query
        .split('&')
        .find_map(|pair| pair.split_once('=').and_then(|(k, v)| (k == "q").then_some(v)))?;
    Some(stable_hash(sql))
}

fn stable_hash(value: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
