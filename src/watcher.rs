use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, RwLock},
};

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};

use crate::resources::scan_resources;

pub fn start_resource_watcher(folder: Arc<PathBuf>, resources: Arc<RwLock<BTreeSet<String>>>) {
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

        for event in rx {
            match event {
                Ok(_) => match scan_resources(&folder) {
                    Ok(new_resources) => {
                        if let Ok(mut cache) = resources.write() {
                            *cache = new_resources;
                        }
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
