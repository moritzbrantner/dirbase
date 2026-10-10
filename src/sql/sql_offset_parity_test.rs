#[path = "../test_support/mod.rs"]
mod support;

use support::{http_get, parse_http_body, spawn_folder_server};

#[test]
fn sql_limit_offset_uses_exact_row_offset() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let users = serde_json::json!([
        {"id": 1, "name": "Ada"},
        {"id": 2, "name": "Bob"},
        {"id": 3, "name": "Cara"},
        {"id": 4, "name": "Dora"},
        {"id": 5, "name": "Eli"}
    ]);

    std::fs::write(
        temp.path().join("users.json"),
        serde_json::to_string_pretty(&users).expect("serialize users"),
    )
    .expect("write users json");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), true);

    let response = http_get(
        &bind_addr,
        "/sql?q=SELECT%20id,name%20FROM%20users%20ORDER%20BY%20id%20ASC%20LIMIT%202%20OFFSET%201",
    );
    assert!(response.contains("200 OK"), "{response}");
    let payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&response)).expect("json payload");

    assert_eq!(payload["row_count"], 2);
    assert_eq!(payload["rows"][0], serde_json::json!({"id": 2, "name": "Bob"}));
    assert_eq!(payload["rows"][1], serde_json::json!({"id": 3, "name": "Cara"}));

    let tail_response = http_get(
        &bind_addr,
        "/sql?q=SELECT%20id%20FROM%20users%20ORDER%20BY%20id%20ASC%20LIMIT%202%20OFFSET%204",
    );
    assert!(tail_response.contains("200 OK"), "{tail_response}");
    let tail_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&tail_response)).expect("tail json payload");

    assert_eq!(tail_payload["row_count"], 1);
    assert_eq!(tail_payload["rows"][0]["id"], 5);
}

fn write_users_fixture(folder: &std::path::Path) {
    let users = serde_json::json!([
        {"id": 1, "name": "Ada"},
        {"id": 2, "name": "Bob"},
        {"id": 3, "name": "Cara"}
    ]);
    std::fs::write(
        folder.join("users.json"),
        serde_json::to_string_pretty(&users).expect("serialize users"),
    )
    .expect("write users json");
}

fn assert_sql_empty_result(bind_addr: &str, path: &str) {
    let response = http_get(bind_addr, path);
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{path}\n{response}");
    let payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&response)).expect("json payload");
    assert_eq!(payload["row_count"], 0, "{path}\n{response}");
    assert_eq!(payload["rows"], serde_json::json!([]), "{path}\n{response}");
}

fn assert_server_still_serves_rows(bind_addr: &str) {
    let response = http_get(
        bind_addr,
        "/sql?q=SELECT%20id%20FROM%20users%20ORDER%20BY%20id%20ASC%20LIMIT%202",
    );
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    let payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&response)).expect("follow-up json payload");
    assert_eq!(payload["row_count"], 2, "{response}");
    assert_eq!(payload["rows"], serde_json::json!([{"id": 1}, {"id": 2}]), "{response}");
}

#[test]
fn sql_limit_zero_returns_no_rows() {
    let temp = tempfile::tempdir().expect("create temp directory");
    write_users_fixture(temp.path());
    let (_child, bind_addr) = spawn_folder_server(temp.path(), true);

    assert_sql_empty_result(
        &bind_addr,
        "/sql?q=SELECT%20id,name%20FROM%20users%20ORDER%20BY%20id%20ASC%20LIMIT%200",
    );
    assert_server_still_serves_rows(&bind_addr);
}

#[test]
fn sql_limit_zero_with_offset_returns_no_rows_without_panicking() {
    let temp = tempfile::tempdir().expect("create temp directory");
    write_users_fixture(temp.path());
    let (_child, bind_addr) = spawn_folder_server(temp.path(), true);

    assert_sql_empty_result(
        &bind_addr,
        "/sql?q=SELECT%20id,name%20FROM%20users%20ORDER%20BY%20id%20ASC%20LIMIT%200%20OFFSET%201",
    );
    assert_server_still_serves_rows(&bind_addr);
}

#[test]
fn sql_offset_without_limit_is_still_invalid_sql() {
    let temp = tempfile::tempdir().expect("create temp directory");
    write_users_fixture(temp.path());
    let (_child, bind_addr) = spawn_folder_server(temp.path(), true);

    let response = http_get(
        &bind_addr,
        "/sql?q=SELECT%20id%20FROM%20users%20ORDER%20BY%20id%20ASC%20OFFSET%201",
    );
    assert!(response.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{response}");
    let payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&response)).expect("error payload");
    assert_eq!(payload["code"], "invalid_sql", "{response}");
    assert!(
        payload["error"].as_str().expect("error message").contains("OFFSET requires LIMIT"),
        "{response}"
    );
    assert_server_still_serves_rows(&bind_addr);
}
