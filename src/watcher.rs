use std::{
    collections::{BTreeSet, HashMap},
    fs,
    sync::Arc,
};

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::Value;
use tokio::sync::RwLock;

use crate::{
    app::{CachedMetadata, CachedResource, DataSource},
    storage::{is_valid_resource_name, scan_resources},
};

pub fn start_resource_watcher(
    data_source: Arc<DataSource>,
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

        let watch_path = match &*data_source {
            DataSource::Folder(folder) => folder.clone(),
            DataSource::File(file) => file.clone(),
        };

        if let Err(err) = watcher.watch(&watch_path, RecursiveMode::NonRecursive) {
            tracing::error!("Failed to watch path {}: {err}", watch_path.display());
            return;
        }

        for event in rx {
            match event {
                Ok(event) => match scan_resources(&data_source) {
                    Ok(new_resources) => {
                        {
                            let mut cache = resources.blocking_write();
                            *cache = new_resources;
                        }
                        {
                            let mut cache = resource_cache.blocking_write();
                            match &*data_source {
                                DataSource::Folder(_) => {
                                    for path in &event.paths {
                                        let is_json = path.extension().and_then(|ext| ext.to_str())
                                            == Some("json");
                                        if !is_json {
                                            continue;
                                        }
                                        let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                                        else {
                                            continue;
                                        };
                                        if !is_valid_resource_name(stem) {
                                            continue;
                                        }

                                        if path.exists() {
                                            match fs::read_to_string(path).ok().and_then(|raw| {
                                                serde_json::from_str::<Value>(&raw).ok()
                                            }) {
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
                                }
                                DataSource::File(_) => {
                                    cache.clear();
                                }
                            }
                        }
                    }
                    Err(err) => {
                        tracing::error!(
                            "Failed to refresh resources for {}: {err}",
                            watch_path.display()
                        )
                    }
                },
                Err(err) => tracing::warn!("File watch event error: {err}"),
            }
        }
    });
}
