use std::{fs, thread, time::Duration};

#[path = "test_support/mod.rs"]
mod support;

use support::{
    TestDatasetBuilder, TestServerBuilder, assert_status, http_request, parse_json_body,
    spawn_file_server, wait_for_http,
};

#[test]
fn folder_invalid_json_marks_ready_false_and_recovers() {
    let data = TestDatasetBuilder::new()
        .json_file("users.json", serde_json::json!([{"id": 1, "name": "Ada"}]));
    let users_path = data.file_path("users.json");
    let (_child, addr) = TestServerBuilder::folder(data.path()).spawn();

    fs::write(&users_path, r#"[{"id":1"#).expect("write invalid users");
    let not_ready = wait_for_http(&addr, "GET", "/readyz", None, |response| {
        response.starts_with("HTTP/1.1 503 Service Unavailable\r\n")
    });
    assert!(not_ready.contains("\"ready\":false"), "{not_ready}");

    fs::write(&users_path, r#"[{"id":1,"name":"Ada"},{"id":2,"name":"Grace"}]"#)
        .expect("restore users");
    let ready = wait_for_http(&addr, "GET", "/readyz", None, |response| {
        response.starts_with("HTTP/1.1 200 OK\r\n")
    });
    assert!(ready.contains("\"ready\":true"), "{ready}");

    let users = wait_for_http(&addr, "GET", "/users", None, |response| {
        response.starts_with("HTTP/1.1 200 OK\r\n") && response.contains("\"Grace\"")
    });
    assert!(users.contains("\"Ada\""), "{users}");
}

#[test]
fn file_mode_rejects_invalid_root_without_corrupting_file() {
    let data = TestDatasetBuilder::new()
        .write_file("db.json", r#"{"users":[{"id":1,"name":"Ada"}],"profile":{"theme":"dark"}}"#);
    let db_path = data.file_path("db.json");
    let (_child, addr) = spawn_file_server(&db_path);

    let loaded = http_request(&addr, "GET", "/users", None);
    assert_status(&loaded, "200 OK");

    fs::write(&db_path, r#"[{"id":1}]"#).expect("write invalid db root");
    let rejected = http_request(&addr, "PUT", "/users/1", Some(r#"{"name":"Grace"}"#));
    assert_status(&rejected, "404 Not Found");
    assert!(rejected.contains("Resource 'users' not found"), "{rejected}");
    assert_eq!(fs::read_to_string(&db_path).expect("read db"), r#"[{"id":1}]"#);
}

#[test]
fn concurrent_file_mode_writes_preserve_sibling_resources() {
    let data = TestDatasetBuilder::new().write_file(
        "db.json",
        r#"{"users":[{"id":1,"name":"Ada"}],"posts":[{"id":10,"title":"Hello"}],"profile":{"theme":"dark"}}"#,
    );
    let db_path = data.file_path("db.json");
    let (_child, addr) = spawn_file_server(&db_path);

    let users_addr = addr.clone();
    let posts_addr = addr.clone();
    let profile_addr = addr.clone();
    let users = thread::spawn(move || {
        http_request(&users_addr, "PATCH", "/users/1", Some(r#"{"name":"Grace"}"#))
    });
    let posts = thread::spawn(move || {
        http_request(&posts_addr, "PATCH", "/posts/10", Some(r#"{"title":"World"}"#))
    });
    let profile = thread::spawn(move || {
        http_request(&profile_addr, "PATCH", "/profile", Some(r#"{"theme":"light"}"#))
    });

    assert_status(&users.join().expect("users thread"), "200 OK");
    assert_status(&posts.join().expect("posts thread"), "200 OK");
    assert_status(&profile.join().expect("profile thread"), "200 OK");

    let parsed: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&db_path).expect("read db")).expect("db json");
    assert_eq!(parsed["users"][0]["name"], "Grace");
    assert_eq!(parsed["posts"][0]["title"], "World");
    assert_eq!(parsed["profile"]["theme"], "light");
}

#[test]
fn stale_temp_files_are_ignored_and_not_overwritten() {
    let data = TestDatasetBuilder::new()
        .json_file("users.json", serde_json::json!([{"id": 1, "name": "Ada"}]))
        .write_file("users.json.tmp.crash-simulation", r#"[{"id":"partial""#);
    let stale_temp = data.file_path("users.json.tmp.crash-simulation");
    let (_child, addr) = TestServerBuilder::folder(data.path()).spawn();

    let created = http_request(&addr, "POST", "/users", Some(r#"{"name":"Grace"}"#));
    assert_status(&created, "201 Created");

    let parsed: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(data.file_path("users.json")).expect("read users"),
    )
    .expect("users json");
    assert_eq!(parsed.as_array().expect("users").len(), 2);
    assert_eq!(fs::read_to_string(stale_temp).expect("read stale temp"), r#"[{"id":"partial""#);
}

#[test]
fn resource_cache_invalidates_after_external_file_deletion() {
    let data = TestDatasetBuilder::new()
        .json_file("users.json", serde_json::json!([{"id": 1, "name": "Ada"}]));
    let users_path = data.file_path("users.json");
    let (_child, addr) = TestServerBuilder::folder(data.path()).spawn();

    let loaded = http_request(&addr, "GET", "/users", None);
    assert_status(&loaded, "200 OK");

    fs::remove_file(users_path).expect("delete users");
    let missing = wait_for_http(&addr, "GET", "/users", None, |response| {
        response.starts_with("HTTP/1.1 404 Not Found\r\n")
    });
    assert!(missing.contains("Resource 'users' not found"), "{missing}");
}

#[test]
fn schema_file_edit_invalidates_graphql_schema_cache() {
    let data = TestDatasetBuilder::new()
        .write_file(
            "schema.dbml",
            r#"
Table users {
  id int [pk]
  name varchar
}
"#,
        )
        .json_file("users.json", serde_json::json!([{"id": 1, "name": "Ada"}]));
    let schema_path = data.file_path("schema.dbml");
    let users_path = data.file_path("users.json");
    let (_child, addr) = TestServerBuilder::folder(data.path()).spawn();

    let first = support::http_post_json(
        &addr,
        "/graphql",
        serde_json::json!({"query": "{ users { id name } }"}),
    );
    assert_status(&first, "200 OK");
    assert_eq!(parse_json_body(&first)["data"]["users"][0]["name"], "Ada");

    fs::write(
        &schema_path,
        r#"
Table users {
  id int [pk]
  name varchar
  email varchar
}
"#,
    )
    .expect("update schema");
    fs::write(&users_path, r#"[{"id":1,"name":"Ada","email":"ada@example.com"}]"#)
        .expect("update users");

    let updated = wait_for_http(
        &addr,
        "POST",
        "/graphql",
        Some(r#"{"query":"{ users { id name email } }"}"#),
        |response| {
            response.starts_with("HTTP/1.1 200 OK\r\n") && response.contains("ada@example.com")
        },
    );
    assert_eq!(parse_json_body(&updated)["data"]["users"][0]["email"], "ada@example.com");

    thread::sleep(Duration::from_millis(10));
}
