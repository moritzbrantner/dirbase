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
