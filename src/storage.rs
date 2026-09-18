mod cache;
mod index;
mod io;
mod validation;

use std::{collections::BTreeMap, sync::Arc};

use axum::http::StatusCode;
use serde_json::Value;

use crate::{
    app::{AppState, DataSource},
    error::AppError,
    schema::{
        infer_schema_from_data_source, infer_table_from_value, replace_inferred_tables,
    },
};

pub(crate) use cache::cached_resource_from_value;
#[allow(unused_imports)]
pub use cache::{clear_resource_cache, remove_cached_resource, update_cached_resource};
pub use index::{
    build_id_index, coerce_id_value, find_item_by_key, find_item_index_by_key, next_numeric_id,
};
pub use io::{
    create_resource_value, delete_resource_value, is_reserved_resource_name,
    is_valid_resource_name, resource_file_path, scan_resources,
};
pub(crate) use validation::validate_resource_snapshot;
pub use validation::{validate_resource_data, validate_sql_identifier};

pub async fn load_resource(state: &AppState, resource: &str) -> Result<Arc<Value>, AppError> {
    let file = resource_file_path(&state.data_source, resource)?;
    let current_validation_revision = state.validation_schema_snapshot(resource).0;

    if let Some(cached) = state.resource_cache.read().await.get(resource).cloned() {
        if cached.validation_revision == Some(current_validation_revision) {
            state.metrics.record_resource_cache_hit();
            return Ok(cached.value);
        }

        state.metrics.record_resource_cache_revalidation();
        let value = cached.value;
        let validation_revision =
            validation::validate_resource_snapshot(state, resource, value.as_ref())?;
        cache::mark_cached_resource_validated(
            state,
            resource,
            value.clone(),
            validation_revision,
        )
        .await;
        return Ok(value);
    }

    state.metrics.record_resource_cache_miss();
    if !state.resources.read().await.contains(resource) {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            format!("Resource '{resource}' not found"),
        ));
    }

    let value = Arc::new(io::read_resource_value(&state.data_source, &file, resource).await?);
    let validation_revision =
        validation::validate_resource_snapshot(state, resource, value.as_ref())?;
    cache::mark_cached_resource_validated(state, resource, value.clone(), validation_revision).await;
    Ok(value)
}

pub async fn write_resource(
    state: &AppState,
    resource: &str,
    value: Value,
) -> Result<(), AppError> {
    write_resource_snapshot(state, resource, value, None).await
}

pub(crate) async fn write_validated_resource(
    state: &AppState,
    resource: &str,
    value: Value,
    validation_revision: u64,
) -> Result<(), AppError> {
    write_resource_snapshot(state, resource, value, Some(validation_revision)).await
}

async fn write_resource_snapshot(
    state: &AppState,
    resource: &str,
    value: Value,
    validation_revision: Option<u64>,
) -> Result<(), AppError> {
    let file = resource_file_path(&state.data_source, resource)?;
    let can_refresh_locally =
        matches!(state.data_source.as_ref(), DataSource::Folder(_)) && state.health.is_ready();
    let value = Arc::new(value);

    if matches!(state.data_source.as_ref(), DataSource::File(_)) {
        let _guard = state.write_lock_for_resource("__db_file__").await;
        io::persist_resource_value(&state.data_source, &file, resource, value.clone()).await?;
    } else {
        io::persist_resource_value(&state.data_source, &file, resource, value.clone()).await?;
    }

    // Persistence, cache installation, and schema inference now share this one immutable snapshot.
    // Once persistence succeeds the cache reflects the written value even if effective-schema
    // rebuilding reports an error, preserving the previous failure semantics.
    cache::update_cached_resource(state, resource, value.clone()).await;

    if can_refresh_locally {
        let resource_name = resource.to_string();
        let value_for_inference = value.clone();
        let table = tokio::task::spawn_blocking(move || {
            infer_table_from_value(&resource_name, value_for_inference.as_ref())
        })
        .await
        .map_err(|err| {
            AppError::internal(format!("Resource schema inference task failed: {err}"))
        })?;

        // Install the already-inferred table into the latest schema snapshot. Only metadata
        // relation rebuilding happens while holding this lock, so concurrent writes to different
        // resources cannot lose each other's inferred-table updates.
        let mut store = state.schema_store.write().expect("schema store");
        let inferred = replace_inferred_tables(
            store.inferred.clone(),
            BTreeMap::from([(resource.to_string(), table)]),
        );
        store.replace_inferred(inferred).map_err(AppError::internal)?;
    } else {
        // When readiness is already false, retain the previous fail-closed recovery semantics:
        // only a full refresh may prove that every resource is valid and mark the server ready.
        // Single-file databases also stay on the conservative full-refresh path in this slice.
        refresh_inferred_schema(state).await?;
    }

    // Rebuild cache metadata only after inferred-schema installation. A mutation may already have
    // validated this exact snapshot; carrying that revision avoids another O(N) read-admission scan.
    // If declared schema changed concurrently, the older revision remains visible and the next read
    // revalidates before trusting it.
    if let Some(validation_revision) = validation_revision {
        cache::mark_cached_resource_validated(
            state,
            resource,
            value.clone(),
            validation_revision,
        )
        .await;
    }

    state.invalidate_graphql_schema().await;
    state.emit_event("resource_changed", Some(resource.to_string()));
    state.emit_event("schema_changed", None);
    state.emit_event("overview_changed", None);
    Ok(())
}

pub async fn resource_exists(state: &AppState, resource: &str) -> Result<bool, AppError> {
    Ok(state.resources.read().await.contains(resource))
}

pub(crate) async fn refresh_inferred_schema(state: &AppState) -> Result<(), AppError> {
    let resources = state.resources.read().await.clone();
    let data_source = state.data_source.clone();
    let inferred = tokio::task::spawn_blocking(move || {
        infer_schema_from_data_source(&data_source, &resources)
    })
    .await
    .map_err(|err| AppError::internal(format!("Schema refresh task failed: {err}")))?
    .map_err(AppError::internal)?;
    state.update_inferred_schema(inferred).map_err(AppError::internal)?;
    state.health.mark_ready();
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet, HashMap},
        path::PathBuf,
        sync::Arc,
    };

    use tokio::sync::RwLock;

    use super::*;
    use crate::{
        app::{AppState, DataSource},
        schema::{ColumnSchema, ColumnType, DeclaredSchema, DeclaredTableSchema},
    };

    fn declared_users_schema(name_type: ColumnType) -> DeclaredSchema {
        DeclaredSchema {
            tables: BTreeMap::from([(
                "users".to_string(),
                DeclaredTableSchema {
                    primary_key: Some("id".to_string()),
                    columns: BTreeMap::from([
                        ("id".to_string(), ColumnSchema::new(ColumnType::Integer, false)),
                        ("name".to_string(), ColumnSchema::new(name_type, false)),
                    ]),
                    ..DeclaredTableSchema::default()
                },
            )]),
        }
    }

    fn test_state(data_source: DataSource) -> AppState {
        AppState {
            data_source: Arc::new(data_source),
            config: Arc::new(crate::app::AppConfig {
                readonly: false,
                enable_log: false,
                response_format: crate::app::ResponseFormat::Json,
                clone_proxy: None,
                auth_token: None,
                cors_origin: None,
                protect_ops: false,
                max_body_bytes: 1024 * 1024,
                max_query_bytes: 262_144,
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
    async fn load_resource_validates_immutable_snapshot_once_per_declared_revision() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let state = test_state(DataSource::Folder(temp.path().to_path_buf()));
        state.resources.write().await.insert("users".to_string());
        state
            .update_declared_schema(Some(declared_users_schema(ColumnType::String)))
            .expect("declared schema");
        std::fs::write(
            temp.path().join("users.json"),
            r#"[{"id":1,"name":"Ada"},{"id":2,"name":"Grace"},{"id":3,"name":"Lin"}]"#,
        )
        .expect("write users");

        let first = load_resource(&state, "users").await.expect("first load");
        let second = load_resource(&state, "users").await.expect("second load");

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(
            state
                .metrics
                .resource_cache_misses_total
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert_eq!(
            state
                .metrics
                .resource_cache_hits_total
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert_eq!(
            state
                .metrics
                .resource_validation_passes_total
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert_eq!(
            state
                .metrics
                .resource_validation_rows_total
                .load(std::sync::atomic::Ordering::Relaxed),
            3
        );
    }

    #[tokio::test]
    async fn declared_schema_change_revalidates_cached_snapshot_before_read() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let state = test_state(DataSource::Folder(temp.path().to_path_buf()));
        state.resources.write().await.insert("users".to_string());
        state
            .update_declared_schema(Some(declared_users_schema(ColumnType::String)))
            .expect("initial declared schema");
        std::fs::write(temp.path().join("users.json"), r#"[{"id":1,"name":"Ada"}]"#)
            .expect("write users");

        load_resource(&state, "users").await.expect("initial valid load");
        state
            .update_declared_schema(Some(declared_users_schema(ColumnType::Integer)))
            .expect("updated declared schema");

        let err = load_resource(&state, "users")
            .await
            .expect_err("cached snapshot must be revalidated");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert_eq!(
            state
                .metrics
                .resource_cache_revalidations_total
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert_eq!(
            state
                .metrics
                .resource_validation_passes_total
                .load(std::sync::atomic::Ordering::Relaxed),
            2
        );
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
        write_resource(&state, resource, updated_value.clone()).await.expect("atomic write succeeds");

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
    async fn write_resource_does_not_reread_unrelated_resource_files_while_ready() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let state = test_state(DataSource::Folder(temp.path().to_path_buf()));
        state.resources.write().await.extend(["users".to_string(), "broken".to_string()]);

        std::fs::write(temp.path().join("users.json"), "[{\"id\":1}]\n")
            .expect("write initial resource");
        let broken_path = temp.path().join("broken.json");
        std::fs::write(&broken_path, "{ definitely not json")
            .expect("write unrelated invalid resource");

        let updated = serde_json::json!([{"id": 1, "name": "Ada"}]);
        write_resource(&state, "users", updated)
            .await
            .expect("healthy localized write must not parse unrelated resources");

        assert_eq!(
            std::fs::read_to_string(&broken_path).expect("read unrelated resource"),
            "{ definitely not json"
        );
        assert!(state.schema_table("users").is_some());
        assert!(state.health.is_ready());
    }

    #[tokio::test]
    async fn write_resource_preserves_unrelated_not_ready_failure() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let state = test_state(DataSource::Folder(temp.path().to_path_buf()));
        state.resources.write().await.extend(["users".to_string(), "broken".to_string()]);
        state.health.mark_not_ready("broken resource");

        std::fs::write(temp.path().join("users.json"), "[{\"id\":1}]\n")
            .expect("write initial resource");
        std::fs::write(temp.path().join("broken.json"), "{ definitely not json")
            .expect("write unrelated invalid resource");

        let updated = serde_json::json!([{"id": 1, "name": "Ada"}]);
        let err = write_resource(&state, "users", updated)
            .await
            .expect_err("not-ready state requires full validation");

        assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!state.health.is_ready());
        assert!(state.health.last_error().is_some());
    }

    #[tokio::test]
    async fn concurrent_local_schema_refreshes_preserve_both_tables() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let state = test_state(DataSource::Folder(temp.path().to_path_buf()));
        state.resources.write().await.extend(["posts".to_string(), "users".to_string()]);
        std::fs::write(temp.path().join("users.json"), "[{\"id\":1}]\n")
            .expect("write users resource");
        std::fs::write(temp.path().join("posts.json"), "[{\"id\":10}]\n")
            .expect("write posts resource");

        let users = serde_json::json!([{"id": 1, "name": "Ada"}]);
        let posts = serde_json::json!([{"id": 10, "title": "Hello"}]);
        let (users_result, posts_result) = tokio::join!(
            write_resource(&state, "users", users),
            write_resource(&state, "posts", posts),
        );
        users_result.expect("users write succeeds");
        posts_result.expect("posts write succeeds");

        let inferred = state.inferred_schema_snapshot();
        assert!(inferred.tables.contains_key("users"));
        assert!(inferred.tables.contains_key("posts"));
    }

    #[tokio::test]
    async fn write_resource_rejects_non_object_database_file_roots() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let db_path = temp.path().join("db.json");
        std::fs::write(&db_path, "[{\"id\":1}]\n").expect("write invalid db root");

        let state = test_state(DataSource::File(PathBuf::from(&db_path)));
        state.resources.write().await.insert("users".to_string());

        let err = write_resource(&state, "users", serde_json::json!([{"id": 1}]))
            .await
            .expect_err("write should fail");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Database file must contain a JSON object"));
    }

    #[tokio::test]
    async fn write_resource_returns_internal_error_when_parent_directory_is_missing() {
        let temp = tempfile::tempdir().expect("create tempdir");
        let missing_folder = temp.path().join("missing");
        let state = test_state(DataSource::Folder(missing_folder));
        state.resources.write().await.insert("users".to_string());

        let err = write_resource(&state, "users", serde_json::json!([{"id": 1}]))
            .await
            .expect_err("write should fail");
        assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            state.resource_cache.read().await.get("users").is_none(),
            "failed writes must not update cache"
        );
    }
}
