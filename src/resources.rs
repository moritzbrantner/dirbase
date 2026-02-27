use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use axum::http::StatusCode;
use serde_json::Value;

use crate::error::AppError;

pub fn load_resource(folder: &Path, resource: &str) -> Result<Value, AppError> {
    let file = resource_file_path(folder, resource)?;
    if !file.exists() {
        return Err(AppError::new(
            StatusCode::NOT_FOUND,
            format!("Resource '{resource}' not found"),
        ));
    }

    let raw = fs::read_to_string(&file)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    serde_json::from_str::<Value>(&raw).map_err(|e| {
        AppError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Invalid JSON: {e}"),
        )
    })
}

pub fn write_resource(folder: &Path, resource: &str, value: &Value) -> Result<(), AppError> {
    let file = resource_file_path(folder, resource)?;
    let content = serde_json::to_string_pretty(value)
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    fs::write(file, format!("{content}\n"))
        .map_err(|e| AppError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub fn resource_file_path(folder: &Path, resource: &str) -> Result<PathBuf, AppError> {
    if !is_valid_resource_name(resource) {
        return Err(AppError::new(
            StatusCode::BAD_REQUEST,
            "Resource name must only contain letters, numbers, underscore, and dash",
        ));
    }

    Ok(folder.join(format!("{resource}.json")))
}

pub fn is_valid_resource_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn find_item<'a>(items: &'a [Value], id: &str) -> Option<&'a Value> {
    items.iter().find(|item| id_matches(item, id))
}

pub fn find_item_index(items: &[Value], id: &str) -> Option<usize> {
    items.iter().position(|item| id_matches(item, id))
}

fn id_matches(item: &Value, expected: &str) -> bool {
    item.as_object()
        .and_then(|obj| obj.get("id"))
        .is_some_and(|id| match id {
            Value::Number(n) => n.to_string() == expected,
            Value::String(s) => s == expected,
            _ => false,
        })
}

pub fn next_numeric_id(items: &[Value]) -> i64 {
    items
        .iter()
        .filter_map(|item| item.as_object().and_then(|obj| obj.get("id")))
        .filter_map(|id| id.as_i64())
        .max()
        .map_or(1, |max| max + 1)
}

pub fn coerce_id_value(id: &str) -> Value {
    id.parse::<i64>()
        .map_or_else(|_| Value::String(id.to_string()), Value::from)
}

pub fn scan_resources(folder: &Path) -> Result<BTreeSet<String>, std::io::Error> {
    let mut resources = BTreeSet::new();
    let entries = fs::read_dir(folder)?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            && is_valid_resource_name(stem)
        {
            resources.insert(stem.to_owned());
        }
    }

    Ok(resources)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_resource_names() {
        assert!(is_valid_resource_name("users"));
        assert!(is_valid_resource_name("blog_posts-2025"));
        assert!(!is_valid_resource_name(""));
        assert!(!is_valid_resource_name("../evil"));
        assert!(!is_valid_resource_name("with space"));
    }

    #[test]
    fn finds_next_numeric_id() {
        let items = serde_json::json!([
            {"id": 1, "name": "a"},
            {"id": 5, "name": "b"},
            {"id": "abc", "name": "c"}
        ]);

        assert_eq!(next_numeric_id(items.as_array().expect("array")), 6);
    }

    #[test]
    fn writes_and_reads_resource_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let value = serde_json::json!([{"id": 1, "name": "example"}]);

        write_resource(temp.path(), "users", &value).expect("write resource");
        let loaded = load_resource(temp.path(), "users").expect("load resource");

        assert_eq!(value, loaded);
    }

    #[test]
    fn scans_only_valid_json_resource_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(temp.path().join("users.json"), "[]").expect("write users");
        fs::write(temp.path().join("posts.json"), "[]").expect("write posts");
        fs::write(temp.path().join("notes.txt"), "hello").expect("write txt");
        fs::write(temp.path().join("bad name.json"), "[]").expect("write invalid");

        let resources = scan_resources(temp.path()).expect("scan resources");

        assert_eq!(
            resources.into_iter().collect::<Vec<_>>(),
            vec!["posts".to_string(), "users".to_string()]
        );
    }
}
