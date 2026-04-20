use std::fs;

mod support;

use support::{http_get, http_request, parse_http_body, spawn_folder_server};

#[test]
fn retrieval_supports_string_ids_and_returns_404_for_missing_item() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("users.json"),
        r#"[
  {"id": "user-1", "name": "Ada"},
  {"id": "user-2", "name": "Grace"}
]
"#,
    )
    .expect("write users");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let get_existing = http_get(&bind_addr, "/users/user-1");
    assert!(get_existing.starts_with("HTTP/1.1 200 OK\r\n"), "{get_existing}");
    let existing_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&get_existing)).expect("valid json body");
    assert_eq!(existing_payload["name"], "Ada");

    let get_missing = http_get(&bind_addr, "/users/user-99");
    assert!(get_missing.starts_with("HTTP/1.1 404 Not Found\r\n"), "{get_missing}");
    assert!(get_missing.contains("\"error\":\"Item not found\""), "{get_missing}");
}

#[test]
fn query_parameters_support_pagination_defaults_when_page_or_size_is_missing() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("posts.json"),
        r#"[
  {"id": 1, "title": "one"},
  {"id": 2, "title": "two"},
  {"id": 3, "title": "three"}
]
"#,
    )
    .expect("write posts");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let per_page_only = http_get(&bind_addr, "/posts?_per_page=2");
    assert!(per_page_only.starts_with("HTTP/1.1 200 OK\r\n"), "{per_page_only}");
    let per_page_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&per_page_only)).expect("valid json body");
    assert_eq!(per_page_payload["first"], 1);
    assert_eq!(per_page_payload["last"], 2);
    assert_eq!(per_page_payload["prev"], serde_json::Value::Null);
    assert_eq!(per_page_payload["next"], 2);

    let page_only = http_get(&bind_addr, "/posts?_page=2");
    assert!(page_only.starts_with("HTTP/1.1 200 OK\r\n"), "{page_only}");
    let page_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&page_only)).expect("valid json body");
    assert_eq!(page_payload["first"], 1);
    assert_eq!(page_payload["last"], 1);
    assert_eq!(page_payload["pages"], 1);
    assert_eq!(page_payload["items"], 3);
    assert_eq!(page_payload["data"].as_array().expect("array").len(), 3);
}

#[test]
fn query_parameters_return_clear_400_errors_for_invalid_values() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("posts.json"), "[]\n").expect("write posts");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let bad_page = http_get(&bind_addr, "/posts?_page=0");
    assert!(bad_page.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{bad_page}");
    assert!(bad_page.contains("\"error\":\"'_page' must be greater than 0\""));

    let bad_operator = http_get(&bind_addr, "/posts?title:unknown=value");
    assert!(bad_operator.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{bad_operator}");
    assert!(
        bad_operator
            .contains("\"error\":\"Unsupported filter operator 'unknown' in 'title:unknown'\""),
        "{bad_operator}"
    );

    let bad_per_page = http_get(&bind_addr, "/posts?per_page=abc");
    assert!(bad_per_page.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{bad_per_page}");
    assert!(
        bad_per_page.contains("\"error\":\"Invalid value for 'per_page': 'abc'\""),
        "{bad_per_page}"
    );
}

#[test]
fn query_parameters_support_unprefixed_pagination_aliases() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("posts.json"),
        r#"[
  {"id": 1, "title": "one"},
  {"id": 2, "title": "two"},
  {"id": 3, "title": "three"},
  {"id": 4, "title": "four"}
]
"#,
    )
    .expect("write posts");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let page_two = http_get(&bind_addr, "/posts?page=2&per_page=2");
    assert!(page_two.starts_with("HTTP/1.1 200 OK\r\n"), "{page_two}");
    let payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&page_two)).expect("valid json body");
    assert_eq!(payload["prev"], 1);
    assert_eq!(payload["next"], serde_json::Value::Null);
    assert_eq!(payload["last"], 2);

    let ids = payload["data"]
        .as_array()
        .expect("array response")
        .iter()
        .map(|item| item["id"].as_i64().expect("numeric id"))
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![3, 4]);
}

#[test]
fn query_parameters_support_embed_for_foreign_keys() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("schema.dbml"),
        r#"
Table users {
  id int [pk]
  name varchar
}

Table posts {
  id int [pk]
  title varchar
  author_id int [ref: > users.id]
}
"#,
    )
    .expect("write schema");
    fs::write(
        temp.path().join("users.json"),
        r#"[
  {"id": 1, "name": "Ada"},
  {"id": 2, "name": "Grace"}
]
"#,
    )
    .expect("write users");
    fs::write(
        temp.path().join("posts.json"),
        r#"[
  {"id": 1, "title": "Hello", "author_id": 1},
  {"id": 2, "title": "World", "author_id": 2}
]
"#,
    )
    .expect("write posts");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let response = http_get(&bind_addr, "/posts?embed=author_id");
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");

    let payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&response)).expect("valid json body");
    assert_eq!(payload[0]["author_id"]["id"], 1);
    assert_eq!(payload[0]["author_id"]["name"], "Ada");
    assert_eq!(payload[1]["author_id"]["id"], 2);
    assert_eq!(payload[1]["author_id"]["name"], "Grace");
}

#[test]
fn retrieval_returns_400_for_invalid_resource_name() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("users.json"), "[]\n").expect("write users");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let response = http_get(&bind_addr, "/users..bad/1");
    assert!(response.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{response}");
    assert!(
        response.contains(
            "\"error\":\"Resource name must only contain letters, numbers, underscore, and dash\""
        ),
        "{response}"
    );
}

#[test]
fn retrieval_and_create_use_declared_primary_key() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("schema.json"),
        r#"{
  "tables": {
    "users": {
      "primary_key": "user_id",
      "columns": {
        "user_id": {"column_type": "integer", "nullable": false},
        "name": {"column_type": "string", "nullable": false}
      }
    }
  }
}
"#,
    )
    .expect("write schema");
    fs::write(
        temp.path().join("users.json"),
        r#"[
  {"user_id": 1, "name": "Ada"},
  {"user_id": 2, "name": "Grace"}
]
"#,
    )
    .expect("write users");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let get_existing = http_get(&bind_addr, "/users/1");
    assert!(get_existing.starts_with("HTTP/1.1 200 OK\r\n"), "{get_existing}");
    let existing_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&get_existing)).expect("valid json body");
    assert_eq!(existing_payload["name"], "Ada");

    let post_response = http_request(&bind_addr, "POST", "/users", Some(r#"{"name":"Lin"}"#));
    assert!(post_response.starts_with("HTTP/1.1 201 Created\r\n"), "{post_response}");
    let created_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&post_response)).expect("created json");
    assert_eq!(created_payload["user_id"], 3);
    assert_eq!(created_payload["name"], "Lin");

    let get_created = http_get(&bind_addr, "/users/3");
    assert!(get_created.starts_with("HTTP/1.1 200 OK\r\n"), "{get_created}");
}

#[test]
fn overlay_schema_validation_allows_undeclared_resources_and_columns() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("schema.json"),
        r#"{
  "tables": {
    "posts": {
      "columns": {
        "title": {"column_type": "string", "nullable": false}
      }
    }
  }
}
"#,
    )
    .expect("write schema");
    fs::write(
        temp.path().join("posts.json"),
        r#"[
  {"id": 1, "title": "Hello", "extra": true}
]
"#,
    )
    .expect("write posts");
    fs::write(
        temp.path().join("users.json"),
        r#"[
  {"id": 1, "name": "Ada"}
]
"#,
    )
    .expect("write users");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let users_response = http_get(&bind_addr, "/users");
    assert!(users_response.starts_with("HTTP/1.1 200 OK\r\n"), "{users_response}");

    let posts_response = http_get(&bind_addr, "/posts");
    assert!(posts_response.starts_with("HTTP/1.1 200 OK\r\n"), "{posts_response}");
    let posts_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&posts_response)).expect("posts json");
    assert_eq!(posts_payload[0]["extra"], true);
}
