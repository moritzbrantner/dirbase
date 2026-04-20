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

pub async fn create_item(
    state: &AppState,
    resource: &str,
    mut payload: Value,
) -> Result<Value, AppError> {
    let _guard = state.write_lock_for_resource(resource).await;
    let mut data = load_resource(state, resource).await?.as_ref().clone();
    validate_resource_data(state, resource, &data)?;
    let array = data
        .as_array_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    let item = payload
        .as_object_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Payload must be a JSON object"))?;
    let table = state.schema_table(resource);
    maybe_fill_missing_id(item, array, table.as_ref());
    let created = Value::Object(item.clone());
    array.push(created.clone());
    validate_resource_data(state, resource, &data)?;
    write_resource(state, resource, &data).await?;
    Ok(created)
}

pub async fn replace_item(
    state: &AppState,
    resource: &str,
    id: &str,
    mut payload: Value,
) -> Result<Value, AppError> {
    let _guard = state.write_lock_for_resource(resource).await;
    let mut data = load_resource(state, resource).await?.as_ref().clone();
    let table = state.schema_table(resource);
    let item_key = primary_key_name(table.as_ref()).to_string();
    let array = data
        .as_array_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    let object = payload
        .as_object_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Payload must be a JSON object"))?;
    object.insert(item_key.clone(), coerce_id_value(id, table.as_ref()));
    let replacement = Value::Object(object.clone());
    let position = find_item_index_by_key(array, &item_key, id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?;
    array[position] = replacement.clone();
    validate_resource_data(state, resource, &data)?;
    write_resource(state, resource, &data).await?;
    Ok(replacement)
}

pub async fn patch_item(
    state: &AppState,
    resource: &str,
    id: &str,
    payload: Value,
) -> Result<Value, AppError> {
    let _guard = state.write_lock_for_resource(resource).await;
    let mut data = load_resource(state, resource).await?.as_ref().clone();
    let table = state.schema_table(resource);
    let item_key = primary_key_name(table.as_ref()).to_string();
    validate_resource_data(state, resource, &data)?;
    let array = data
        .as_array_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    let patch = payload
        .as_object()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Payload must be a JSON object"))?;
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
    let updated = Value::Object(current.clone());
    validate_resource_data(state, resource, &data)?;
    write_resource(state, resource, &data).await?;
    Ok(updated)
}

pub async fn delete_item(state: &AppState, resource: &str, id: &str) -> Result<(), AppError> {
    let _guard = state.write_lock_for_resource(resource).await;
    let mut data = load_resource(state, resource).await?.as_ref().clone();
    let table = state.schema_table(resource);
    let item_key = primary_key_name(table.as_ref()).to_string();
    validate_resource_data(state, resource, &data)?;
    let array = data
        .as_array_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON array"))?;
    let index = find_item_index_by_key(array, &item_key, id)
        .ok_or_else(|| AppError::new(StatusCode::NOT_FOUND, "Item not found"))?;
    array.remove(index);
    validate_resource_data(state, resource, &data)?;
    write_resource(state, resource, &data).await?;
    Ok(())
}

pub async fn replace_resource_object(
    state: &AppState,
    resource: &str,
    payload: Value,
) -> Result<Value, AppError> {
    let _guard = state.write_lock_for_resource(resource).await;
    let mut data = load_resource(state, resource).await?.as_ref().clone();
    if !data.is_object() || !payload.is_object() {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Payload and resource must be JSON objects",
        ));
    }
    data = payload;
    validate_resource_data(state, resource, &data)?;
    write_resource(state, resource, &data).await?;
    Ok(data)
}

pub async fn patch_resource_object(
    state: &AppState,
    resource: &str,
    payload: Value,
) -> Result<Value, AppError> {
    let _guard = state.write_lock_for_resource(resource).await;
    let mut data = load_resource(state, resource).await?.as_ref().clone();
    let current = data
        .as_object_mut()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Resource is not a JSON object"))?;
    let patch = payload
        .as_object()
        .ok_or_else(|| AppError::new(StatusCode::BAD_REQUEST, "Payload must be a JSON object"))?;
    for (key, value) in patch {
        current.insert(key.clone(), value.clone());
    }
    let updated = Value::Object(current.clone());
    validate_resource_data(state, resource, &data)?;
    write_resource(state, resource, &data).await?;
    Ok(updated)
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
    use tokio::sync::RwLock;

    use super::*;
    use crate::{
        app::{
            AppConfig, AppState, DataSource, GraphqlStore, HealthState, MetricsStore, SchemaStore,
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
        (name.to_string(), ColumnSchema { column_type, nullable })
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
