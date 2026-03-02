use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock},
    time::SystemTime,
};

use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

use crate::{
    error::AppError,
    schema::{Schema, TableSchema},
};
use axum::http::StatusCode;
use serde_json::Value;

#[derive(Clone)]
pub struct AppState {
    pub folder: Arc<PathBuf>,
    pub resources: Arc<StdRwLock<BTreeSet<String>>>,
    pub resource_cache: Arc<StdRwLock<HashMap<String, CachedResource>>>,
    pub resource_locks: Arc<RwLock<HashMap<String, Arc<RwLock<()>>>>>,
    pub schema: Arc<Option<Schema>>,
    pub request_log: Option<Arc<StdMutex<fs::File>>>,
}

#[derive(Clone)]
pub struct CachedResource {
    pub value: Arc<Value>,
    pub metadata: CachedMetadata,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CachedMetadata {
    pub modified: SystemTime,
    pub len: u64,
}

impl AppState {
    pub fn schema_table(&self, resource: &str) -> Result<Option<&TableSchema>, AppError> {
        let Some(schema) = self.schema.as_ref() else {
            return Ok(None);
        };

        schema.tables.get(resource).map(Some).ok_or_else(|| {
            AppError::new(
                StatusCode::BAD_REQUEST,
                format!("Resource '{resource}' is not defined in schema"),
            )
        })
    }

    pub fn resource_names_sorted(&self) -> Result<Vec<String>, AppError> {
        Ok(self
            .resources
            .read()
            .map_err(|_| {
                AppError::new(StatusCode::INTERNAL_SERVER_ERROR, "Resource cache lock poisoned")
            })?
            .iter()
            .cloned()
            .collect())
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
}
