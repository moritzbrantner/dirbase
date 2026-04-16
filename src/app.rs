use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    sync::{
        Arc, RwLock as StdRwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use async_graphql::dynamic::Schema as DynamicSchema;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock, broadcast};

use crate::schema::{DeclaredSchema, DeclaredTableSchema, Schema, TableSchema, merge_schemas};
use serde::Serialize;
use serde_json::Value;

#[derive(Clone)]
pub enum DataSource {
    Folder(PathBuf),
    File(PathBuf),
}

#[derive(Clone)]
pub struct AppState {
    pub data_source: Arc<DataSource>,
    pub config: Arc<AppConfig>,
    pub resources: Arc<RwLock<BTreeSet<String>>>,
    pub resource_cache: Arc<RwLock<HashMap<String, CachedResource>>>,
    pub resource_locks: Arc<RwLock<HashMap<String, Arc<RwLock<()>>>>>,
    pub schema_store: Arc<StdRwLock<SchemaStore>>,
    pub graphql_store: Arc<RwLock<GraphqlStore>>,
    pub metrics: Arc<MetricsStore>,
    pub health: Arc<HealthState>,
    pub event_bus: broadcast::Sender<ServerEvent>,
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

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub readonly: bool,
    pub enable_log: bool,
    pub auth_token: Option<String>,
    pub cors_origin: Option<String>,
    pub max_body_bytes: usize,
    pub max_per_page: usize,
    pub max_sql_scan_rows: usize,
    pub max_sql_selected_rows: usize,
}

#[derive(Debug, Default)]
pub struct MetricsStore {
    pub requests_total: AtomicU64,
    pub responses_total: AtomicU64,
    pub responses_error: AtomicU64,
    pub auth_failures: AtomicU64,
    pub events_sent: AtomicU64,
}

#[derive(Debug, Default)]
pub struct HealthState {
    ready: AtomicBool,
    last_error: StdRwLock<Option<String>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ServerEvent {
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
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

    pub fn replace_declared(&mut self, declared: Option<DeclaredSchema>) -> Result<(), String> {
        self.declared = declared;
        self.merged = merge_schemas(self.declared.as_ref(), &self.inferred)?;
        Ok(())
    }
}

impl MetricsStore {
    pub fn record_request(&self) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_response(&self, is_error: bool) {
        self.responses_total.fetch_add(1, Ordering::Relaxed);
        if is_error {
            self.responses_error.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn record_auth_failure(&self) {
        self.auth_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_event(&self) {
        self.events_sent.fetch_add(1, Ordering::Relaxed);
    }

    pub fn render_prometheus(&self) -> String {
        format!(
            concat!(
                "# HELP folder_server_requests_total Total HTTP requests received.\n",
                "# TYPE folder_server_requests_total counter\n",
                "folder_server_requests_total {}\n",
                "# HELP folder_server_responses_total Total HTTP responses sent.\n",
                "# TYPE folder_server_responses_total counter\n",
                "folder_server_responses_total {}\n",
                "# HELP folder_server_responses_error_total Total HTTP error responses sent.\n",
                "# TYPE folder_server_responses_error_total counter\n",
                "folder_server_responses_error_total {}\n",
                "# HELP folder_server_auth_failures_total Total failed auth checks.\n",
                "# TYPE folder_server_auth_failures_total counter\n",
                "folder_server_auth_failures_total {}\n",
                "# HELP folder_server_events_sent_total Total SSE events published.\n",
                "# TYPE folder_server_events_sent_total counter\n",
                "folder_server_events_sent_total {}\n"
            ),
            self.requests_total.load(Ordering::Relaxed),
            self.responses_total.load(Ordering::Relaxed),
            self.responses_error.load(Ordering::Relaxed),
            self.auth_failures.load(Ordering::Relaxed),
            self.events_sent.load(Ordering::Relaxed),
        )
    }
}

impl HealthState {
    pub fn new(ready: bool, last_error: Option<String>) -> Self {
        Self { ready: AtomicBool::new(ready), last_error: StdRwLock::new(last_error) }
    }

    pub fn mark_ready(&self) {
        self.ready.store(true, Ordering::Relaxed);
        *self.last_error.write().expect("health state") = None;
    }

    pub fn mark_not_ready(&self, error: impl Into<String>) {
        self.ready.store(false, Ordering::Relaxed);
        *self.last_error.write().expect("health state") = Some(error.into());
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    pub fn last_error(&self) -> Option<String> {
        self.last_error.read().expect("health state").clone()
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

    pub fn update_declared_schema(&self, declared: Option<DeclaredSchema>) -> Result<(), String> {
        self.schema_store.write().expect("schema store").replace_declared(declared)
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

    pub fn emit_event(&self, kind: &'static str, resource: Option<String>) {
        let _ = self.event_bus.send(ServerEvent { kind, resource });
        self.metrics.record_event();
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<ServerEvent> {
        self.event_bus.subscribe()
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
