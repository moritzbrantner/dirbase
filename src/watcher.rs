use std::{
    collections::{BTreeSet, HashMap},
    fs,
    sync::{Arc, RwLock as StdRwLock},
};

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::Value;
use tokio::sync::RwLock;

use crate::{
    app::{CachedResource, DataSource, GraphqlStore, SchemaStore},
    schema::{infer_schema_from_data_source, primary_key_name},
    storage::{build_id_index, is_reserved_resource_name, is_valid_resource_name, scan_resources},
};

pub fn start_resource_watcher(
    data_source: Arc<DataSource>,
    resources: Arc<RwLock<BTreeSet<String>>>,
    resource_cache: Arc<RwLock<HashMap<String, CachedResource>>>,
    schema_store: Arc<StdRwLock<SchemaStore>>,
    graphql_store: Arc<RwLock<GraphqlStore>>,
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
                            *cache = new_resources.clone();
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
                                        if !is_valid_resource_name(stem)
                                            || is_reserved_resource_name(stem)
                                        {
                                            continue;
                                        }

                                        if path.exists() {
                                            match fs::read_to_string(path).ok().and_then(|raw| {
                                                serde_json::from_str::<Value>(&raw).ok()
                                            }) {
                                                Some(value) => {
                                                    let table = schema_store
                                                        .read()
                                                        .expect("schema store")
                                                        .merged
                                                        .tables
                                                        .get(stem)
                                                        .cloned();
                                                    cache.insert(
                                                        stem.to_string(),
                                                        CachedResource {
                                                            id_index: build_id_index(
                                                                &value,
                                                                table.as_ref(),
                                                            ),
                                                            primary_key: primary_key_name(
                                                                table.as_ref(),
                                                            )
                                                            .to_string(),
                                                            value: Arc::new(value),
                                                        },
                                                    );
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
                        match infer_schema_from_data_source(&data_source, &new_resources) {
                            Ok(schema) => {
                                if let Err(err) = schema_store
                                    .write()
                                    .expect("schema store")
                                    .replace_inferred(schema)
                                {
                                    tracing::error!(
                                        "Failed to merge schema for {}: {err}",
                                        watch_path.display()
                                    );
                                } else {
                                    *graphql_store.blocking_write() = GraphqlStore::default();
                                }
                            }
                            Err(err) => tracing::error!(
                                "Failed to infer schema for {}: {err}",
                                watch_path.display()
                            ),
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
