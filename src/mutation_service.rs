use axum::http::StatusCode;
use serde_json::Value;

use crate::{
    app::AppState,
    error::AppError,
    schema::{TableSchema, primary_key_name},
    storage::{
        coerce_id_value, find_item_index_by_key, load_resource, next_numeric_id,
        validate_resource_data, write_resource,
    },
};

enum ResourceValidation {
    BeforeAndAfter,
    AfterOnly,
}

async fn update_locked_resource<T>(
    state: &AppState,
    resource: &str,
    validation: ResourceValidation,
    update: impl FnOnce(&mut Value) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let _guard = state.write_lock_for_resource(resource).await;
    let mut data = load_resource(state, resource).await?.as_ref().clone();
    if matches!(validation, ResourceValidation::BeforeAndAfter) {
        validate_resource_data(state, resource, &data)?;
    }
    let result = update(&mut data)?;
    validate_resource_data(state, resource, &data)?;
    write_resource(state, resource, &data).await?;
    Ok(result)
}

pub async fn create_item(
    state: &AppState,
    resource: &str,
    mut payload: Value,
) -> Result<Value, AppError> {
    let table = state.schema_table(resource);
    update_locked_resource(state, resource, ResourceValidation::BeforeAndAfter, |data| {
        let array = data.as_array_mut().ok_or_else(|| {
            AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array")
        })?;
        let item = payload.as_object_mut().ok_or_else(|| {
            AppError::new(StatusCode::BAD_REQUEST, "Payload must be a JSON object")
        })?;
        maybe_fill_missing_id(item, array, table.as_ref());
        let created = Value::Object(item.clone());
        array.push(created.clone());
        Ok(created)
    })
    .await
}

pub async fn replace_item(
    state: &AppState,
    resource: &str,
    id: &str,
    mut payload: Value,
) -> Result<Value, AppError> {
    let table = state.schema_table(resource);
    let item_key = primary_key_name(table.as_ref()).to_string();
    update_locked_resource(state, resource, ResourceValidation::AfterOnly, |data| {
        let array = data.as_array_mut().ok_or_else(|| {
            AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array")
        })?;
        let object = payload.as_object_mut().ok_or_else(|| {
            AppError::new(StatusCode::BAD_REQUEST, "Payload must be a JSON object")
        })?;
        object.insert(item_key.clone(), coerce_id_value(id, table.as_ref()));
        let replacement = Value::Object(object.clone());
        let position = find_item_index_by_key(array, &item_key, id)
            .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?;
        array[position] = replacement.clone();
        Ok(replacement)
    })
    .await
}

pub async fn patch_item(
    state: &AppState,
    resource: &str,
    id: &str,
    payload: Value,
) -> Result<Value, AppError> {
    let table = state.schema_table(resource);
    let item_key = primary_key_name(table.as_ref()).to_string();
    update_locked_resource(state, resource, ResourceValidation::BeforeAndAfter, |data| {
        let array = data.as_array_mut().ok_or_else(|| {
            AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array")
        })?;
        let patch = payload.as_object().ok_or_else(|| {
            AppError::new(StatusCode::BAD_REQUEST, "Payload must be a JSON object")
        })?;
        let index = find_item_index_by_key(array, &item_key, id)
            .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?;
        let current = array[index].as_object_mut().ok_or_else(|| {
            AppError::new(StatusCode::BAD_REQUEST, "Array item must be a JSON object")
        })?;
        for (key, value) in patch {
            if key != &item_key {
                current.insert(key.clone(), value.clone());
            }
        }
        Ok(Value::Object(current.clone()))
    })
    .await
}

pub async fn delete_item(state: &AppState, resource: &str, id: &str) -> Result<(), AppError> {
    let table = state.schema_table(resource);
    let item_key = primary_key_name(table.as_ref()).to_string();
    update_locked_resource(state, resource, ResourceValidation::BeforeAndAfter, |data| {
        let array = data.as_array_mut().ok_or_else(|| {
            AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array")
        })?;
        let index = find_item_index_by_key(array, &item_key, id)
            .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?;
        array.remove(index);
        Ok(())
    })
    .await
}

pub async fn replace_resource_object(
    state: &AppState,
    resource: &str,
    payload: Value,
) -> Result<Value, AppError> {
    update_locked_resource(state, resource, ResourceValidation::AfterOnly, |data| {
        if !data.is_object() || !payload.is_object() {
            return Err(AppError::new(
                StatusCode::BAD_REQUEST,
                "Payload and resource must be JSON objects",
            ));
        }
        *data = payload;
        Ok(data.clone())
    })
    .await
}

pub async fn patch_resource_object(
    state: &AppState,
    resource: &str,
    payload: Value,
) -> Result<Value, AppError> {
    update_locked_resource(state, resource, ResourceValidation::AfterOnly, |data| {
        let current = data.as_object_mut().ok_or_else(|| {
            AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON object")
        })?;
        let patch = payload.as_object().ok_or_else(|| {
            AppError::new(StatusCode::BAD_REQUEST, "Payload must be a JSON object")
        })?;
        for (key, value) in patch {
            current.insert(key.clone(), value.clone());
        }
        Ok(Value::Object(current.clone()))
    })
    .await
}

fn maybe_fill_missing_id(
    item: &mut serde_json::Map<String, Value>,
    array: &[Value],
    table: Option<&TableSchema>,
) {
    let item_key = primary_key_name(table);
    if item.contains_key(item_key) {
        return;
    }
    let id_value = match table.and_then(|table| table.columns.get(item_key)) {
        Some(column) if matches!(column.column_type, crate::schema::ColumnType::String) => {
            Value::String(format!("{}", next_numeric_id(array, item_key)))
        }
        _ => Value::from(next_numeric_id(array, item_key)),
    };
    item.insert(item_key.to_string(), id_value);
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet, HashMap},
        path::Path,
        sync::Arc,
    };

    use axum::http::StatusCode;
    use serde_json::{Value, json};
    use tokio::sync::{Barrier, RwLock};

    use super::*;
    use crate::{
        app::{
            AppConfig, AppState, DataSource, GraphqlStore, HealthState, MetricsStore,
            ResponseFormat, SchemaStore,
        },
        schema::{
            ColumnSchema, ColumnType, DeclaredSchema, DeclaredTableSchema, Schema, TableKind,
        },
    };

    fn test_state_for_folder(
        temp_path: &Path,
        resource_names: &[&str],
        declared_schema_opt: Option<DeclaredSchema>,
    ) -> AppState {
        AppState {
            data_source: Arc::new(DataSource::Folder(temp_path.to_path_buf())),
            config: Arc::new(AppConfig {
                readonly: false,
                enable_log: false,
                response_format: ResponseFormat::Json,
                auth_token: None,
                cors_origin: None,
                max_body_bytes: 1024 * 1024,
                max_per_page: 100,
                max_sql_scan_rows: 50_000,
                max_sql_selected_rows: 1_000,
            }),
            resources: Arc::new(RwLock::new(
                resource_names.iter().map(|name| (*name).to_string()).collect::<BTreeSet<_>>(),
            )),
            resource_cache: Arc::new(RwLock::new(HashMap::new())),
            resource_locks: Arc::new(RwLock::new(HashMap::new())),
            schema_store: Arc::new(std::sync::RwLock::new(
                SchemaStore::new(declared_schema_opt, Schema::default())
                    .expect("valid schema store"),
            )),
            graphql_store: Arc::new(RwLock::new(GraphqlStore::default())),
            metrics: Arc::new(MetricsStore::default()),
            health: Arc::new(HealthState::new(true, None)),
            event_bus: tokio::sync::broadcast::channel(16).0,
        }
    }

    fn write_json(path: &Path, value: &Value) {
        std::fs::write(
            path,
            format!("{}\n", serde_json::to_string_pretty(value).expect("json payload")),
        )
        .expect("write json");
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&std::fs::read_to_string(path).expect("read json"))
            .expect("parse json")
    }

    fn declared_schema(resource: &str, table: DeclaredTableSchema) -> DeclaredSchema {
        let mut tables = BTreeMap::new();
        tables.insert(resource.to_string(), table);
        DeclaredSchema { tables }
    }

    fn declared_column(
        name: &str,
        column_type: ColumnType,
        nullable: bool,
    ) -> (String, ColumnSchema) {
        (name.to_string(), ColumnSchema::new(column_type, nullable))
    }

    #[tokio::test]
    async fn create_item_generates_numeric_id_when_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("users.json");
        write_json(&path, &json!([{"id": 2, "name": "Ada"}]));
        let state = test_state_for_folder(
            temp.path(),
            &["users"],
            Some(declared_schema(
                "users",
                DeclaredTableSchema {
                    primary_key: Some("id".to_string()),
                    columns: BTreeMap::from([
                        declared_column("id", ColumnType::Integer, false),
                        declared_column("name", ColumnType::String, false),
                    ]),
                    ..DeclaredTableSchema::default()
                },
            )),
        );

        let created = create_item(&state, "users", json!({"name": "Grace"})).await.expect("create");
        assert_eq!(created, json!({"id": 3, "name": "Grace"}));
        assert_eq!(read_json(&path), json!([{"id": 2, "name": "Ada"}, {"id": 3, "name": "Grace"}]));
    }

    #[tokio::test]
    async fn create_item_generates_string_id_when_declared_pk_is_string() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("users.json");
        write_json(&path, &json!([]));
        let state = test_state_for_folder(
            temp.path(),
            &["users"],
            Some(declared_schema(
                "users",
                DeclaredTableSchema {
                    primary_key: Some("slug".to_string()),
                    columns: BTreeMap::from([
                        declared_column("slug", ColumnType::String, false),
                        declared_column("name", ColumnType::String, false),
                    ]),
                    ..DeclaredTableSchema::default()
                },
            )),
        );

        let created = create_item(&state, "users", json!({"name": "Ada"})).await.expect("create");
        assert_eq!(created, json!({"slug": "1", "name": "Ada"}));
        assert_eq!(read_json(&path), json!([{"slug": "1", "name": "Ada"}]));
    }

    #[tokio::test]
    async fn create_item_preserves_explicit_id() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("users.json");
        write_json(&path, &json!([]));
        let state = test_state_for_folder(temp.path(), &["users"], None);

        let created = create_item(&state, "users", json!({"id": "user-7", "name": "Ada"}))
            .await
            .expect("create");
        assert_eq!(created, json!({"id": "user-7", "name": "Ada"}));
        assert_eq!(read_json(&path), json!([{"id": "user-7", "name": "Ada"}]));
    }

    #[tokio::test]
    async fn concurrent_create_item_calls_on_same_resource_do_not_lose_rows() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("users.json");
        write_json(&path, &json!([{"id": 1, "name": "Ada"}]));
        let state = test_state_for_folder(temp.path(), &["users"], None);
        let barrier = Arc::new(Barrier::new(9));

        let mut handles = Vec::new();
        for index in 0..8 {
            let state = state.clone();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                create_item(&state, "users", json!({"name": format!("User {index}")}))
                    .await
                    .expect("create")
            }));
        }

        barrier.wait().await;
        let mut created_ids = Vec::new();
        for handle in handles {
            let created = handle.await.expect("join create task");
            created_ids.push(created["id"].as_i64().expect("numeric id"));
        }
        created_ids.sort_unstable();

        let persisted = read_json(&path);
        let rows = persisted.as_array().expect("users array");
        let mut persisted_ids = rows
            .iter()
            .map(|row| row["id"].as_i64().expect("persisted numeric id"))
            .collect::<Vec<_>>();
        persisted_ids.sort_unstable();

        assert_eq!(created_ids, vec![2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(persisted_ids, vec![1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(rows.len(), 9);
    }

    #[tokio::test]
    async fn create_item_rejects_non_object_payload() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("users.json");
        write_json(&path, &json!([]));
        let state = test_state_for_folder(temp.path(), &["users"], None);

        let err = create_item(&state, "users", json!(["bad"])).await.expect_err("payload");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Payload must be a JSON object"));
        assert_eq!(read_json(&path), json!([]));
    }

    #[tokio::test]
    async fn create_item_rejects_non_array_resource() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("profile.json");
        write_json(&path, &json!({"name": "Ada"}));
        let state = test_state_for_folder(temp.path(), &["profile"], None);

        let err =
            create_item(&state, "profile", json!({"name": "Grace"})).await.expect_err("resource");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Resource is not a JSON array"));
        assert_eq!(read_json(&path), json!({"name": "Ada"}));
    }

    #[tokio::test]
    async fn replace_item_overwrites_target_and_coerces_path_id() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("posts.json");
        write_json(
            &path,
            &json!([
                {"id": 1, "title": "hello", "status": "published"},
                {"id": 2, "title": "world", "status": "draft"}
            ]),
        );
        let state = test_state_for_folder(temp.path(), &["posts"], None);

        let replacement =
            replace_item(&state, "posts", "2", json!({"title": "updated"})).await.expect("replace");
        assert_eq!(replacement, json!({"id": 2, "title": "updated"}));
        assert_eq!(
            read_json(&path),
            json!([
                {"id": 1, "title": "hello", "status": "published"},
                {"id": 2, "title": "updated"}
            ])
        );
    }

    #[tokio::test]
    async fn replace_item_rejects_non_object_payload() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("posts.json");
        write_json(&path, &json!([{"id": 1, "title": "hello"}]));
        let state = test_state_for_folder(temp.path(), &["posts"], None);

        let err = replace_item(&state, "posts", "1", json!(["bad"])).await.expect_err("payload");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Payload must be a JSON object"));
        assert_eq!(read_json(&path), json!([{"id": 1, "title": "hello"}]));
    }

    #[tokio::test]
    async fn replace_item_returns_not_found_for_missing_id() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("posts.json");
        write_json(&path, &json!([{"id": 1, "title": "hello"}]));
        let state = test_state_for_folder(temp.path(), &["posts"], None);

        let err = replace_item(&state, "posts", "99", json!({"title": "ghost"}))
            .await
            .expect_err("missing");
        assert_eq!(err.status, StatusCode::NOT_FOUND);
        assert!(err.message.contains("Item not found"));
        assert_eq!(read_json(&path), json!([{"id": 1, "title": "hello"}]));
    }

    #[tokio::test]
    async fn patch_item_merges_fields_without_overwriting_primary_key() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("users.json");
        write_json(
            &path,
            &json!([{"slug": "ada", "name": "Ada", "role": "admin", "active": true}]),
        );
        let state = test_state_for_folder(
            temp.path(),
            &["users"],
            Some(declared_schema(
                "users",
                DeclaredTableSchema {
                    primary_key: Some("slug".to_string()),
                    columns: BTreeMap::from([
                        declared_column("slug", ColumnType::String, false),
                        declared_column("name", ColumnType::String, false),
                        declared_column("role", ColumnType::String, false),
                        declared_column("active", ColumnType::Boolean, false),
                    ]),
                    ..DeclaredTableSchema::default()
                },
            )),
        );

        let updated = patch_item(&state, "users", "ada", json!({"slug": "grace", "name": "Grace"}))
            .await
            .expect("patch");
        assert_eq!(
            updated,
            json!({"slug": "ada", "name": "Grace", "role": "admin", "active": true})
        );
        assert_eq!(
            read_json(&path),
            json!([{"slug": "ada", "name": "Grace", "role": "admin", "active": true}])
        );
    }

    #[tokio::test]
    async fn patch_item_rejects_non_object_payload() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("posts.json");
        write_json(&path, &json!([{"id": 1, "title": "hello"}]));
        let state = test_state_for_folder(temp.path(), &["posts"], None);

        let err = patch_item(&state, "posts", "1", json!(["bad"])).await.expect_err("payload");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Payload must be a JSON object"));
        assert_eq!(read_json(&path), json!([{"id": 1, "title": "hello"}]));
    }

    #[tokio::test]
    async fn patch_item_returns_not_found_for_missing_id() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("posts.json");
        write_json(&path, &json!([{"id": 1, "title": "hello"}]));
        let state = test_state_for_folder(temp.path(), &["posts"], None);

        let err = patch_item(&state, "posts", "99", json!({"title": "ghost"}))
            .await
            .expect_err("missing");
        assert_eq!(err.status, StatusCode::NOT_FOUND);
        assert!(err.message.contains("Item not found"));
        assert_eq!(read_json(&path), json!([{"id": 1, "title": "hello"}]));
    }

    #[tokio::test]
    async fn patch_item_rejects_non_object_array_member() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("posts.json");
        write_json(&path, &json!([1, 2, 3]));
        let state = test_state_for_folder(temp.path(), &["posts"], None);

        let err =
            patch_item(&state, "posts", "1", json!({"title": "ghost"})).await.expect_err("missing");
        assert_eq!(err.status, StatusCode::NOT_FOUND);
        assert!(err.message.contains("Item not found"));
        assert_eq!(read_json(&path), json!([1, 2, 3]));
    }

    #[tokio::test]
    async fn delete_item_removes_target_row() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("posts.json");
        write_json(&path, &json!([{"id": 1, "title": "hello"}, {"id": 2, "title": "world"}]));
        let state = test_state_for_folder(temp.path(), &["posts"], None);

        delete_item(&state, "posts", "1").await.expect("delete");
        assert_eq!(read_json(&path), json!([{"id": 2, "title": "world"}]));
    }

    #[tokio::test]
    async fn concurrent_mixed_item_mutations_on_same_resource_are_all_persisted() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("posts.json");
        write_json(
            &path,
            &json!([
                {"id": 1, "title": "hello", "status": "draft"},
                {"id": 2, "title": "world", "status": "draft"},
                {"id": 3, "title": "old", "status": "archived"}
            ]),
        );
        let state = test_state_for_folder(temp.path(), &["posts"], None);
        let barrier = Arc::new(Barrier::new(5));

        let patch_state = state.clone();
        let patch_barrier = barrier.clone();
        let patch_task = tokio::spawn(async move {
            patch_barrier.wait().await;
            patch_item(&patch_state, "posts", "1", json!({"status": "published"}))
                .await
                .expect("patch")
        });

        let replace_state = state.clone();
        let replace_barrier = barrier.clone();
        let replace_task = tokio::spawn(async move {
            replace_barrier.wait().await;
            replace_item(&replace_state, "posts", "2", json!({"title": "updated"}))
                .await
                .expect("replace")
        });

        let delete_state = state.clone();
        let delete_barrier = barrier.clone();
        let delete_task = tokio::spawn(async move {
            delete_barrier.wait().await;
            delete_item(&delete_state, "posts", "3").await.expect("delete");
        });

        let create_state = state.clone();
        let create_barrier = barrier.clone();
        let create_task = tokio::spawn(async move {
            create_barrier.wait().await;
            create_item(&create_state, "posts", json!({"id": 99, "title": "new"}))
                .await
                .expect("create")
        });

        barrier.wait().await;
        let (patched, replaced, deleted, created) =
            tokio::join!(patch_task, replace_task, delete_task, create_task);
        patched.expect("join patch task");
        replaced.expect("join replace task");
        deleted.expect("join delete task");
        created.expect("join create task");

        let persisted = read_json(&path);
        assert_eq!(
            persisted,
            json!([
                {"id": 1, "title": "hello", "status": "published"},
                {"id": 2, "title": "updated"},
                {"id": 99, "title": "new"}
            ])
        );
    }

    #[tokio::test]
    async fn concurrent_update_and_delete_conflict_keeps_resource_valid() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("posts.json");
        write_json(&path, &json!([{"id": 1, "title": "hello", "status": "draft"}]));
        let state = test_state_for_folder(temp.path(), &["posts"], None);
        let barrier = Arc::new(Barrier::new(3));

        let patch_state = state.clone();
        let patch_barrier = barrier.clone();
        let patch_task = tokio::spawn(async move {
            patch_barrier.wait().await;
            patch_item(&patch_state, "posts", "1", json!({"status": "published"})).await
        });

        let delete_state = state.clone();
        let delete_barrier = barrier.clone();
        let delete_task = tokio::spawn(async move {
            delete_barrier.wait().await;
            delete_item(&delete_state, "posts", "1").await
        });

        barrier.wait().await;
        let patch_result = patch_task.await.expect("join patch task");
        delete_task.await.expect("join delete task").expect("delete");

        if let Err(err) = patch_result {
            assert_eq!(err.status, StatusCode::NOT_FOUND);
            assert!(err.message.contains("Item not found"));
        }
        assert_eq!(read_json(&path), json!([]));
    }

    #[tokio::test]
    async fn delete_item_returns_not_found_for_missing_id() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("posts.json");
        write_json(&path, &json!([{"id": 1, "title": "hello"}]));
        let state = test_state_for_folder(temp.path(), &["posts"], None);

        let err = delete_item(&state, "posts", "99").await.expect_err("missing");
        assert_eq!(err.status, StatusCode::NOT_FOUND);
        assert!(err.message.contains("Item not found"));
        assert_eq!(read_json(&path), json!([{"id": 1, "title": "hello"}]));
    }

    #[tokio::test]
    async fn replace_resource_object_replaces_whole_object() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("profile.json");
        write_json(&path, &json!({"name": "Ada", "theme": "dark"}));
        let state = test_state_for_folder(
            temp.path(),
            &["profile"],
            Some(declared_schema(
                "profile",
                DeclaredTableSchema {
                    kind: Some(TableKind::Object),
                    columns: BTreeMap::from([
                        declared_column("name", ColumnType::String, false),
                        declared_column("theme", ColumnType::String, false),
                    ]),
                    ..DeclaredTableSchema::default()
                },
            )),
        );

        let updated =
            replace_resource_object(&state, "profile", json!({"name": "Grace", "theme": "light"}))
                .await
                .expect("replace");
        assert_eq!(updated, json!({"name": "Grace", "theme": "light"}));
        assert_eq!(read_json(&path), json!({"name": "Grace", "theme": "light"}));
    }

    #[tokio::test]
    async fn replace_resource_object_rejects_non_object_payload() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("profile.json");
        write_json(&path, &json!({"name": "Ada"}));
        let state = test_state_for_folder(temp.path(), &["profile"], None);

        let err =
            replace_resource_object(&state, "profile", json!(["bad"])).await.expect_err("payload");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Payload and resource must be JSON objects"));
        assert_eq!(read_json(&path), json!({"name": "Ada"}));
    }

    #[tokio::test]
    async fn replace_resource_object_rejects_non_object_resource() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("posts.json");
        write_json(&path, &json!([{"id": 1}]));
        let state = test_state_for_folder(temp.path(), &["posts"], None);

        let err =
            replace_resource_object(&state, "posts", json!({"id": 2})).await.expect_err("resource");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Payload and resource must be JSON objects"));
        assert_eq!(read_json(&path), json!([{"id": 1}]));
    }

    #[tokio::test]
    async fn patch_resource_object_merges_fields() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("profile.json");
        write_json(&path, &json!({"name": "Ada", "theme": "dark", "lang": "en"}));
        let state = test_state_for_folder(temp.path(), &["profile"], None);

        let updated = patch_resource_object(&state, "profile", json!({"theme": "light"}))
            .await
            .expect("patch");
        assert_eq!(updated, json!({"name": "Ada", "theme": "light", "lang": "en"}));
        assert_eq!(read_json(&path), json!({"name": "Ada", "theme": "light", "lang": "en"}));
    }

    #[tokio::test]
    async fn patch_resource_object_rejects_non_object_payload() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("profile.json");
        write_json(&path, &json!({"name": "Ada"}));
        let state = test_state_for_folder(temp.path(), &["profile"], None);

        let err =
            patch_resource_object(&state, "profile", json!(["bad"])).await.expect_err("payload");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Payload must be a JSON object"));
        assert_eq!(read_json(&path), json!({"name": "Ada"}));
    }

    #[tokio::test]
    async fn patch_resource_object_rejects_non_object_resource() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("posts.json");
        write_json(&path, &json!([{"id": 1}]));
        let state = test_state_for_folder(temp.path(), &["posts"], None);

        let err =
            patch_resource_object(&state, "posts", json!({"id": 2})).await.expect_err("resource");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Resource is not a JSON object"));
        assert_eq!(read_json(&path), json!([{"id": 1}]));
    }
}
