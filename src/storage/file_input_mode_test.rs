use std::{
    fs,
    sync::{Arc, Barrier},
    thread,
    time::Duration,
};

#[path = "../test_support/mod.rs"]
mod support;

use support::{
    http_request, http_request_with_headers, parse_http_body, spawn_file_server, wait_for_server,
};

#[test]
fn serves_json_server_style_single_file_input() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let db_path = temp.path().join("db.json");
    fs::write(
        &db_path,
        r#"{
  "users": [
    {"id": 1, "name": "Ada"}
  ],
  "settings": {"theme": "dark"}
}
"#,
    )
    .expect("write db file");

    let (_child, bind_addr) = spawn_file_server(&db_path);
    wait_for_server(&bind_addr, Duration::from_secs(5));

    let root_response = http_request(&bind_addr, "GET", "/", None);
    assert!(root_response.starts_with("HTTP/1.1 200 OK\r\n"), "{root_response}");
    let root_body: serde_json::Value =
        serde_json::from_str(parse_http_body(&root_response)).expect("root body json");
    let resources = root_body["resources"].as_array().expect("resources array");
    assert!(resources.iter().any(|resource| resource.as_str() == Some("users")));
    assert!(resources.iter().any(|resource| resource.as_str() == Some("settings")));

    let post_response = http_request(&bind_addr, "POST", "/users", Some(r#"{"name":"Lin"}"#));
    assert!(post_response.starts_with("HTTP/1.1 201 Created\r\n"), "{post_response}");

    let db_text = fs::read_to_string(&db_path).expect("read db file");
    let db: serde_json::Value = serde_json::from_str(&db_text).expect("db json");
    assert_eq!(db["users"].as_array().expect("users array").len(), 2);
    assert_eq!(db["settings"]["theme"], "dark");
}

#[test]
fn file_input_mode_persists_collection_and_object_mutations() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let db_path = temp.path().join("db.json");
    fs::write(
        &db_path,
        r#"{
  "users": [
    {"id": 1, "name": "Ada"},
    {"id": 2, "name": "Bob"}
  ],
  "settings": {"theme": "dark", "locale": "en"}
}
"#,
    )
    .expect("write db file");

    let (_child, bind_addr) = spawn_file_server(&db_path);
    wait_for_server(&bind_addr, Duration::from_secs(5));

    let put_response = http_request(
        &bind_addr,
        "PUT",
        "/users/2",
        Some(r#"{"name":"Bobby","department":"engineering"}"#),
    );
    assert!(put_response.starts_with("HTTP/1.1 200 OK\r\n"), "{put_response}");
    let db_after_put: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&db_path).expect("read db")).expect("db json");
    assert_eq!(db_after_put["users"][1]["id"], 2);
    assert_eq!(db_after_put["users"][1]["name"], "Bobby");
    assert_eq!(db_after_put["users"][1]["department"], "engineering");
    assert_eq!(db_after_put["settings"]["theme"], "dark");

    let patch_response = http_request(&bind_addr, "PATCH", "/users/2", Some(r#"{"role":"admin"}"#));
    assert!(patch_response.starts_with("HTTP/1.1 200 OK\r\n"), "{patch_response}");
    let db_after_patch: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&db_path).expect("read db")).expect("db json");
    assert_eq!(db_after_patch["users"][1]["name"], "Bobby");
    assert_eq!(db_after_patch["users"][1]["role"], "admin");
    assert_eq!(db_after_patch["settings"]["locale"], "en");

    let delete_response = http_request(&bind_addr, "DELETE", "/users/1", None);
    assert!(delete_response.starts_with("HTTP/1.1 204 No Content\r\n"), "{delete_response}");
    let db_after_delete: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&db_path).expect("read db")).expect("db json");
    assert_eq!(db_after_delete["users"].as_array().expect("users array").len(), 1);
    assert_eq!(db_after_delete["users"][0]["id"], 2);
    assert_eq!(db_after_delete["settings"]["theme"], "dark");

    let put_settings =
        http_request(&bind_addr, "PUT", "/settings", Some(r#"{"theme":"light","locale":"de"}"#));
    assert!(put_settings.starts_with("HTTP/1.1 200 OK\r\n"), "{put_settings}");

    let patch_settings =
        http_request(&bind_addr, "PATCH", "/settings", Some(r#"{"timezone":"UTC"}"#));
    assert!(patch_settings.starts_with("HTTP/1.1 200 OK\r\n"), "{patch_settings}");

    let final_db: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&db_path).expect("read db")).expect("db json");
    assert_eq!(
        final_db,
        serde_json::json!({
            "users": [
                {"id": 2, "name": "Bobby", "department": "engineering", "role": "admin"}
            ],
            "settings": {"theme": "light", "locale": "de", "timezone": "UTC"}
        })
    );
}

#[test]
fn file_input_mode_concurrent_writes_to_different_resources_are_not_lost() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let db_path = temp.path().join("db.json");
    fs::write(
        &db_path,
        r#"{
  "users": [
    {"id": 1, "name": "Ada"}
  ],
  "posts": [
    {"id": 10, "title": "Hello"}
  ],
  "settings": {"theme": "dark"}
}
"#,
    )
    .expect("write db file");

    let (_child, bind_addr) = spawn_file_server(&db_path);
    wait_for_server(&bind_addr, Duration::from_secs(5));

    let barrier = Arc::new(Barrier::new(4));
    let users_addr = bind_addr.clone();
    let users_barrier = barrier.clone();
    let users_handle = thread::spawn(move || {
        users_barrier.wait();
        http_request(&users_addr, "POST", "/users", Some(r#"{"name":"Grace"}"#))
    });

    let posts_addr = bind_addr.clone();
    let posts_barrier = barrier.clone();
    let posts_handle = thread::spawn(move || {
        posts_barrier.wait();
        http_request(&posts_addr, "PATCH", "/posts/10", Some(r#"{"status":"published"}"#))
    });

    let settings_addr = bind_addr.clone();
    let settings_barrier = barrier.clone();
    let settings_handle = thread::spawn(move || {
        settings_barrier.wait();
        http_request(&settings_addr, "PATCH", "/settings", Some(r#"{"locale":"en"}"#))
    });

    barrier.wait();
    let users_response = users_handle.join().expect("join users request");
    let posts_response = posts_handle.join().expect("join posts request");
    let settings_response = settings_handle.join().expect("join settings request");

    assert!(users_response.starts_with("HTTP/1.1 201 Created\r\n"), "{users_response}");
    assert!(posts_response.starts_with("HTTP/1.1 200 OK\r\n"), "{posts_response}");
    assert!(settings_response.starts_with("HTTP/1.1 200 OK\r\n"), "{settings_response}");

    let final_db: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&db_path).expect("read db")).expect("db json");
    assert_eq!(
        final_db,
        serde_json::json!({
            "users": [
                {"id": 1, "name": "Ada"},
                {"id": 2, "name": "Grace"}
            ],
            "posts": [
                {"id": 10, "title": "Hello", "status": "published"}
            ],
            "settings": {"theme": "dark", "locale": "en"}
        })
    );
}

#[test]
fn file_input_mode_rejects_invalid_mutations_without_touching_disk() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let db_path = temp.path().join("db.json");
    fs::write(
        &db_path,
        r#"{
  "users": [
    {"id": 1, "name": "Ada"}
  ],
  "settings": {"theme": "dark"}
}
"#,
    )
    .expect("write db file");
    let original = fs::read_to_string(&db_path).expect("read db");

    let (_child, bind_addr) = spawn_file_server(&db_path);
    wait_for_server(&bind_addr, Duration::from_secs(5));

    let bad_post = http_request(&bind_addr, "POST", "/users", Some(r#"["bad"]"#));
    assert!(bad_post.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{bad_post}");
    assert!(bad_post.contains("Payload must be a JSON object"), "{bad_post}");

    let missing_put = http_request(&bind_addr, "PUT", "/users/999", Some(r#"{"name":"Ghost"}"#));
    assert!(missing_put.starts_with("HTTP/1.1 404 Not Found\r\n"), "{missing_put}");

    let bad_patch = http_request(&bind_addr, "PATCH", "/users/1", Some(r#"["bad"]"#));
    assert!(bad_patch.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{bad_patch}");
    assert!(bad_patch.contains("Payload must be a JSON object"), "{bad_patch}");

    let missing_delete = http_request(&bind_addr, "DELETE", "/users/999", None);
    assert!(missing_delete.starts_with("HTTP/1.1 404 Not Found\r\n"), "{missing_delete}");

    let final_db = fs::read_to_string(&db_path).expect("read db");
    assert_eq!(final_db, original);
}

#[test]
fn html_overview_explains_top_level_keys_in_file_mode() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let db_path = temp.path().join("db.json");
    fs::write(
        &db_path,
        r#"{
  "users": [
    {"id": 1, "name": "Ada"}
  ]
}
"#,
    )
    .expect("write db file");

    let (_child, bind_addr) = spawn_file_server(&db_path);
    wait_for_server(&bind_addr, Duration::from_secs(5));

    let html_response = http_request_with_headers(
        &bind_addr,
        "GET",
        "/",
        Some("Accept: text/html,application/xhtml+xml\r\n"),
        None,
    );
    assert!(html_response.starts_with("HTTP/1.1 200 OK\r\n"), "{html_response}");
    assert!(
        html_response.contains("Each valid top-level key in the JSON file becomes `/{resource}`."),
        "{html_response}"
    );
}

#[test]
fn serves_folder_when_directory_is_passed_positionally() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("users.json"),
        r#"[
  {"id": 1, "name": "Ada"}
]
"#,
    )
    .expect("write users file");

    let bind_addr = support::next_addr();
    let _child = support::ChildGuard::new(
        std::process::Command::new(env!("CARGO_BIN_EXE_dirbase"))
            .arg(temp.path())
            .arg("--bind")
            .arg(&bind_addr)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("start dirbase"),
    );

    wait_for_server(&bind_addr, Duration::from_secs(5));

    let response = http_request(&bind_addr, "GET", "/users", None);
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");

    let payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&response)).expect("users body json");
    assert_eq!(payload.as_array().expect("users array")[0]["name"], "Ada");
}
