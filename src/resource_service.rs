use axum::http::StatusCode;
use serde_json::Value;

use crate::{
    app::AppState,
    error::AppError,
    storage::{
        create_resource_value, delete_resource_value, is_reserved_resource_name,
        is_valid_resource_name, refresh_inferred_schema, remove_cached_resource,
        resource_file_path, update_cached_resource, validate_resource_data,
    },
};

pub async fn create_resource(
    state: &AppState,
    resource: &str,
    initial: Value,
) -> Result<Value, AppError> {
    validate_resource_name_for_management(resource)?;
    validate_resource_data(state, resource, &initial)?;

    let _guard = state.write_lock_for_resource(resource).await;
    if state.resources.read().await.contains(resource) {
        return Err(AppError::new(StatusCode::CONFLICT, "Resource already exists"));
    }

    let file = resource_file_path(&state.data_source, resource)?;
    create_resource_value(&state.data_source, &file, resource, &initial).await?;

    state.resources.write().await.insert(resource.to_string());
    update_cached_resource(state, resource, std::sync::Arc::new(initial.clone())).await;
    refresh_after_resource_set_change(state).await?;
    state.emit_event("resource_changed", Some(resource.to_string()));

    Ok(initial)
}

pub async fn delete_resource(state: &AppState, resource: &str) -> Result<(), AppError> {
    validate_resource_name_for_management(resource)?;

    let _guard = state.write_lock_for_resource(resource).await;
    if !state.resources.read().await.contains(resource) {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            format!("Resource '{resource}' not found"),
        ));
    }

    let file = resource_file_path(&state.data_source, resource)?;
    delete_resource_value(&state.data_source, &file, resource).await?;

    state.resources.write().await.remove(resource);
    remove_cached_resource(state, resource).await;
    refresh_after_resource_set_change(state).await?;
    state.emit_event("resource_changed", Some(resource.to_string()));

    Ok(())
}

fn validate_resource_name_for_management(resource: &str) -> Result<(), AppError> {
    if !is_valid_resource_name(resource) {
        return Err(AppError::bad_request(
            "Resource name must only contain letters, numbers, underscore, and dash",
        ));
    }
    if is_reserved_resource_name(resource) {
        return Err(AppError::bad_request(format!(
            "Resource name '{resource}' is reserved for dirbase"
        )));
    }
    Ok(())
}

async fn refresh_after_resource_set_change(state: &AppState) -> Result<(), AppError> {
    refresh_inferred_schema(state).await?;
    state.invalidate_graphql_schema().await;
    state.emit_event("schema_changed", None);
    state.emit_event("overview_changed", None);
    state.health.mark_ready();
    Ok(())
}
