use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
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
    schema::{
        infer_schema_from_data_source, infer_table_from_value, load_schema, replace_inferred_tables,
    },
    storage::{
        cached_resource_from_value, is_reserved_resource_name, is_valid_resource_name,
        scan_resources,
    },
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

                    let changed_resources = changed_resource_names(&event.paths);
                    let schema_definition_changed =
                        event_touches_schema_definition(&data_source, &event.paths);

                    match scan_resources(&data_source) {
                        Ok(new_resources) => {
                            let previous_resources = resources.blocking_read().clone();

                            if schema_definition_changed {
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
                            }

                            {
                                let mut current = resources.blocking_write();
                                *current = new_resources.clone();
                            }

                            let changed_values = match &*data_source {
                                DataSource::Folder(_) => {
                                    match read_changed_folder_resources(&event.paths) {
                                        Ok(values) => values,
                                        Err(err) => {
                                            let mut cache = resource_cache.blocking_write();
                                            for resource in &changed_resources {
                                                cache.remove(resource);
                                            }
                                            drop(cache);
                                            health.mark_not_ready(err.clone());
                                            tracing::error!(
                                                "Failed to refresh changed resource for {}: {err}",
                                                watch_path.display()
                                            );
                                            app_state.emit_event("overview_changed", None);
                                            for resource in changed_resources {
                                                app_state
                                                    .emit_event("resource_changed", Some(resource));
                                            }
                                            continue;
                                        }
                                    }
                                }
                                DataSource::File(_) => BTreeMap::new(),
                            };

                            let full_inference_required = watcher_requires_full_inference(
                                &data_source,
                                &previous_resources,
                                &new_resources,
                                health.is_ready(),
                            );

                            if full_inference_required {
                                let schema = match infer_schema_from_data_source(
                                    &data_source,
                                    &new_resources,
                                ) {
                                    Ok(schema) => schema,
                                    Err(err) => {
                                        health.mark_not_ready(err.clone());
                                        tracing::error!(
                                            "Failed to infer schema for {}: {err}",
                                            watch_path.display()
                                        );
                                        continue;
                                    }
                                };
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
                                    continue;
                                }
                            } else if !changed_values.is_empty() {
                                // Row scanning happens on this dedicated watcher thread before the
                                // schema write lock is taken. Installation then starts from the
                                // latest inferred schema so concurrent API writes cannot be lost.
                                let replacements = changed_values
                                    .iter()
                                    .map(|(resource, value)| {
                                        (resource.clone(), infer_table_from_value(resource, value))
                                    })
                                    .collect::<BTreeMap<_, _>>();
                                let mut store = schema_store.write().expect("schema store");
                                let inferred =
                                    replace_inferred_tables(store.inferred.clone(), replacements);
                                if let Err(err) = store.replace_inferred(inferred) {
                                    tracing::error!(
                                        "Failed to merge schema for {}: {err}",
                                        watch_path.display()
                                    );
                                    health.mark_not_ready(err);
                                    continue;
                                }
                            }

                            {
                                let mut cache = resource_cache.blocking_write();
                                match &*data_source {
                                    DataSource::Folder(_) => {
                                        for resource in &changed_resources {
                                            if let Some(value) = changed_values.get(resource) {
                                                let table = schema_store
                                                    .read()
                                                    .expect("schema store")
                                                    .merged
                                                    .tables
                                                    .get(resource)
                                                    .cloned();
                                                cache.insert(
                                                    resource.clone(),
                                                    cached_resource_from_value(
                                                        Arc::new(value.clone()),
                                                        table.as_ref(),
                                                    ),
                                                );
                                            } else {
                                                cache.remove(resource);
                                            }
                                        }
                                    }
                                    DataSource::File(_) => {
                                        cache.clear();
                                    }
                                }
                            }

                            *graphql_store.blocking_write() = GraphqlStore::default();
                            if full_inference_required {
                                health.mark_ready();
                            }
                            app_state.emit_event("schema_changed", None);
                            app_state.emit_event("overview_changed", None);
                            for resource in changed_resources {
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

fn event_touches_schema_definition(data_source: &DataSource, paths: &[PathBuf]) -> bool {
    matches!(data_source, DataSource::Folder(_))
        && paths.iter().any(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| matches!(name, "schema.json" | "schema.xsd" | "schema.dbml"))
        })
}

fn watcher_requires_full_inference(
    data_source: &DataSource,
    previous_resources: &BTreeSet<String>,
    new_resources: &BTreeSet<String>,
    currently_ready: bool,
) -> bool {
    !currently_ready
        || matches!(data_source, DataSource::File(_))
        || previous_resources != new_resources
}

fn read_changed_folder_resources(paths: &[PathBuf]) -> Result<BTreeMap<String, Value>, String> {
    let mut values = BTreeMap::new();
    for path in paths {
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") || !path.exists() {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        if !is_valid_resource_name(stem) || is_reserved_resource_name(stem) {
            continue;
        }
        let raw = fs::read_to_string(path).map_err(|err| format!("{}: {err}", path.display()))?;
        let value: Value = serde_json::from_str(&raw)
            .map_err(|err| format!("{}: invalid json: {err}", path.display()))?;
        values.insert(stem.to_string(), value);
    }
    Ok(values)
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
        assert!(event_touches_schema_definition(&data_source, &event.paths));
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

    #[test]
    fn watcher_localizes_existing_folder_resource_content_changes_when_ready() {
        let data_source = DataSource::Folder(PathBuf::from("/tmp/data"));
        let resources = BTreeSet::from(["posts".to_string(), "users".to_string()]);

        assert!(!watcher_requires_full_inference(&data_source, &resources, &resources, true,));
    }

    #[test]
    fn watcher_full_refreshes_when_folder_resource_set_changes() {
        let data_source = DataSource::Folder(PathBuf::from("/tmp/data"));
        let previous = BTreeSet::from(["users".to_string()]);
        let next = BTreeSet::from(["posts".to_string(), "users".to_string()]);

        assert!(watcher_requires_full_inference(&data_source, &previous, &next, true,));
    }

    #[test]
    fn watcher_full_refreshes_single_database_file_changes() {
        let data_source = DataSource::File(PathBuf::from("/tmp/db.json"));
        let resources = BTreeSet::from(["users".to_string()]);

        assert!(watcher_requires_full_inference(&data_source, &resources, &resources, true,));
    }

    #[test]
    fn watcher_full_refreshes_before_recovering_not_ready_state() {
        let data_source = DataSource::Folder(PathBuf::from("/tmp/data"));
        let resources = BTreeSet::from(["posts".to_string(), "users".to_string()]);

        assert!(watcher_requires_full_inference(&data_source, &resources, &resources, false,));
    }
}
