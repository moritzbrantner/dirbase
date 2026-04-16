use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    sync::{Arc, RwLock as StdRwLock},
};

use async_graphql::dynamic::Schema as DynamicSchema;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

use crate::schema::{DeclaredSchema, DeclaredTableSchema, Schema, TableSchema, merge_schemas};
use serde_json::Value;

#[derive(Clone)]
pub enum DataSource {
    Folder(PathBuf),
    File(PathBuf),
}

#[derive(Clone)]
pub struct AppState {
    pub data_source: Arc<DataSource>,
    pub resources: Arc<RwLock<BTreeSet<String>>>,
    pub resource_cache: Arc<RwLock<HashMap<String, CachedResource>>>,
    pub resource_locks: Arc<RwLock<HashMap<String, Arc<RwLock<()>>>>>,
    pub schema_store: Arc<StdRwLock<SchemaStore>>,
    pub graphql_store: Arc<RwLock<GraphqlStore>>,
}

#[derive(Clone)]
pub struct CachedResource {
    pub value: Arc<Value>,
    pub id_index: Option<HashMap<String, usize>>,
    pub primary_key: String,
}

#[derive(Clone, Debug, Default)]
pub struct SchemaStore {
    pub declared: Option<DeclaredSchema>,
    pub inferred: Schema,
    pub merged: Schema,
}

#[derive(Clone, Debug, Default)]
pub struct GraphqlStore {
    pub schema: Option<DynamicSchema>,
    pub build_error: Option<String>,
}

impl SchemaStore {
    pub fn new(declared: Option<DeclaredSchema>, inferred: Schema) -> Result<Self, String> {
        let merged = merge_schemas(declared.as_ref(), &inferred)?;
        Ok(Self { declared, inferred, merged })
    }

    pub fn replace_inferred(&mut self, inferred: Schema) -> Result<(), String> {
        self.inferred = inferred;
        self.merged = merge_schemas(self.declared.as_ref(), &self.inferred)?;
        Ok(())
    }
}

impl AppState {
    pub fn schema_snapshot(&self) -> Schema {
        self.schema_store.read().expect("schema store").merged.clone()
    }

    pub fn schema_table(&self, resource: &str) -> Option<TableSchema> {
        self.schema_store.read().expect("schema store").merged.tables.get(resource).cloned()
    }

    pub fn validation_schema_table(&self, resource: &str) -> Option<DeclaredTableSchema> {
        let store = self.schema_store.read().expect("schema store");
        store.declared.as_ref().and_then(|schema| schema.tables.get(resource).cloned())
    }

    pub fn update_inferred_schema(&self, inferred: Schema) -> Result<(), String> {
        self.schema_store.write().expect("schema store").replace_inferred(inferred)
    }

    pub async fn resource_names_sorted(&self) -> Vec<String> {
        self.resources.read().await.iter().cloned().collect()
    }

    pub async fn read_lock_for_resource(&self, resource: &str) -> OwnedRwLockReadGuard<()> {
        self.resource_lock(resource).await.read_owned().await
    }

    pub async fn write_lock_for_resource(&self, resource: &str) -> OwnedRwLockWriteGuard<()> {
        self.resource_lock(resource).await.write_owned().await
    }

    pub async fn read_locks_for_resources(
        &self,
        resources: &[String],
    ) -> Vec<OwnedRwLockReadGuard<()>> {
        let mut guards = Vec::with_capacity(resources.len());
        for resource in resources {
            guards.push(self.read_lock_for_resource(resource).await);
        }
        guards
    }

    async fn resource_lock(&self, resource: &str) -> Arc<RwLock<()>> {
        if let Some(lock) = self.resource_locks.read().await.get(resource).cloned() {
            return lock;
        }

        let mut locks = self.resource_locks.write().await;
        locks.entry(resource.to_string()).or_insert_with(|| Arc::new(RwLock::new(()))).clone()
    }

    pub async fn invalidate_graphql_schema(&self) {
        let mut store = self.graphql_store.write().await;
        *store = GraphqlStore::default();
    }

    pub async fn graphql_schema(&self) -> Result<DynamicSchema, String> {
        {
            let store = self.graphql_store.read().await;
            if let Some(schema) = &store.schema {
                return Ok(schema.clone());
            }
            if let Some(error) = &store.build_error {
                return Err(error.clone());
            }
        }

        let built = crate::graphql::build_schema(self).await;
        let mut store = self.graphql_store.write().await;
        if store.schema.is_none() && store.build_error.is_none() {
            match built {
                Ok(schema) => {
                    store.schema = Some(schema.clone());
                    return Ok(schema);
                }
                Err(error) => {
                    store.build_error = Some(error.clone());
                    return Err(error);
                }
            }
        }

        if let Some(schema) = &store.schema {
            return Ok(schema.clone());
        }
        Err(store.build_error.clone().unwrap_or_else(|| "GraphQL schema unavailable".to_string()))
    }
}
