use std::{collections::HashMap, path::PathBuf, sync::Arc};

use app::{AppConfig, AppState, HealthState, MetricsStore};
use cli::{CliLoadError, load_cli};
use http::router::build_router;
use startup::{
    StartupSummary, browser_url_for, data_source_kind_label, data_source_path_label,
    print_startup_summary, resolve_data_source, schema_status_label,
};
use tokio::sync::RwLock;

mod app;
mod cli;
mod error;
mod graphql;
mod http;
mod mutation_service;
mod openapi;
mod query;
mod relations;
mod resource_service;
mod schema;
mod sql;
mod startup;
mod storage;
mod watcher;

use schema::{Schema, infer_schema_from_data_source, load_schema};
use storage::scan_resources;
use watcher::start_resource_watcher;

#[tokio::main]
async fn main() {
    let cli = match load_cli() {
        Ok(Some(cli)) => cli,
        Ok(None) => return,
        Err(CliLoadError::CommandLine(err)) => err.exit(),
        Err(CliLoadError::Config(message)) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };

    let _guard = if cli.log {
        let file_appender = tracing_appender::rolling::never(
            cli.logname.parent().unwrap_or(std::path::Path::new(".")),
            cli.logname.file_name().and_then(|n| n.to_str()).unwrap_or("requests.log"),
        );
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
        tracing_subscriber::fmt().with_writer(non_blocking).init();
        Some(guard)
    } else {
        tracing_subscriber::fmt::init();
        None
    };

    let data_source = resolve_data_source(&cli).await;

    let schema_root = match &data_source {
        app::DataSource::Folder(folder) => folder.clone(),
        app::DataSource::File(file) => {
            file.parent().map(|parent| parent.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
        }
    };

    let declared_schema = match load_schema(&schema_root, cli.schema.as_deref()) {
        Ok(schema) => schema,
        Err(err) => {
            eprintln!("Failed to load schema: {err}");
            std::process::exit(1);
        }
    };

    let initial_resources = scan_resources(&data_source).unwrap_or_default();
    let (inferred_schema, health) =
        match infer_schema_from_data_source(&data_source, &initial_resources) {
            Ok(schema) => (schema, Arc::new(HealthState::new(true, None))),
            Err(err) => {
                eprintln!("Failed to infer schema: {err}");
                (Schema::default(), Arc::new(HealthState::new(false, Some(err))))
            }
        };
    let startup_summary = StartupSummary {
        source_kind: data_source_kind_label(&data_source),
        source_path: data_source_path_label(&data_source),
        resource_count: initial_resources.len(),
        schema_status: schema_status_label(&declared_schema, &inferred_schema),
        mode: if cli.readonly { "readonly" } else { "read-write" },
    };
    let config = Arc::new(AppConfig {
        readonly: cli.readonly,
        enable_log: cli.log,
        response_format: cli.response_format,
        auth_token: cli.auth_token.clone(),
        cors_origin: cli.cors_origin.clone(),
        protect_ops: cli.protect_ops,
        max_body_bytes: cli.max_body_bytes,
        max_query_bytes: cli.max_query_bytes,
        max_per_page: cli.max_per_page,
        max_sql_scan_rows: cli.max_sql_scan_rows,
        max_sql_selected_rows: cli.max_sql_selected_rows,
    });
    let metrics = Arc::new(MetricsStore::default());
    let (event_bus, _) = tokio::sync::broadcast::channel(256);
    let state = AppState {
        data_source: Arc::new(data_source),
        config,
        resources: Arc::new(RwLock::new(initial_resources)),
        resource_cache: Arc::new(RwLock::new(HashMap::new())),
        resource_locks: Arc::new(RwLock::new(HashMap::new())),
        schema_store: Arc::new(std::sync::RwLock::new(
            app::SchemaStore::new(declared_schema, inferred_schema).unwrap_or_else(|err| {
                eprintln!("Failed to build schema: {err}");
                std::process::exit(1);
            }),
        )),
        graphql_store: Arc::new(RwLock::new(app::GraphqlStore::default())),
        metrics,
        health,
        event_bus,
    };

    start_resource_watcher(
        state.data_source.clone(),
        state.resources.clone(),
        state.resource_cache.clone(),
        state.schema_store.clone(),
        state.graphql_store.clone(),
        state.health.clone(),
        state.clone(),
    );

    let app = build_router(state.clone());
    let listener = tokio::net::TcpListener::bind(cli.bind).await.expect("binding server listener");
    let listen_addr = listener.local_addr().expect("reading server listener address");
    let browser_url = browser_url_for(listen_addr);
    tracing::info!(readonly = cli.readonly, "Readonly mode");
    tracing::info!(listen_addr = %listen_addr, browser_url = %browser_url, "Server started");
    print_startup_summary(&browser_url, &cli, &startup_summary);
    axum::serve(listener, app).await.expect("running server");
}
