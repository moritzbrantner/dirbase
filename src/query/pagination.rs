use axum::http::StatusCode;
use serde_json::Value;

use crate::error::AppError;

use super::types::{Pagination, PaginationWindow};

pub fn paginate_collection_data(data: Value, pagination: Pagination) -> Result<Value, AppError> {
    let items = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    let window = pagination_window(items.len(), pagination);
    let data = if window.start < window.items {
        items[window.start..window.end].to_vec()
    } else {
        Vec::new()
    };

    Ok(serde_json::json!({
        "first": window.first,
        "prev": window.prev,
        "next": window.next,
        "last": window.last,
        "page": window.page,
        "pages": window.pages,
        "items": window.items,
        "data": data,
    }))
}

pub fn pagination_window(total_items: usize, pagination: Pagination) -> PaginationWindow {
    let pages = if total_items == 0 { 1 } else { total_items.div_ceil(pagination.per_page) };
    let page = pagination.page.max(1).min(pages.max(1));
    let start = (page - 1) * pagination.per_page;
    let end = (start + pagination.per_page).min(total_items);

    PaginationWindow {
        first: 1,
        prev: if page > 1 { Some(page - 1) } else { None },
        next: if page < pages { Some(page + 1) } else { None },
        last: pages,
        page,
        pages,
        items: total_items,
        start,
        end,
    }
}

pub fn paginate_collection_refs(items: &[&Value], pagination: Pagination) -> Value {
    let window = pagination_window(items.len(), pagination);
    let data = if window.start < window.items {
        items[window.start..window.end].iter().map(|item| (*item).clone()).collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    serde_json::json!({
        "first": window.first,
        "prev": window.prev,
        "next": window.next,
        "last": window.last,
        "page": window.page,
        "pages": window.pages,
        "items": window.items,
        "data": data,
    })
}
