use std::{
    collections::BTreeSet,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::http::StatusCode;
use fs2::FileExt;
use serde_json::Value;
use tokio::fs;

use crate::{app::DataSource, error::AppError};

pub async fn read_resource_value(
    data_source: &DataSource,
    file: &Path,
    resource: &str,
) -> Result<Value, AppError> {
    match data_source {
        DataSource::Folder(_) => {
            let raw = fs::read_to_string(file).await.map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => {
                    AppError::new(StatusCode::NOT_FOUND, format!("Resource '{resource}' not found"))
                }
                _ => AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            })?;
            serde_json::from_str::<Value>(&raw).map_err(|e| {
                AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Invalid JSON: {e}"))
            })
        }
        DataSource::File(_) => {
            let raw = fs::read_to_string(file).await.map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => {
                    AppError::new(StatusCode::NOT_FOUND, format!("Resource '{resource}' not found"))
                }
                _ => AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            })?;
            let root: Value = serde_json::from_str(&raw).map_err(|e| {
                AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Invalid JSON: {e}"))
            })?;
            root.as_object().and_then(|obj| obj.get(resource).cloned()).ok_or_else(|| {
                AppError::new(StatusCode::NOT_FOUND, format!("Resource '{resource}' not found"))
            })
        }
    }
}

pub async fn persist_resource_value(
    data_source: &DataSource,
    file: &Path,
    resource: &str,
    value: &Value,
) -> Result<(), AppError> {
    let file_for_write = file.to_path_buf();
    let resource_name = resource.to_string();
    let value_for_write = value.clone();

    match data_source {
        DataSource::Folder(_) => {
            let content = serde_json::to_string_pretty(value)
                .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let payload = format!("{content}\n");
            tokio::task::spawn_blocking(move || write_json_atomically(&file_for_write, payload))
                .await
                .map_err(|e| {
                    AppError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Atomic write task failed: {e}"),
                    )
                })?
        }
        DataSource::File(_) => tokio::task::spawn_blocking(move || {
            write_resource_in_db_file(&file_for_write, &resource_name, &value_for_write)
        })
        .await
        .map_err(|e| {
            AppError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Atomic write task failed: {e}"),
            )
        })?,
    }
}

pub fn resource_file_path(data_source: &DataSource, resource: &str) -> Result<PathBuf, AppError> {
    if !is_valid_resource_name(resource) {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Resource name must only contain letters, numbers, underscore, and dash",
        ));
    }
    Ok(match data_source {
        DataSource::Folder(folder) => folder.join(format!("{resource}.json")),
        DataSource::File(file) => file.clone(),
    })
}

pub fn is_valid_resource_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn is_reserved_resource_name(name: &str) -> bool {
    matches!(name, "schema" | "graphql" | "sql" | "events" | "healthz" | "readyz" | "metrics")
}

pub fn scan_resources(data_source: &DataSource) -> Result<BTreeSet<String>, std::io::Error> {
    match data_source {
        DataSource::Folder(folder) => scan_resources_folder(folder),
        DataSource::File(file) => scan_resources_file(file),
    }
}

fn scan_resources_folder(folder: &Path) -> Result<BTreeSet<String>, std::io::Error> {
    let mut resources = BTreeSet::new();
    let entries = std::fs::read_dir(folder)?;
    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if is_valid_resource_name(stem) && !is_reserved_resource_name(stem) {
            resources.insert(stem.to_owned());
        }
    }
    Ok(resources)
}

fn scan_resources_file(file: &Path) -> Result<BTreeSet<String>, std::io::Error> {
    let mut resources = BTreeSet::new();
    if !file.exists() {
        return Ok(resources);
    }
    let raw = std::fs::read_to_string(file)?;
    let parsed: Value = serde_json::from_str(&raw).map_err(std::io::Error::other)?;
    if let Some(root) = parsed.as_object() {
        for key in root.keys() {
            if is_valid_resource_name(key) && !is_reserved_resource_name(key) {
                resources.insert(key.to_string());
            }
        }
    }
    Ok(resources)
}

fn write_json_atomically(file: &Path, payload: String) -> Result<(), AppError> {
    with_exclusive_file_lock(file, || write_json_atomically_unlocked(file, &payload))
}

fn write_json_atomically_unlocked(file: &Path, payload: &str) -> Result<(), AppError> {
    let temp_file = temp_file_path(file);
    let mut handle = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp_file)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    handle
        .write_all(payload.as_bytes())
        .and_then(|_| handle.flush())
        .and_then(|_| handle.sync_all())
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    std::fs::rename(&temp_file, file)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(parent) = file.parent() {
        sync_parent_dir(parent)
            .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    Ok(())
}

fn write_resource_in_db_file(file: &Path, resource: &str, value: &Value) -> Result<(), AppError> {
    with_exclusive_file_lock(file, || {
        let raw = std::fs::read_to_string(file)
            .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let mut root: Value = serde_json::from_str(&raw).map_err(|e| {
            AppError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("Invalid JSON: {e}"))
        })?;
        let root_obj = root.as_object_mut().ok_or_else(|| {
            AppError::new(StatusCode::BAD_REQUEST, "Database file must contain a JSON object")
        })?;
        root_obj.insert(resource.to_string(), value.clone());
        let content = serde_json::to_string_pretty(&root)
            .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        write_json_atomically_unlocked(file, &format!("{content}\n"))
    })
}

fn with_exclusive_file_lock<T>(
    file: &Path,
    f: impl FnOnce() -> Result<T, AppError>,
) -> Result<T, AppError> {
    let lock_path = lock_file_path(file);
    let lock_handle = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    lock_handle
        .lock_exclusive()
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let result = f();
    let _ = lock_handle.unlock();
    result
}

fn lock_file_path(file: &Path) -> PathBuf {
    let file_name = file.file_name().and_then(|name| name.to_str()).unwrap_or("resource");
    file.with_file_name(format!("{file_name}.lock"))
}

fn temp_file_path(file: &Path) -> PathBuf {
    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    let file_name = file.file_name().and_then(|name| name.to_str()).unwrap_or("resource.json");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let unique_id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    file.with_file_name(format!("{file_name}.tmp.{now}-{unique_id}"))
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path) -> std::io::Result<()> {
    std::fs::File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use axum::http::StatusCode;
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn read_resource_value_reads_folder_resource() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("users.json");
        std::fs::write(&path, "[{\"id\":1}]\n").expect("write users");

        let value =
            read_resource_value(&DataSource::Folder(temp.path().to_path_buf()), &path, "users")
                .await
                .expect("read resource");
        assert_eq!(value, json!([{"id": 1}]));
    }

    #[tokio::test]
    async fn read_resource_value_returns_not_found_for_missing_folder_resource() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("users.json");

        let err =
            read_resource_value(&DataSource::Folder(temp.path().to_path_buf()), &path, "users")
                .await
                .expect_err("missing");
        assert_eq!(err.status, StatusCode::NOT_FOUND);
        assert!(err.message.contains("Resource 'users' not found"));
    }

    #[tokio::test]
    async fn read_resource_value_returns_invalid_json_error_for_bad_folder_json() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("users.json");
        std::fs::write(&path, "[{\"id\":").expect("write invalid json");

        let err =
            read_resource_value(&DataSource::Folder(temp.path().to_path_buf()), &path, "users")
                .await
                .expect_err("invalid");
        assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(err.message.contains("Invalid JSON"));
    }

    #[tokio::test]
    async fn read_resource_value_reads_file_mode_top_level_key() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("db.json");
        std::fs::write(&path, "{\"users\":[{\"id\":1}],\"posts\":[]}\n").expect("write db");

        let value = read_resource_value(&DataSource::File(path.clone()), &path, "users")
            .await
            .expect("read resource");
        assert_eq!(value, json!([{"id": 1}]));
    }

    #[tokio::test]
    async fn read_resource_value_returns_not_found_for_missing_file_mode_key() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("db.json");
        std::fs::write(&path, "{\"posts\":[]}\n").expect("write db");

        let err = read_resource_value(&DataSource::File(path.clone()), &path, "users")
            .await
            .expect_err("missing");
        assert_eq!(err.status, StatusCode::NOT_FOUND);
        assert!(err.message.contains("Resource 'users' not found"));
    }

    #[tokio::test]
    async fn persist_resource_value_folder_mode_writes_pretty_json() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("users.json");

        persist_resource_value(
            &DataSource::Folder(temp.path().to_path_buf()),
            &path,
            "users",
            &json!([{"id": 1, "name": "Ada"}]),
        )
        .await
        .expect("persist");

        let raw = std::fs::read_to_string(&path).expect("read file");
        assert!(raw.ends_with('\n'));
        assert!(raw.contains("\n  {\n"));
    }

    #[tokio::test]
    async fn persist_resource_value_file_mode_preserves_sibling_keys() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("db.json");
        std::fs::write(&path, "{\"users\":[{\"id\":1}],\"posts\":[{\"id\":10}]}\n")
            .expect("write db");

        persist_resource_value(
            &DataSource::File(path.clone()),
            &path,
            "users",
            &json!([{"id": 2, "name": "Grace"}]),
        )
        .await
        .expect("persist");

        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read file"))
                .expect("json");
        assert_eq!(parsed["users"], json!([{"id": 2, "name": "Grace"}]));
        assert_eq!(parsed["posts"], json!([{"id": 10}]));
    }

    #[test]
    fn scan_resources_folder_ignores_non_json_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("users.json"), "[]\n").expect("write users");
        std::fs::write(temp.path().join("notes.txt"), "ignore").expect("write notes");

        let resources = scan_resources_folder(temp.path()).expect("scan");
        assert_eq!(resources, BTreeSet::from(["users".to_string()]));
    }

    #[test]
    fn scan_resources_folder_ignores_reserved_resource_names() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("users.json"), "[]\n").expect("write users");
        std::fs::write(temp.path().join("schema.json"), "{}\n").expect("write schema");
        std::fs::write(temp.path().join("metrics.json"), "{}\n").expect("write metrics");

        let resources = scan_resources_folder(temp.path()).expect("scan");
        assert_eq!(resources, BTreeSet::from(["users".to_string()]));
    }

    #[test]
    fn scan_resources_file_ignores_reserved_and_invalid_keys() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("db.json");
        std::fs::write(
            &path,
            "{\"users\":[],\"schema\":{},\"bad key\":[],\"readyz\":{},\"teams\":[]}\n",
        )
        .expect("write db");

        let resources = scan_resources_file(&path).expect("scan");
        assert_eq!(resources, BTreeSet::from(["teams".to_string(), "users".to_string()]));
    }

    #[test]
    fn resource_file_path_rejects_invalid_resource_name() {
        let err = resource_file_path(&DataSource::Folder(PathBuf::from(".")), "bad name")
            .expect_err("invalid");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(
            err.message
                .contains("Resource name must only contain letters, numbers, underscore, and dash")
        );
    }
}
