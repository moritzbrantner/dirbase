use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, RwLock},
};

use tokio::sync::Mutex;

#[derive(Clone)]
pub struct AppState {
    pub folder: Arc<PathBuf>,
    pub resources: Arc<RwLock<BTreeSet<String>>>,
    pub io_lock: Arc<Mutex<()>>,
}
