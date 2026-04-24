use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
};

use app::{AppConfig, AppState, HealthState, MetricsStore};
use clap::{CommandFactory, Parser};
use tokio::sync::RwLock;

mod app;
mod error;
mod graphql;
mod http;
mod mutation_service;
mod query;
mod relations;
mod schema;
mod sql;
mod storage;
mod watcher;

use http::routes::build_router;
use schema::{Schema, infer_schema_from_data_source, load_schema};
use storage::scan_resources;
use watcher::start_resource_watcher;

#[derive(Parser, Debug)]
#[command(author, version, about = "Serve JSON resources from a folder or database file")]
struct Cli {
    #[arg(value_name = "PATH", conflicts_with_all = ["folder", "file"])]
    path: Option<PathBuf>,
    #[arg(short, long, conflicts_with_all = ["file", "path"])]
    folder: Option<PathBuf>,
    #[arg(long, conflicts_with_all = ["folder", "path"])]
    file: Option<PathBuf>,
    #[arg(short, long, default_value = "127.0.0.1:4444")]
    bind: SocketAddr,
    #[arg(long)]
    readonly: bool,
    #[arg(long)]
    schema: Option<PathBuf>,
    #[arg(long)]
    log: bool,
    #[arg(long, default_value = "requests.log")]
    logname: PathBuf,
    #[arg(long)]
    auth_token: Option<String>,
    #[arg(long)]
    cors_origin: Option<String>,
    #[arg(long, default_value_t = 1024 * 1024)]
    max_body_bytes: usize,
    #[arg(long, default_value_t = 100)]
    max_per_page: usize,
    #[arg(long, default_value_t = 50_000)]
    max_sql_scan_rows: usize,
    #[arg(long, default_value_t = 1_000)]
    max_sql_selected_rows: usize,
}

#[tokio::main]
async fn main() {
    if std::env::args_os().len() == 1 {
        let mut command = Cli::command();
        command.print_help().expect("print CLI help");
        println!();
        return;
    }

    let cli = Cli::parse();

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
    let config = Arc::new(AppConfig {
        readonly: cli.readonly,
        enable_log: cli.log,
        auth_token: cli.auth_token.clone(),
        cors_origin: cli.cors_origin.clone(),
        max_body_bytes: cli.max_body_bytes,
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
    eprintln!("Open {browser_url}");
    axum::serve(listener, app).await.expect("running server");
}

fn browser_url_for(addr: SocketAddr) -> String {
    let browser_addr = match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), addr.port())
        }
        IpAddr::V6(ip) if ip.is_unspecified() => {
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), addr.port())
        }
        _ => addr,
    };

    format!("http://{browser_addr}/")
}

async fn resolve_data_source(cli: &Cli) -> app::DataSource {
    if let Some(file) = cli.file.clone() {
        if let Err(err) = tokio::fs::try_exists(&file).await {
            eprintln!("Failed to inspect data file {}: {err}", file.display());
            std::process::exit(1);
        }
        return app::DataSource::File(file);
    }

    if let Some(folder) = cli.folder.clone() {
        ensure_folder_exists(&folder).await;
        return app::DataSource::Folder(folder);
    }

    if let Some(path) = cli.path.clone() {
        match tokio::fs::metadata(&path).await {
            Ok(metadata) if metadata.is_file() => return app::DataSource::File(path),
            Ok(metadata) if metadata.is_dir() => return app::DataSource::Folder(path),
            Ok(_) => {
                eprintln!("Path {} is neither a regular file nor a directory", path.display());
                std::process::exit(1);
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                    return app::DataSource::File(path);
                }
                ensure_folder_exists(&path).await;
                return app::DataSource::Folder(path);
            }
            Err(err) => {
                eprintln!("Failed to inspect path {}: {err}", path.display());
                std::process::exit(1);
            }
        }
    }

    let folder = PathBuf::from("./data");
    ensure_folder_exists(&folder).await;
    app::DataSource::Folder(folder)
}

async fn ensure_folder_exists(folder: &std::path::Path) {
    if let Err(err) = tokio::fs::create_dir_all(folder).await {
        eprintln!("Failed to create data folder {}: {err}", folder.display());
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::browser_url_for;

    #[test]
    fn browser_url_preserves_specific_bind_addresses() {
        let addr = "127.0.0.1:4444".parse().expect("socket addr");
        assert_eq!(browser_url_for(addr), "http://127.0.0.1:4444/");
    }

    #[test]
    fn browser_url_maps_unspecified_ipv4_to_loopback() {
        let addr = "0.0.0.0:4444".parse().expect("socket addr");
        assert_eq!(browser_url_for(addr), "http://127.0.0.1:4444/");
    }

    #[test]
    fn browser_url_maps_unspecified_ipv6_to_loopback() {
        let addr = "[::]:4444".parse().expect("socket addr");
        assert_eq!(browser_url_for(addr), "http://[::1]:4444/");
    }
}
