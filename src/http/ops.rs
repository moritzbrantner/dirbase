use std::convert::Infallible;

use axum::{
    Json,
    extract::State,
    http::{StatusCode, header::CONTENT_TYPE},
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use serde_json::Value;
use tokio_stream::{StreamExt, wrappers::BroadcastStream};

use crate::app::AppState;

pub async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        state.metrics.render_prometheus(),
    )
}

pub async fn healthz(State(state): State<AppState>) -> Json<Value> {
    Json(serde_json::json!({
        "ok": true,
        "ready": state.health.is_ready(),
        "last_error": state.health.last_error(),
    }))
}

pub async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    let status =
        if state.health.is_ready() { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (
        status,
        Json(serde_json::json!({
            "ready": state.health.is_ready(),
            "last_error": state.health.last_error(),
        })),
    )
}

pub async fn get_events(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let stream =
        BroadcastStream::new(state.subscribe_events()).filter_map(|message| match message {
            Ok(event) => {
                let payload = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
                Some(Ok(Event::default().event(event.kind).data(payload)))
            }
            Err(_) => None,
        });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
