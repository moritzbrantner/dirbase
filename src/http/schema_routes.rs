use axum::{Json, extract::State, http::StatusCode};
use serde::Serialize;
use serde_json::Value;

use crate::{
    app::AppState,
    error::AppError,
    schema::{
        DeclaredSchema, default_schema_output_path, export_declared_schema_snapshot,
        infer_schema_from_data_source,
    },
};

pub async fn get_schema(State(state): State<AppState>) -> Json<crate::schema::Schema> {
    Json(state.schema_snapshot())
}
#[derive(Serialize)]
pub struct SchemaEditorPayload {
    pub inferred: crate::schema::Schema,
    pub declared: Option<DeclaredSchema>,
    pub effective: crate::schema::Schema,
    pub save_path: String,
}

pub async fn get_schema_editor(State(state): State<AppState>) -> Json<SchemaEditorPayload> {
    let save_path = default_schema_output_path(&state.data_source);
    Json(SchemaEditorPayload {
        inferred: state.inferred_schema_snapshot(),
        declared: state.declared_schema_snapshot(),
        effective: state.schema_snapshot(),
        save_path: save_path.display().to_string(),
    })
}

pub async fn save_schema(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let schema = state.schema_snapshot();
    let declared_snapshot =
        export_declared_schema_snapshot(state.declared_schema_snapshot().as_ref(), &schema);
    let path = default_schema_output_path(&state.data_source);
    let path_for_write = path.clone();
    tokio::task::spawn_blocking(move || {
        let payload = serde_json::to_string_pretty(&declared_snapshot)
            .map_err(|err| format!("{}: {err}", path_for_write.display()))?;
        std::fs::write(&path_for_write, format!("{payload}\n"))
            .map_err(|err| format!("{}: {err}", path_for_write.display()))
    })
    .await
    .map_err(|err| {
        AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Schema save task failed: {err}"))
    })?
    .map_err(|err| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, err))?;

    state.emit_event("schema_changed", None);

    Ok(Json(serde_json::json!({
        "saved": true,
        "path": path.display().to_string(),
    })))
}

pub async fn save_declared_schema(
    State(state): State<AppState>,
    Json(declared): Json<DeclaredSchema>,
) -> Result<Json<Value>, AppError> {
    let inferred = state.schema_store.read().expect("schema store").inferred.clone();
    crate::schema::merge_schemas(Some(&declared), &inferred)
        .map_err(|err| AppError::new(StatusCode::BAD_REQUEST, err))?;

    let path = default_schema_output_path(&state.data_source);
    let path_for_write = path.clone();
    let declared_for_write = declared.clone();
    tokio::task::spawn_blocking(move || {
        let payload = serde_json::to_string_pretty(&declared_for_write)
            .map_err(|err| format!("{}: {err}", path_for_write.display()))?;
        std::fs::write(&path_for_write, format!("{payload}\n"))
            .map_err(|err| format!("{}: {err}", path_for_write.display()))
    })
    .await
    .map_err(|err| {
        AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Schema save task failed: {err}"))
    })?
    .map_err(|err| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, err))?;

    state
        .update_declared_schema(Some(declared))
        .map_err(|err| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, err))?;
    state.invalidate_graphql_schema().await;
    state.emit_event("schema_changed", None);
    state.emit_event("overview_changed", None);
    state.health.mark_ready();

    Ok(Json(serde_json::json!({
        "saved": true,
        "declared": true,
        "path": path.display().to_string(),
    })))
}

pub async fn infer_and_save_schema(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    let resources = state.resources.read().await.clone();
    let data_source = state.data_source.clone();
    let inferred = tokio::task::spawn_blocking(move || {
        infer_schema_from_data_source(&data_source, &resources)
    })
    .await
    .map_err(|err| {
        AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Schema infer task failed: {err}"))
    })?
    .map_err(|err| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, err))?;

    let declared_snapshot = export_declared_schema_snapshot(None, &inferred);
    let path = default_schema_output_path(&state.data_source);
    let path_for_write = path.clone();
    let declared_for_write = declared_snapshot.clone();
    tokio::task::spawn_blocking(move || {
        let payload = serde_json::to_string_pretty(&declared_for_write)
            .map_err(|err| format!("{}: {err}", path_for_write.display()))?;
        std::fs::write(&path_for_write, format!("{payload}\n"))
            .map_err(|err| format!("{}: {err}", path_for_write.display()))
    })
    .await
    .map_err(|err| {
        AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Schema save task failed: {err}"))
    })?
    .map_err(|err| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, err))?;

    state
        .update_inferred_schema(inferred)
        .map_err(|err| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, err))?;
    state
        .update_declared_schema(Some(declared_snapshot))
        .map_err(|err| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, err))?;
    state.invalidate_graphql_schema().await;
    state.emit_event("schema_changed", None);
    state.emit_event("overview_changed", None);
    state.health.mark_ready();

    Ok(Json(serde_json::json!({
        "saved": true,
        "inferred": true,
        "path": path.display().to_string(),
    })))
}
