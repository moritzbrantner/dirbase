use std::sync::Arc;

use serde_json::Value;

use crate::{
    app::{AppState, CachedResource},
    schema::{TableSchema, primary_key_name},
    storage::build_id_index,
};

pub(crate) fn cached_resource_from_value(
    value: Arc<Value>,
    table: Option<&TableSchema>,
) -> CachedResource {
    CachedResource {
        id_index: build_id_index(value.as_ref(), table),
        primary_key: primary_key_name(table).to_string(),
        value,
    }
}

pub async fn update_cached_resource(state: &AppState, resource: &str, value: Arc<Value>) {
    let table = state.schema_table(resource);
    state
        .resource_cache
        .write()
        .await
        .insert(resource.to_string(), cached_resource_from_value(value, table.as_ref()));
}

#[allow(dead_code)]
pub async fn remove_cached_resource(state: &AppState, resource: &str) {
    state.resource_cache.write().await.remove(resource);
}

#[allow(dead_code)]
pub async fn clear_resource_cache(state: &AppState) {
    state.resource_cache.write().await.clear();
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use serde_json::json;

    use super::*;
    use crate::schema::{ColumnSchema, ColumnType, TableKind};

    #[test]
    fn cached_resource_uses_schema_primary_key_for_id_index() {
        let table = TableSchema {
            kind: TableKind::Unknown,
            primary_key: Some("user_id".to_string()),
            columns: BTreeMap::from([(
                "user_id".to_string(),
                ColumnSchema::new(ColumnType::Integer, false),
            )]),
            foreign_keys: BTreeMap::new(),
            many_to_many: BTreeMap::new(),
        };
        let cached = cached_resource_from_value(
            Arc::new(json!([
                {"user_id": 10, "name": "Ada"},
                {"user_id": 20, "name": "Grace"}
            ])),
            Some(&table),
        );

        assert_eq!(cached.primary_key, "user_id");
        assert_eq!(cached.id_index.as_ref().expect("id index").get("20"), Some(&1));
    }
}
