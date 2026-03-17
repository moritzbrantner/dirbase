use std::{collections::HashMap, net::SocketAddr, path::PathBuf, sync::Arc};

use app::AppState;
use clap::{CommandFactory, Parser};
use tokio::sync::RwLock;

mod app;
mod error;
mod http;
mod query;
mod schema;
mod sql;
mod storage;
mod watcher;

use http::routes::build_router;
use schema::load_schema;
use storage::scan_resources;
use watcher::start_resource_watcher;

#[derive(Parser, Debug)]
#[command(author, version, about = "Serve all JSON files in a folder as a REST API")]
struct Cli {
    #[arg(short, long, conflicts_with = "file")]
    folder: Option<PathBuf>,
    #[arg(long, conflicts_with = "folder")]
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

    let data_source = if let Some(file) = cli.file.clone() {
        if let Err(err) = tokio::fs::try_exists(&file).await {
            eprintln!("Failed to inspect data file {}: {err}", file.display());
            std::process::exit(1);
        }
        app::DataSource::File(file)
    } else {
        let folder = cli.folder.clone().unwrap_or_else(|| PathBuf::from("./data"));
        if let Err(err) = tokio::fs::create_dir_all(&folder).await {
            eprintln!("Failed to create data folder {}: {err}", folder.display());
            std::process::exit(1);
        }
        app::DataSource::Folder(folder)
    };

    let schema_root = match &data_source {
        app::DataSource::Folder(folder) => folder.clone(),
        app::DataSource::File(file) => {
            file.parent().map(|parent| parent.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
        }
    };

    let schema = match load_schema(&schema_root, cli.schema.as_deref()) {
        Ok(schema) => schema,
        Err(err) => {
            eprintln!("Failed to load schema: {err}");
            std::process::exit(1);
        }
    };

    let initial_resources = scan_resources(&data_source).unwrap_or_default();
    let state = AppState {
        data_source: Arc::new(data_source),
        resources: Arc::new(RwLock::new(initial_resources)),
        resource_cache: Arc::new(RwLock::new(HashMap::new())),
        resource_locks: Arc::new(RwLock::new(HashMap::new())),
        schema: Arc::new(schema),
    };

    start_resource_watcher(
        state.data_source.clone(),
        state.resources.clone(),
        state.resource_cache.clone(),
    );

    let app = build_router(state.clone(), cli.readonly, cli.log);
    tracing::info!(readonly = cli.readonly, "Readonly mode");
    tracing::info!("Listening on http://{}", cli.bind);
    let listener = tokio::net::TcpListener::bind(cli.bind).await.expect("binding server listener");
    axum::serve(listener, app).await.expect("running server");
}
