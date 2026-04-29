use std::fs;

mod support;

use support::{http_request_with_headers, spawn_folder_server, spawn_folder_server_with_args};

#[test]
fn root_serves_html_overview_for_browser_requests() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("users.json"),
        r#"[
  {"id": 1, "name": "Ada"},
  {"id": 2, "name": "Linus"}
]
"#,
    )
    .expect("write users");
    fs::write(
        temp.path().join("posts.json"),
        r#"[
  {"id": 1, "title": "Hello", "user_id": 1}
]
"#,
    )
    .expect("write posts");
    fs::write(
        temp.path().join("schema.dbml"),
        r#"
Table users {
  id int [pk]
  name varchar [not null]
}

Table posts {
  id int [pk]
  title varchar [not null]
  user_id int [ref: > users.id]
}
"#,
    )
    .expect("write schema");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);
    let response = http_request_with_headers(
        &bind_addr,
        "GET",
        "/",
        Some("Accept: text/html,application/xhtml+xml\r\n"),
        None,
    );

    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(response.contains("content-type: text/html; charset=utf-8"), "{response}");
    assert!(response.contains("<h1>Data workspace</h1>"), "{response}");
    assert!(response.contains("Routes and quick checks"), "{response}");
    assert!(!response.contains("Rules of paths"), "{response}");
    assert!(!response.contains("First 60 seconds"), "{response}");
    assert!(response.contains("Create one row"), "{response}");
    assert!(response.contains("<span class=\"overview-method\">POST</span> /posts"), "{response}");
    assert!(response.contains("id=\"overview-root\""), "{response}");
    assert!(response.contains("data-overview-endpoint=\"/overview.json\""), "{response}");
    assert!(response.contains("href=\"/assets/overview.css\""), "{response}");
    assert!(response.contains("src=\"/assets/overview.js\""), "{response}");
    assert!(response.contains("Source mode: folder"), "{response}");
    assert!(response.contains("Each valid `*.json` filename becomes `/{resource}`."), "{response}");
    assert!(
        response
            .contains("This server is writable. Use the overview to create, edit, and delete rows"),
        "{response}"
    );
    assert!(response.contains("data-resource=\"posts\""), "{response}");
    assert!(response.contains("data-resource=\"users\""), "{response}");
}

#[test]
fn readonly_root_html_calls_out_disabled_mutations() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("users.json"), "[{\"id\":1,\"name\":\"Ada\"}]\n")
        .expect("write users");

    let (_child, bind_addr) = spawn_folder_server_with_args(temp.path(), &["--readonly"]);
    let response = http_request_with_headers(
        &bind_addr,
        "GET",
        "/",
        Some("Accept: text/html,application/xhtml+xml\r\n"),
        None,
    );

    assert!(response.contains("This server is in readonly mode."), "{response}");
    assert!(response.contains("mutations and schema writes are disabled"), "{response}");
}
