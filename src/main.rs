use std::{
    fs,
    sync::{Arc, RwLock},
};

use axum::{Router, routing::get};
use clap::Parser;
use tokio::sync::Mutex;

mod cli;
mod error;
mod handlers;
mod resources;
mod state;
mod watcher;

use cli::Cli;
use handlers::{
    create_item, delete_item, get_collection, get_item, list_resources, patch_item, replace_item,
};
use resources::scan_resources;
use state::AppState;
use watcher::start_resource_watcher;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();
    if let Err(err) = fs::create_dir_all(&cli.folder) {
        eprintln!(
            "Failed to create data folder {}: {err}",
            cli.folder.display()
        );
        std::process::exit(1);
    }

    let initial_resources = scan_resources(&cli.folder).unwrap_or_else(|err| {
        eprintln!("Failed to scan data folder {}: {err}", cli.folder.display());
        std::collections::BTreeSet::new()
    });

    let state = AppState {
        folder: Arc::new(cli.folder),
        resources: Arc::new(RwLock::new(initial_resources)),
        io_lock: Arc::new(Mutex::new(())),
    };

    start_resource_watcher(state.folder.clone(), state.resources.clone());

    let app = if cli.readonly {
        Router::new()
            .route("/", get(list_resources))
            .route("/{resource}", get(get_collection))
            .route("/{resource}/{id}", get(get_item))
            .with_state(state)
    } else {
        Router::new()
            .route("/", get(list_resources))
            .route("/{resource}", get(get_collection).post(create_item))
            .route(
                "/{resource}/{id}",
                get(get_item)
                    .put(replace_item)
                    .patch(patch_item)
                    .delete(delete_item),
            )
            .with_state(state)
    };

    tracing::info!(readonly = cli.readonly, "Readonly mode");
    tracing::info!("Listening on http://{}", cli.bind);
    let listener = tokio::net::TcpListener::bind(cli.bind)
        .await
        .expect("binding server listener");
    axum::serve(listener, app).await.expect("running server");
}
