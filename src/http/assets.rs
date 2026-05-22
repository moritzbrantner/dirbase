use axum::response::Response;

use crate::http::overview;

pub async fn get_overview_css() -> Response {
    overview::overview_css()
}

pub async fn get_overview_js() -> Response {
    overview::overview_js()
}
