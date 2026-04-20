use fake::{Fake, faker::name::en::Name};

mod support;

use support::{ChildGuard, http_get, parse_http_body};

#[test]
fn collection_supports_filtering_with_multiple_query_parameters() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let users_path = temp.path().join("users.json");

    let mut users = Vec::new();
    for id in 1..=6 {
        let role = if id % 2 == 0 { "admin" } else { "member" };
        let active = id % 2 != 0;
        users.push(serde_json::json!({
            "id": id,
            "name": Name().fake::<String>(),
            "role": role,
            "active": active
        }));
    }

    users.push(serde_json::json!({
        "id": 99,
        "name": Name().fake::<String>(),
        "role": "admin",
        "active": true
    }));

    std::fs::write(users_path, serde_json::to_string_pretty(&users).expect("serialize fake users"))
        .expect("write users json");

    let (_child, bind_addr) = spawn_server(temp.path());
    let response = http_get(&bind_addr, "/users?role=admin&active=true");

    assert!(
        response.starts_with("HTTP/1.1 200 OK\r\n"),
        "expected 200 OK response, got: {response}"
    );
    assert!(
        response.contains("\"id\":99")
            && !response.contains("\"id\":2")
            && !response.contains("\"id\":6"),
        "expected filtered body containing only active admins, got: {response}"
    );
}

#[test]
fn collection_supports_sorting_by_multiple_columns() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let users_path = temp.path().join("users.json");

    let users = serde_json::json!([
        {"id": 3, "name": "Zed", "role": "admin"},
        {"id": 1, "name": "Ada", "role": "member"},
        {"id": 2, "name": "Bob", "role": "admin"}
    ]);

    std::fs::write(users_path, serde_json::to_string_pretty(&users).expect("serialize users"))
        .expect("write users json");

    let (_child, bind_addr) = spawn_server(temp.path());
    let response = http_get(&bind_addr, "/users?sort=role,name");

    assert!(
        response.starts_with("HTTP/1.1 200 OK\r\n"),
        "expected 200 OK response, got: {response}"
    );

    let body = parse_http_body(&response);
    let users: serde_json::Value = serde_json::from_str(body).expect("valid json body");
    let sorted_ids = users
        .as_array()
        .expect("array response")
        .iter()
        .map(|item| item["id"].as_i64().expect("numeric id"))
        .collect::<Vec<_>>();

    assert_eq!(sorted_ids, vec![2, 3, 1]);
}

#[test]
fn collection_supports_operator_filters_nested_fields_desc_sort_and_pagination_keywords() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let posts_path = temp.path().join("posts.json");

    let posts = serde_json::json!([
        {"id": 1, "title": "Hello world", "views": 100, "author": {"name": "Typicode"}},
        {"id": 2, "title": "HELLO rust", "views": 250, "author": {"name": "Typicode"}},
        {"id": 3, "title": "Another", "views": 300, "author": {"name": "Alice"}},
        {"id": 4, "title": "hello api", "views": 200, "author": {"name": "Typicode"}}
    ]);

    std::fs::write(posts_path, serde_json::to_string_pretty(&posts).expect("serialize posts"))
        .expect("write posts json");

    let (_child, bind_addr) = spawn_server(temp.path());
    let response = http_get(
        &bind_addr,
        "/posts?views:gte=100&title:contains=hello&author.name:eq=Typicode&_sort=-views&_page=1&_per_page=2",
    );

    assert!(
        response.starts_with("HTTP/1.1 200 OK\r\n"),
        "expected 200 OK response, got: {response}"
    );

    let body = parse_http_body(&response);
    let payload: serde_json::Value = serde_json::from_str(body).expect("valid json body");

    assert_eq!(payload["first"], 1);
    assert_eq!(payload["last"], 2);
    assert_eq!(payload["pages"], 2);
    assert_eq!(payload["items"], 3);
    assert_eq!(payload["next"], 2);

    let ids = payload["data"]
        .as_array()
        .expect("array response")
        .iter()
        .map(|item| item["id"].as_i64().expect("numeric id"))
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![2, 4]);
}

#[test]
fn collection_rejects_invalid_filter_operator_and_invalid_pagination_values() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let users_path = temp.path().join("users.json");

    let users = serde_json::json!([
        {"id": 1, "name": "Ada", "role": "admin"},
        {"id": 2, "name": "Bob", "role": "member"}
    ]);

    std::fs::write(users_path, serde_json::to_string_pretty(&users).expect("serialize users"))
        .expect("write users json");

    let (_child, bind_addr) = spawn_server(temp.path());

    let invalid_operator = http_get(&bind_addr, "/users?role:badop=admin");
    assert!(
        invalid_operator.starts_with("HTTP/1.1 400 Bad Request\r\n"),
        "expected 400 Bad Request response, got: {invalid_operator}"
    );

    let invalid_page = http_get(&bind_addr, "/users?_page=0&_per_page=2");
    assert!(
        invalid_page.starts_with("HTTP/1.1 400 Bad Request\r\n"),
        "expected 400 Bad Request response, got: {invalid_page}"
    );
}

#[test]
fn collection_clamps_pagination_page_to_last_page() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let users_path = temp.path().join("users.json");

    let users = serde_json::json!([
        {"id": 1, "name": "Ada"},
        {"id": 2, "name": "Bob"},
        {"id": 3, "name": "Cara"}
    ]);

    std::fs::write(users_path, serde_json::to_string_pretty(&users).expect("serialize users"))
        .expect("write users json");

    let (_child, bind_addr) = spawn_server(temp.path());
    let response = http_get(&bind_addr, "/users?_page=99&_per_page=2");
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));

    let body = parse_http_body(&response);
    let payload: serde_json::Value = serde_json::from_str(body).expect("valid json body");
    assert_eq!(payload["last"], 2);
    assert_eq!(payload["prev"], 1);
    assert_eq!(payload["next"], serde_json::Value::Null);

    let ids = payload["data"]
        .as_array()
        .expect("array response")
        .iter()
        .map(|item| item["id"].as_i64().expect("numeric id"))
        .collect::<Vec<_>>();
    assert_eq!(ids, vec![3]);
}

#[test]
fn collection_supports_null_filter_operators() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let users_path = temp.path().join("users.json");

    let users = serde_json::json!([
        {"id": 1, "name": "Ada", "deleted_at": null},
        {"id": 2, "name": "Grace", "deleted_at": "2026-01-01"}
    ]);

    std::fs::write(users_path, serde_json::to_string_pretty(&users).expect("serialize users"))
        .expect("write users json");

    let (_child, bind_addr) = spawn_server(temp.path());

    let is_null = http_get(&bind_addr, "/users?deleted_at:isNull=true");
    assert!(is_null.starts_with("HTTP/1.1 200 OK\r\n"), "{is_null}");
    assert!(is_null.contains("\"id\":1"), "{is_null}");
    assert!(!is_null.contains("\"id\":2"), "{is_null}");

    let is_not_null = http_get(&bind_addr, "/users?deleted_at:isNotNull=true");
    assert!(is_not_null.starts_with("HTTP/1.1 200 OK\r\n"), "{is_not_null}");
    assert!(is_not_null.contains("\"id\":2"), "{is_not_null}");
    assert!(!is_not_null.contains("\"id\":1"), "{is_not_null}");
}

fn spawn_server(folder: &std::path::Path) -> (ChildGuard, String) {
    support::spawn_folder_server(folder, false)
}
