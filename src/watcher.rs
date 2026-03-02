use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::PathBuf,
    sync::Arc,
};

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::Value;
use tokio::sync::RwLock;

use crate::{
    app::{CachedMetadata, CachedResource},
    storage::{is_valid_resource_name, scan_resources},
};

pub fn start_resource_watcher(
    folder: Arc<PathBuf>,
    resources: Arc<RwLock<BTreeSet<String>>>,
    resource_cache: Arc<RwLock<HashMap<String, CachedResource>>>,
) {
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = match RecommendedWatcher::new(
            move |result| {
                let _ = tx.send(result);
            },
            Config::default(),
        ) {
            Ok(watcher) => watcher,
            Err(err) => {
                tracing::error!("Failed to create filesystem watcher: {err}");
                return;
            }
        };

        if let Err(err) = watcher.watch(&folder, RecursiveMode::NonRecursive) {
            tracing::error!("Failed to watch folder {}: {err}", folder.display());
            return;
        }

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("watcher runtime");

        for event in rx {
            match event {
                Ok(event) => match scan_resources(&folder) {
                    Ok(new_resources) => {
                        runtime.block_on(async {
                            *resources.write().await = new_resources;
                            let mut cache = resource_cache.write().await;
                            for path in &event.paths {
                                let is_json =
                                    path.extension().and_then(|ext| ext.to_str()) == Some("json");
                                if !is_json {
                                    continue;
                                }
                                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                                    continue;
                                };
                                if !is_valid_resource_name(stem) {
                                    continue;
                                }

                                if path.exists() {
                                    match fs::read_to_string(path)
                                        .ok()
                                        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                                    {
                                        Some(value) => {
                                            if let Ok(metadata) = fs::metadata(path) {
                                                if let Ok(modified) = metadata.modified() {
                                                    cache.insert(
                                                        stem.to_string(),
                                                        CachedResource {
                                                            value: Arc::new(value),
                                                            metadata: CachedMetadata {
                                                                modified,
                                                                len: metadata.len(),
                                                            },
                                                        },
                                                    );
                                                } else {
                                                    cache.remove(stem);
                                                }
                                            } else {
                                                cache.remove(stem);
                                            }
                                        }
                                        None => {
                                            cache.remove(stem);
                                        }
                                    }
                                } else {
                                    cache.remove(stem);
                                }
                            }
                        });
                    }
                    Err(err) => tracing::error!(
                        "Failed to refresh resources for folder {}: {err}",
                        folder.display()
                    ),
                },
                Err(err) => tracing::warn!("File watch event error: {err}"),
            }
        }
    });
}
