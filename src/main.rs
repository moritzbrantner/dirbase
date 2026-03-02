use std::{
    collections::HashMap,
    fs,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock},
};

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
#[command(
    author,
    version,
    about = "Serve all JSON files in a folder as a REST API"
)]
struct Cli {
    #[arg(short, long, default_value = "./data")]
    folder: PathBuf,
    #[arg(short, long, default_value = "127.0.0.1:3000")]
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

    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    if let Err(err) = fs::create_dir_all(&cli.folder) {
        eprintln!(
            "Failed to create data folder {}: {err}",
            cli.folder.display()
        );
        std::process::exit(1);
    }

    let schema = match load_schema(&cli.folder, cli.schema.as_deref()) {
        Ok(schema) => schema,
        Err(err) => {
            eprintln!("Failed to load schema: {err}");
            std::process::exit(1);
        }
    };

    let initial_resources = scan_resources(&cli.folder).unwrap_or_default();
    let state = AppState {
        folder: Arc::new(cli.folder),
        resources: Arc::new(StdRwLock::new(initial_resources)),
        resource_cache: Arc::new(StdRwLock::new(HashMap::new())),
        resource_locks: Arc::new(RwLock::new(HashMap::new())),
        schema: Arc::new(schema),
        request_log: if cli.log {
            match fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&cli.logname)
            {
                Ok(file) => Some(Arc::new(StdMutex::new(file))),
                Err(err) => {
                    eprintln!("Failed to open log file {}: {err}", cli.logname.display());
                    std::process::exit(1);
                }
            }
        } else {
            None
        },
    };

    start_resource_watcher(
        state.folder.clone(),
        state.resources.clone(),
        state.resource_cache.clone(),
    );

    let app = build_router(state.clone(), cli.readonly, cli.log);
    tracing::info!(readonly = cli.readonly, "Readonly mode");
    tracing::info!("Listening on http://{}", cli.bind);
    let listener = tokio::net::TcpListener::bind(cli.bind)
        .await
        .expect("binding server listener");
    axum::serve(listener, app).await.expect("running server");
}
