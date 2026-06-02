use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{
        HeaderMap, HeaderName, Method, StatusCode, Uri,
        header::{
            ACCEPT, ACCEPT_LANGUAGE, CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, HOST, USER_AGENT,
        },
    },
    response::{IntoResponse, Response},
};
use serde_json::Value;

use crate::{
    app::AppState,
    error::AppError,
    resource_service,
    storage::{
        coerce_id_value, find_item_index_by_key, is_valid_resource_name, load_resource,
        resource_exists, validate_resource_data, write_resource,
    },
};

const REFRESH_QUERY: &str = "_refresh=true";

pub(crate) fn is_enabled(state: &AppState) -> bool {
    state.config.clone_proxy.is_some()
}

pub(crate) fn is_refresh(uri: &Uri) -> bool {
    uri.query() == Some(REFRESH_QUERY)
}

pub(crate) fn has_non_refresh_query(uri: &Uri) -> bool {
    uri.query().is_some_and(|query| query != REFRESH_QUERY)
}

pub(crate) async fn collection_should_fetch(
    state: &AppState,
    resource: &str,
    uri: &Uri,
) -> Result<bool, AppError> {
    if !is_enabled(state) {
        return Ok(false);
    }
    if is_refresh(uri) {
        return Ok(true);
    }
    if has_non_refresh_query(uri) {
        return Ok(false);
    }
    Ok(!resource_exists(state, resource).await?)
}

pub(crate) async fn proxy_resource_request(
    State(state): State<AppState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, AppError> {
    proxy_request(&state, method, &uri, &headers, Some(body)).await
}

pub(crate) async fn proxy_get(
    state: &AppState,
    uri: &Uri,
    headers: &HeaderMap,
) -> Result<Response, AppError> {
    proxy_request(state, Method::GET, uri, headers, None).await
}

pub(crate) async fn fetch_collection_and_cache(
    state: &AppState,
    resource: &str,
    uri: &Uri,
    headers: &HeaderMap,
) -> Result<Response, AppError> {
    validate_cacheable_resource_name(resource)?;
    let remote = send_remote_request(state, Method::GET, uri, headers, None).await?;
    if !remote.status.is_success() {
        return Ok(remote.into_response());
    }
    let Ok(value) = serde_json::from_slice::<Value>(&remote.body) else {
        return Ok(remote.into_response());
    };
    let Some(array) = value.as_array() else {
        tracing::warn!(resource, "Clone collection response was not a JSON array; skipping cache");
        return Ok(remote.into_response());
    };

    let cached = Value::Array(array.clone());
    cache_collection(state, resource, cached).await;
    Ok(remote.into_response())
}

pub(crate) async fn fetch_item_and_cache(
    state: &AppState,
    resource: &str,
    id: &str,
    uri: &Uri,
    headers: &HeaderMap,
) -> Result<Response, AppError> {
    validate_cacheable_resource_name(resource)?;
    let remote = send_remote_request(state, Method::GET, uri, headers, None).await?;
    if !remote.status.is_success() {
        return Ok(remote.into_response());
    }
    let Ok(mut value) = serde_json::from_slice::<Value>(&remote.body) else {
        return Ok(remote.into_response());
    };
    let Some(object) = value.as_object_mut() else {
        tracing::warn!(resource, id, "Clone item response was not a JSON object; skipping cache");
        return Ok(remote.into_response());
    };
    object.insert("id".to_string(), coerce_id_value(id, None));

    cache_item(state, resource, id, Value::Object(object.clone())).await;
    Ok(remote.into_response())
}

async fn cache_collection(state: &AppState, resource: &str, value: Value) {
    if let Err(err) = validate_resource_data(state, resource, &value) {
        tracing::warn!(
            resource,
            error = %err.message,
            "Clone collection failed local schema validation; skipping cache"
        );
        return;
    }

    let result = if state.resources.read().await.contains(resource) {
        let _guard = state.write_lock_for_resource(resource).await;
        write_resource(state, resource, &value).await
    } else {
        resource_service::create_resource(state, resource, value).await.map(|_| ())
    };
    if let Err(err) = result {
        tracing::warn!(resource, error = %err.message, "Failed to persist cloned collection");
    }
}

async fn cache_item(state: &AppState, resource: &str, id: &str, item: Value) {
    if !state.resources.read().await.contains(resource) {
        let value = Value::Array(vec![item]);
        if let Err(err) = validate_resource_data(state, resource, &value) {
            tracing::warn!(
                resource,
                id,
                error = %err.message,
                "Clone item failed local schema validation; skipping cache"
            );
            return;
        }
        if let Err(err) = resource_service::create_resource(state, resource, value).await {
            tracing::warn!(resource, id, error = %err.message, "Failed to persist cloned item");
        }
        return;
    }

    let _guard = state.write_lock_for_resource(resource).await;
    let value = match load_resource(state, resource).await {
        Ok(current) => {
            let mut data = current.as_ref().clone();
            let Some(array) = data.as_array_mut() else {
                tracing::warn!(resource, "Local resource is not a JSON array; skipping item cache");
                return;
            };
            if let Some(index) = find_item_index_by_key(array, "id", id) {
                array[index] = item;
            } else {
                array.push(item);
            }
            data
        }
        Err(err) => {
            tracing::warn!(resource, error = %err.message, "Failed to load local resource for item cache");
            return;
        }
    };

    if let Err(err) = validate_resource_data(state, resource, &value) {
        tracing::warn!(
            resource,
            id,
            error = %err.message,
            "Clone item failed local schema validation; skipping cache"
        );
        return;
    }

    if let Err(err) = write_resource(state, resource, &value).await {
        tracing::warn!(resource, id, error = %err.message, "Failed to persist cloned item");
    }
}

async fn proxy_request(
    state: &AppState,
    method: Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Option<Bytes>,
) -> Result<Response, AppError> {
    let remote = send_remote_request(state, method, uri, headers, body).await?;
    Ok(remote.into_response())
}

async fn send_remote_request(
    state: &AppState,
    method: Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Option<Bytes>,
) -> Result<RemoteResponse, AppError> {
    let config = state
        .config
        .clone_proxy
        .as_ref()
        .ok_or_else(|| AppError::internal("Clone proxy is not configured"))?;
    let remote_url = remote_url(&config.base_url, uri);
    let mut request = config.client.request(method, remote_url);
    for (name, value) in forwarded_headers(headers) {
        request = request.header(name, value);
    }
    for (name, value) in &config.headers {
        request = request.header(
            name.as_str(),
            value.to_str().map_err(|err| {
                AppError::internal(format!("Configured clone header is not valid UTF-8: {err}"))
            })?,
        );
    }
    if let Some(body) = body {
        request = request.body(body);
    }

    let response = request
        .send()
        .await
        .map_err(|err| AppError::internal(format!("Clone proxy request failed: {err}")))?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .bytes()
        .await
        .map_err(|err| AppError::internal(format!("Clone proxy response failed: {err}")))?;
    Ok(RemoteResponse { status, headers, body })
}

fn remote_url(base_url: &reqwest::Url, uri: &Uri) -> reqwest::Url {
    let mut remote_url = base_url.clone();
    let base_path = base_url.path().trim_end_matches('/');
    let request_path = uri.path().trim_start_matches('/');
    let combined_path = if base_path.is_empty() {
        format!("/{request_path}")
    } else if request_path.is_empty() {
        base_path.to_string()
    } else {
        format!("{base_path}/{request_path}")
    };
    remote_url.set_path(&combined_path);
    remote_url.set_query(uri.query().filter(|query| *query != REFRESH_QUERY));
    remote_url
}

fn forwarded_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    [
        ACCEPT,
        ACCEPT_LANGUAGE,
        CONTENT_TYPE,
        USER_AGENT,
        HeaderName::from_static("x-request-id"),
        HeaderName::from_static("x-correlation-id"),
    ]
    .into_iter()
    .filter_map(|name| {
        headers
            .get(&name)
            .and_then(|value| value.to_str().ok())
            .map(|value| (name.as_str().to_string(), value.to_string()))
    })
    .collect()
}

fn validate_cacheable_resource_name(resource: &str) -> Result<(), AppError> {
    if !is_valid_resource_name(resource) {
        return Err(AppError::bad_request(
            "Resource name must only contain letters, numbers, underscore, and dash",
        ));
    }
    Ok(())
}

struct RemoteResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

impl RemoteResponse {
    fn into_response(self) -> Response {
        let mut response = Response::builder().status(self.status);
        if let Some(headers) = response.headers_mut() {
            for (name, value) in self.headers {
                let Some(name) = name else {
                    continue;
                };
                if should_copy_response_header(&name) {
                    headers.insert(name, value);
                }
            }
            headers.remove(CONTENT_LENGTH);
        }
        response.body(Body::from(self.body)).unwrap_or_else(|err| {
            AppError::internal(format!("Failed to build clone proxy response: {err}"))
                .into_response()
        })
    }
}

fn should_copy_response_header(name: &HeaderName) -> bool {
    !matches!(*name, HOST | CONNECTION | CONTENT_LENGTH) && !name.as_str().starts_with("proxy-")
}
