use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{Arc, RwLock as StdRwLock},
};

use notify::{
    Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher,
    event::{MetadataKind, ModifyKind},
};
use serde_json::Value;
use tokio::sync::RwLock;

use crate::{
    app::{AppState, CachedResource, DataSource, GraphqlStore, HealthState, SchemaStore},
    schema::{infer_schema_from_data_source, load_schema, primary_key_name},
    storage::{build_id_index, is_reserved_resource_name, is_valid_resource_name, scan_resources},
};

pub fn start_resource_watcher(
    data_source: Arc<DataSource>,
    resources: Arc<RwLock<BTreeSet<String>>>,
    resource_cache: Arc<RwLock<HashMap<String, CachedResource>>>,
    schema_store: Arc<StdRwLock<SchemaStore>>,
    graphql_store: Arc<RwLock<GraphqlStore>>,
    health: Arc<HealthState>,
    app_state: AppState,
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
        let schema_root = match &*data_source {
            DataSource::Folder(folder) => folder.clone(),
            DataSource::File(file) => file
                .parent()
                .map(|parent| parent.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from(".")),
        };

        if let Err(err) = watcher.watch(&watch_path, RecursiveMode::NonRecursive) {
            tracing::error!("Failed to watch path {}: {err}", watch_path.display());
            return;
        }

        for event in rx {
            match event {
                Ok(event) => {
                    if !should_process_watch_event(&data_source, &event) {
                        continue;
                    }

                    match scan_resources(&data_source) {
                        Ok(new_resources) => {
                            match load_schema(&schema_root, None) {
                                Ok(declared) => {
                                    if let Err(err) = schema_store
                                        .write()
                                        .expect("schema store")
                                        .replace_declared(declared)
                                    {
                                        health.mark_not_ready(err.clone());
                                        tracing::error!(
                                            "Failed to apply declared schema for {}: {err}",
                                            schema_root.display()
                                        );
                                        continue;
                                    }
                                }
                                Err(err) => {
                                    health.mark_not_ready(err.clone());
                                    tracing::error!(
                                        "Failed to load schema for {}: {err}",
                                        schema_root.display()
                                    );
                                    continue;
                                }
                            }
                            {
                                let mut cache = resources.blocking_write();
                                *cache = new_resources.clone();
                            }
                            {
                                let mut cache = resource_cache.blocking_write();
                                match &*data_source {
                                    DataSource::Folder(_) => {
                                        for path in &event.paths {
                                            let is_json =
                                                path.extension().and_then(|ext| ext.to_str())
                                                    == Some("json");
                                            if !is_json {
                                                continue;
                                            }
                                            let Some(stem) =
                                                path.file_stem().and_then(|s| s.to_str())
                                            else {
                                                continue;
                                            };
                                            if !is_valid_resource_name(stem)
                                                || is_reserved_resource_name(stem)
                                            {
                                                continue;
                                            }

                                            if path.exists() {
                                                match fs::read_to_string(path).ok().and_then(
                                                    |raw| serde_json::from_str::<Value>(&raw).ok(),
                                                ) {
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
                                        health.mark_not_ready(err);
                                    } else {
                                        *graphql_store.blocking_write() = GraphqlStore::default();
                                        health.mark_ready();
                                        app_state.emit_event("schema_changed", None);
                                    }
                                }
                                Err(err) => {
                                    health.mark_not_ready(err.clone());
                                    tracing::error!(
                                        "Failed to infer schema for {}: {err}",
                                        watch_path.display()
                                    );
                                }
                            }
                            app_state.emit_event("overview_changed", None);
                            for resource in changed_resource_names(&event.paths) {
                                app_state.emit_event("resource_changed", Some(resource));
                            }
                        }
                        Err(err) => {
                            health.mark_not_ready(err.to_string());
                            tracing::error!(
                                "Failed to refresh resources for {}: {err}",
                                watch_path.display()
                            )
                        }
                    }
                }
                Err(err) => tracing::warn!("File watch event error: {err}"),
            }
        }
    });
}

fn should_process_watch_event(data_source: &DataSource, event: &Event) -> bool {
    if event.paths.is_empty() || !event_touches_relevant_path(data_source, &event.paths) {
        return false;
    }

    !matches!(
        event.kind,
        EventKind::Access(_) | EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime))
    )
}

fn event_touches_relevant_path(data_source: &DataSource, paths: &[PathBuf]) -> bool {
    paths.iter().any(|path| is_relevant_watch_path(data_source, path))
}

fn is_relevant_watch_path(data_source: &DataSource, path: &Path) -> bool {
    match data_source {
        DataSource::Folder(root) => is_relevant_folder_watch_path(root, path),
        DataSource::File(file) => path == file,
    }
}

fn is_relevant_folder_watch_path(root: &Path, path: &Path) -> bool {
    if path == root {
        return false;
    }

    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if matches!(file_name, "schema.json" | "schema.xsd" | "schema.dbml") {
        return true;
    }

    if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
        return false;
    }

    let Some(stem) = path.file_stem().and_then(|name| name.to_str()) else {
        return false;
    };
    is_valid_resource_name(stem) && !is_reserved_resource_name(stem)
}

fn changed_resource_names(paths: &[PathBuf]) -> BTreeSet<String> {
    paths
        .iter()
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .filter_map(|path| path.file_stem().and_then(|stem| stem.to_str()))
        .filter(|stem| is_valid_resource_name(stem) && !is_reserved_resource_name(stem))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::{
        EventKind,
        event::{AccessKind, DataChange, ModifyKind},
    };

    #[test]
    fn watcher_ignores_read_only_access_events_for_resources() {
        let data_source = DataSource::Folder(PathBuf::from("/tmp/data"));
        let event = Event {
            kind: EventKind::Access(AccessKind::Read),
            paths: vec![PathBuf::from("/tmp/data/users.json")],
            attrs: Default::default(),
        };

        assert!(!should_process_watch_event(&data_source, &event));
    }

    #[test]
    fn watcher_ignores_access_time_metadata_updates() {
        let data_source = DataSource::Folder(PathBuf::from("/tmp/data"));
        let event = Event {
            kind: EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime)),
            paths: vec![PathBuf::from("/tmp/data/users.json")],
            attrs: Default::default(),
        };

        assert!(!should_process_watch_event(&data_source, &event));
    }

    #[test]
    fn watcher_ignores_temp_files_in_folder_mode() {
        let data_source = DataSource::Folder(PathBuf::from("/tmp/data"));
        let event = Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec![PathBuf::from("/tmp/data/users.json.tmp.save")],
            attrs: Default::default(),
        };

        assert!(!should_process_watch_event(&data_source, &event));
    }

    #[test]
    fn watcher_processes_data_changes_for_resources() {
        let data_source = DataSource::Folder(PathBuf::from("/tmp/data"));
        let event = Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec![PathBuf::from("/tmp/data/users.json")],
            attrs: Default::default(),
        };

        assert!(should_process_watch_event(&data_source, &event));
    }

    #[test]
    fn watcher_processes_data_changes_for_schema_xsd() {
        let data_source = DataSource::Folder(PathBuf::from("/tmp/data"));
        let event = Event {
            kind: EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            paths: vec![PathBuf::from("/tmp/data/schema.xsd")],
            attrs: Default::default(),
        };

        assert!(should_process_watch_event(&data_source, &event));
    }

    #[test]
    fn watcher_collects_changed_resource_names_once_per_resource() {
        let changed = changed_resource_names(&[
            PathBuf::from("/tmp/data/users.json"),
            PathBuf::from("/tmp/data/users.json"),
            PathBuf::from("/tmp/data/schema.json"),
            PathBuf::from("/tmp/data/metrics.json"),
            PathBuf::from("/tmp/data/posts.json"),
        ]);

        assert_eq!(changed, BTreeSet::from(["posts".to_string(), "users".to_string()]));
    }
}
