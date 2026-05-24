use std::fs;

#[path = "test_support/mod.rs"]
mod support;

use support::{
    assert_status, http_request, parse_http_body, parse_json_body, spawn_file_server,
    spawn_folder_server,
};

#[test]
fn resources_endpoint_creates_and_deletes_folder_resources() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("users.json"), r#"[{"id":1,"name":"Ada"}]"#).expect("write users");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let created = http_request(
        &bind_addr,
        "POST",
        "/resources",
        Some(r#"{"name":"projects","initial":[{"id":1,"name":"Search"}]}"#),
    );
    assert_status(&created, "201 Created");
    let created_payload = parse_json_body(&created);
    assert_eq!(created_payload["created"], true);
    assert_eq!(created_payload["resource"], "projects");
    assert_eq!(created_payload["data"], serde_json::json!([{"id": 1, "name": "Search"}]));

    let persisted: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(temp.path().join("projects.json")).expect("read"))
            .expect("json");
    assert_eq!(persisted, serde_json::json!([{"id": 1, "name": "Search"}]));

    let list = parse_json_body(&http_request(&bind_addr, "GET", "/resources", None));
    assert_eq!(list["resources"], serde_json::json!(["projects", "users"]));

    let get_created = http_request(&bind_addr, "GET", "/projects", None);
    assert_status(&get_created, "200 OK");
    assert_eq!(parse_json_body(&get_created), serde_json::json!([{"id": 1, "name": "Search"}]));

    let delete_without_confirm = http_request(&bind_addr, "DELETE", "/resources/projects", None);
    assert_status(&delete_without_confirm, "400 Bad Request");
    assert!(
        parse_http_body(&delete_without_confirm)
            .contains("Deleting a resource requires confirm=true")
    );
    assert!(temp.path().join("projects.json").exists());

    let deleted = http_request(&bind_addr, "DELETE", "/resources/projects?confirm=true", None);
    assert_status(&deleted, "204 No Content");
    assert!(!temp.path().join("projects.json").exists());

    let get_deleted = http_request(&bind_addr, "GET", "/projects", None);
    assert_status(&get_deleted, "404 Not Found");
}

#[test]
fn resources_endpoint_creates_and_deletes_file_mode_resources() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let db = temp.path().join("db.json");
    fs::write(&db, r#"{"users":[{"id":1,"name":"Ada"}]}"#).expect("write db");

    let (_child, bind_addr) = spawn_file_server(&db);

    let created = http_request(
        &bind_addr,
        "POST",
        "/resources",
        Some(r#"{"name":"settings","initial":{"theme":"dark"}}"#),
    );
    assert_status(&created, "201 Created");
    let persisted: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&db).expect("read db")).expect("json");
    assert_eq!(persisted["users"], serde_json::json!([{"id": 1, "name": "Ada"}]));
    assert_eq!(persisted["settings"], serde_json::json!({"theme": "dark"}));

    let get_created = http_request(&bind_addr, "GET", "/settings", None);
    assert_status(&get_created, "200 OK");
    assert_eq!(parse_json_body(&get_created), serde_json::json!({"theme": "dark"}));

    let deleted = http_request(&bind_addr, "DELETE", "/resources/settings?confirm=true", None);
    assert_status(&deleted, "204 No Content");
    let persisted: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&db).expect("read db")).expect("json");
    assert!(persisted.get("settings").is_none());
    assert_eq!(persisted["users"], serde_json::json!([{"id": 1, "name": "Ada"}]));
}

#[test]
fn resources_endpoint_validates_names_duplicates_defaults_and_readonly() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("users.json"), "[]").expect("write users");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let default_created =
        http_request(&bind_addr, "POST", "/resources", Some(r#"{"name":"empty"}"#));
    assert_status(&default_created, "201 Created");
    assert_eq!(
        parse_json_body(&http_request(&bind_addr, "GET", "/empty", None)),
        serde_json::json!([])
    );

    let duplicate =
        http_request(&bind_addr, "POST", "/resources", Some(r#"{"name":"users","initial":[]}"#));
    assert_status(&duplicate, "409 Conflict");

    let invalid =
        http_request(&bind_addr, "POST", "/resources", Some(r#"{"name":"bad name","initial":[]}"#));
    assert_status(&invalid, "400 Bad Request");

    let reserved = http_request(
        &bind_addr,
        "POST",
        "/resources",
        Some(r#"{"name":"resources","initial":[]}"#),
    );
    assert_status(&reserved, "400 Bad Request");

    let (_readonly_child, readonly_addr) = spawn_folder_server(temp.path(), true);
    let readonly_create =
        http_request(&readonly_addr, "POST", "/resources", Some(r#"{"name":"blocked"}"#));
    assert_status(&readonly_create, "405 Method Not Allowed");
    let readonly_delete =
        http_request(&readonly_addr, "DELETE", "/resources/users?confirm=true", None);
    assert_status(&readonly_delete, "405 Method Not Allowed");
}
