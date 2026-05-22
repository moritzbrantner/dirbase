use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::Value;

use crate::{
    app::AppState,
    error::AppError,
    http::{
        embed::{embed_collection_data, embed_lock_resources},
        overview,
    },
    mutation_service,
    query::filters::{
        filter_collection_refs, paginate_collection_refs, parse_collection_query_params,
        sort_collection_refs,
    },
    schema::primary_key_name,
    storage::{find_item_by_key, load_resource, validate_resource_data},
};

pub async fn list_resources(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let resources = state.resource_names_sorted().await;
    if overview::request_prefers_html(&headers) {
        return Ok(overview::render_root_overview(&state, &resources).await?.into_response());
    }
    Ok(Json(serde_json::json!({"resources": resources})).into_response())
}

pub async fn get_overview(
    State(state): State<AppState>,
) -> Result<Json<overview::OverviewPageData>, AppError> {
    overview::get_overview_json(&state).await
}

pub async fn get_collection(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Query(query_params): Query<Vec<(String, String)>>,
) -> Result<Json<Value>, AppError> {
    let parsed = parse_collection_query_params(query_params)?;
    enforce_per_page_limit(&state, parsed.pagination)?;
    let lock_resources = embed_lock_resources(&state, &resource, &parsed.embeds)?;
    let _guards = state.read_locks_for_resources(&lock_resources).await;

    let data = load_resource(&state, &resource).await?;
    validate_resource_data(&state, &resource, data.as_ref())?;
    if !data.is_array() {
        if !collection_query_operators_present(&parsed) {
            return Ok(Json(data.as_ref().clone()));
        }
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Filtering, sorting, pagination, and embedding require a JSON array resource",
        ));
    }
    let items = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;

    let mut selected = if parsed.filters.is_empty() {
        items.iter().collect::<Vec<_>>()
    } else {
        filter_collection_refs(items, &parsed.filters, None)
    };

    if !parsed.sort_columns.is_empty() {
        sort_collection_refs(selected.as_mut_slice(), &parsed.sort_columns);
    }

    let materialized = if let Some(pagination) = parsed.pagination {
        let mut paginated = paginate_collection_refs(&selected, pagination);
        if parsed.embeds.is_empty() {
            paginated
        } else {
            let data_field = paginated.get_mut("data").ok_or_else(|| {
                AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "Missing page data")
            })?;
            let embedded_page = embed_collection_data(
                &state,
                &resource,
                std::mem::replace(data_field, Value::Array(Vec::new())),
                &parsed.embeds,
            )
            .await?;
            *data_field = embedded_page;
            paginated
        }
    } else {
        let selected = Value::Array(selected.into_iter().cloned().collect());
        if parsed.embeds.is_empty() {
            selected
        } else {
            embed_collection_data(&state, &resource, selected, &parsed.embeds).await?
        }
    };

    Ok(Json(materialized))
}

fn enforce_per_page_limit(
    state: &AppState,
    pagination: Option<crate::query::filters::Pagination>,
) -> Result<(), AppError> {
    if let Some(pagination) = pagination
        && pagination.per_page > state.config.max_per_page
    {
        return Err(AppError::bad_request(format!(
            "per_page exceeds configured max of {}",
            state.config.max_per_page
        ))
        .with_code(crate::error::ERROR_CODE_LIMIT_EXCEEDED));
    }
    Ok(())
}

pub async fn create_item(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Json(payload): Json<Value>,
) -> Result<impl axum::response::IntoResponse, AppError> {
    let created = mutation_service::create_item(&state, &resource, payload).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

pub async fn get_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
) -> Result<Json<Value>, AppError> {
    let _guard = state.read_lock_for_resource(&resource).await;
    let data = load_resource(&state, &resource).await?;
    validate_resource_data(&state, &resource, data.as_ref())?;
    let array = data
        .as_array()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    let table = state.schema_table(&resource);
    let item_key = primary_key_name(table.as_ref());
    if let Some(position) = state
        .resource_cache
        .read()
        .await
        .get(&resource)
        .filter(|cached| cached.primary_key == item_key)
        .and_then(|cached| cached.id_index.as_ref())
        .and_then(|index| index.get(&id).copied())
    {
        return Ok(Json(
            array
                .get(position)
                .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?
                .clone(),
        ));
    }
    Ok(Json(
        find_item_by_key(array, item_key, &id)
            .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?
            .clone(),
    ))
}

pub async fn replace_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let replacement = mutation_service::replace_item(&state, &resource, &id, payload).await?;
    Ok(Json(replacement))
}

pub async fn patch_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let updated = mutation_service::patch_item(&state, &resource, &id, payload).await?;
    Ok(Json(updated))
}

pub async fn delete_item(
    State(state): State<AppState>,
    AxumPath((resource, id)): AxumPath<(String, String)>,
) -> Result<StatusCode, AppError> {
    mutation_service::delete_item(&state, &resource, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn replace_resource_object(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let data = mutation_service::replace_resource_object(&state, &resource, payload).await?;
    Ok(Json(data))
}

pub async fn patch_resource_object(
    State(state): State<AppState>,
    AxumPath(resource): AxumPath<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, AppError> {
    let updated = mutation_service::patch_resource_object(&state, &resource, payload).await?;
    Ok(Json(updated))
}

fn collection_query_operators_present(
    parsed: &crate::query::filters::ParsedCollectionQuery,
) -> bool {
    !parsed.filters.is_empty()
        || !parsed.sort_columns.is_empty()
        || parsed.pagination.is_some()
        || !parsed.embeds.is_empty()
}
