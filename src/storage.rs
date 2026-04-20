mod index;
mod io;
mod validation;

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::Value;

use crate::{
    app::{AppState, CachedResource, DataSource},
    error::AppError,
    schema::{infer_schema_from_data_source, primary_key_name},
};

pub use index::{
    build_id_index, coerce_id_value, find_item_by_key, find_item_index_by_key, next_numeric_id,
};
pub use io::{
    is_reserved_resource_name, is_valid_resource_name, resource_file_path, scan_resources,
};
pub use validation::{validate_resource_data, validate_sql_identifier};

pub async fn load_resource(state: &AppState, resource: &str) -> Result<Arc<Value>, AppError> {
    let file = resource_file_path(&state.data_source, resource)?;

    if let Some(value) =
        state.resource_cache.read().await.get(resource).map(|cached| cached.value.clone())
    {
        return Ok(value);
    }

    if !state.resources.read().await.contains(resource) {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            format!("Resource '{resource}' not found"),
        ));
    }

    let value = Arc::new(io::read_resource_value(&state.data_source, &file, resource).await?);

    update_cached_resource(state, resource, value.clone()).await;
    Ok(value)
}

pub async fn write_resource(
    state: &AppState,
    resource: &str,
    value: &Value,
) -> Result<(), AppError> {
    let file = resource_file_path(&state.data_source, resource)?;

    if matches!(state.data_source.as_ref(), DataSource::File(_)) {
        let _guard = state.write_lock_for_resource("__db_file__").await;
        io::persist_resource_value(&state.data_source, &file, resource, value).await?;
    } else {
        io::persist_resource_value(&state.data_source, &file, resource, value).await?;
    }

    update_cached_resource(state, resource, Arc::new(value.clone())).await;
    refresh_inferred_schema(state).await?;
    state.invalidate_graphql_schema().await;
    state.emit_event("resource_changed", Some(resource.to_string()));
    state.emit_event("schema_changed", None);
    state.emit_event("overview_changed", None);
    state.health.mark_ready();
    Ok(())
}

pub async fn resource_exists(state: &AppState, resource: &str) -> Result<bool, AppError> {
    Ok(state.resources.read().await.contains(resource))
}

async fn update_cached_resource(state: &AppState, resource: &str, value: Arc<Value>) {
    let table = state.schema_table(resource);
    state.resource_cache.write().await.insert(
        resource.to_string(),
        CachedResource {
            value: value.clone(),
            id_index: build_id_index(value.as_ref(), table.as_ref()),
            primary_key: primary_key_name(table.as_ref()).to_string(),
        },
    );
}

async fn refresh_inferred_schema(state: &AppState) -> Result<(), AppError> {
    let resources = state.resources.read().await.clone();
    let data_source = state.data_source.clone();
    let inferred = tokio::task::spawn_blocking(move || {
        infer_schema_from_data_source(&data_source, &resources)
    })
    .await
    .map_err(|err| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Schema refresh task failed: {err}"),
        )
    })?
    .map_err(|err| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, err))?;
    state
        .update_inferred_schema(inferred)
        .map_err(|err| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, err))?;
    state.health.mark_ready();
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeSet, HashMap},
        path::PathBuf,
        sync::Arc,
    };

    use tokio::sync::RwLock;

    use super::*;
    use crate::app::{AppState, DataSource};

    fn test_state(data_source: DataSource) -> AppState {
        AppState {
            data_source: Arc::new(data_source),
            config: Arc::new(crate::app::AppConfig {
                readonly: false,
                enable_log: false,
                auth_token: None,
                cors_origin: None,
                max_body_bytes: 1024 * 1024,
                max_per_page: 100,
                max_sql_scan_rows: 50_000,
                max_sql_selected_rows: 1_000,
            }),
            resources: Arc::new(RwLock::new(BTreeSet::new())),
            resource_cache: Arc::new(RwLock::new(HashMap::new())),
            resource_locks: Arc::new(RwLock::new(HashMap::new())),
            schema_store: Arc::new(std::sync::RwLock::new(crate::app::SchemaStore::default())),
            graphql_store: Arc::new(RwLock::new(crate::app::GraphqlStore::default())),
            metrics: Arc::new(crate::app::MetricsStore::default()),
            health: Arc::new(crate::app::HealthState::new(true, None)),
            event_bus: tokio::sync::broadcast::channel(16).0,
        }
    }

    #[tokio::test]
    async fn write_resource_survives_interrupted_temp_file_and_keeps_output_intact() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let state = test_state(DataSource::Folder(temp.path().to_path_buf()));
        let resource = "users";

        let target_file = temp.path().join("users.json");
        std::fs::write(&target_file, "[{\"id\":1}]\n").expect("write initial resource");

        let stale_temp = temp.path().join("users.json.tmp.crash-simulation");
        std::fs::write(&stale_temp, "[{\"id\":").expect("write stale temp file");

        let updated_value = serde_json::json!([
            {"id": 2, "name": "Ada"},
            {"id": 3, "name": "Lin"}
        ]);
        write_resource(&state, resource, &updated_value).await.expect("atomic write succeeds");

        let final_text = std::fs::read_to_string(&target_file).expect("read final resource file");
        let parsed: Value =
            serde_json::from_str(&final_text).expect("final file should be valid json");
        assert_eq!(parsed, updated_value);

        let tmp_entries = std::fs::read_dir(temp.path())
            .expect("list directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("users.json.tmp."))
            })
            .collect::<Vec<_>>();
        assert_eq!(tmp_entries, vec![stale_temp]);
    }

    #[tokio::test]
    async fn write_resource_rejects_non_object_database_file_roots() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let db_path = temp.path().join("db.json");
        std::fs::write(&db_path, "[{\"id\":1}]\n").expect("write invalid db root");

        let state = test_state(DataSource::File(PathBuf::from(&db_path)));
        state.resources.write().await.insert("users".to_string());

        let err = write_resource(&state, "users", &serde_json::json!([{"id": 1}]))
            .await
            .expect_err("write should fail");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Database file must contain a JSON object"));
    }
}
