use std::fs;

mod support;

use support::{http_request_with_headers, parse_http_body, spawn_file_server, spawn_folder_server};

#[test]
fn overview_json_returns_machine_readable_metadata_for_folder_mode() {
    let temp = tempfile::tempdir().expect("create temp directory");
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
  name varchar
}

Table posts {
  id int [pk]
  title varchar
  user_id int [ref: > users.id]
}
"#,
    )
    .expect("write schema");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let response = http_request_with_headers(&bind_addr, "GET", "/overview.json", None, None);
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(response.contains("content-type: application/json"), "{response}");

    let payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&response)).expect("overview json body");
    assert_eq!(payload["data_source_kind"], "folder");
    assert_eq!(payload["server_capabilities"]["readonly"], false);
    assert_eq!(payload["server_capabilities"]["resource_write"], true);
    assert_eq!(payload["server_capabilities"]["schema_write"], true);
    assert_eq!(payload["server_capabilities"]["schema_infer"], true);
    assert_eq!(payload["stats"]["resource_count"], 2);
    assert_eq!(payload["stats"]["relation_count"], 1);
    assert_eq!(payload["stats"]["total_rows"], 3);
    assert!(payload["resources"].as_array().expect("resources array").iter().any(
        |resource| resource["name"] == "posts"
            && resource["query_capabilities"]["pagination"] == true
            && resource["mutation_capabilities"]["create_item"] == true
            && resource["mutation_capabilities"]["update_item"] == true
            && resource["mutation_capabilities"]["delete_item"] == true
    ));
    assert!(payload["resources"].as_array().expect("resources array").iter().any(
        |resource| resource["name"] == "users"
            && resource["sample_item_id"] == "1"
            && resource["mutation_capabilities"]["replace_object"] == false
    ));
    assert_eq!(payload["edges"][0]["source_table"], "posts");
    assert_eq!(payload["edges"][0]["target_table"], "users");
}

#[test]
fn overview_json_describes_file_mode_and_assets_are_served() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let db_path = temp.path().join("db.json");
    fs::write(
        &db_path,
        r#"{
  "users": [
    {"id": 1, "name": "Ada"}
  ],
  "settings": {
    "theme": "warm"
  }
}
"#,
    )
    .expect("write db file");

    let (_child, bind_addr) = spawn_file_server(&db_path);

    let overview_response =
        http_request_with_headers(&bind_addr, "GET", "/overview.json", None, None);
    assert!(overview_response.starts_with("HTTP/1.1 200 OK\r\n"), "{overview_response}");
    let payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&overview_response)).expect("overview body");
    assert_eq!(payload["data_source_kind"], "file");
    assert_eq!(
        payload["source_rule"],
        "Each valid top-level key in the JSON file becomes `/{resource}`."
    );
    assert!(payload["resources"].as_array().expect("resources array").iter().any(
        |resource| resource["name"] == "settings"
            && resource["kind"] == "object"
            && resource["mutation_capabilities"]["patch_object"] == true
            && resource["mutation_capabilities"]["replace_object"] == true
    ));

    let css_response =
        http_request_with_headers(&bind_addr, "GET", "/assets/overview.css", None, None);
    assert!(css_response.starts_with("HTTP/1.1 200 OK\r\n"), "{css_response}");
    assert!(css_response.contains("content-type: text/css; charset=utf-8"), "{css_response}");
    assert!(parse_http_body(&css_response).contains(".overview-page"), "{css_response}");

    let js_response =
        http_request_with_headers(&bind_addr, "GET", "/assets/overview.js", None, None);
    assert!(js_response.starts_with("HTTP/1.1 200 OK\r\n"), "{js_response}");
    assert!(js_response.contains("content-type: text/javascript; charset=utf-8"), "{js_response}");
    assert!(parse_http_body(&js_response).contains("overview-root"), "{js_response}");
}

#[test]
fn overview_json_reflects_readonly_capabilities() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("users.json"),
        r#"[
  {"id": 1, "name": "Ada"}
]
"#,
    )
    .expect("write users");
    fs::write(
        temp.path().join("settings.json"),
        r#"{"theme":"warm"}
"#,
    )
    .expect("write settings");

    let (_child, bind_addr) = support::spawn_folder_server_with_args(temp.path(), &["--readonly"]);

    let response = http_request_with_headers(&bind_addr, "GET", "/overview.json", None, None);
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");

    let payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&response)).expect("overview json body");
    assert_eq!(payload["server_capabilities"]["readonly"], true);
    assert_eq!(payload["server_capabilities"]["resource_write"], false);
    assert_eq!(payload["server_capabilities"]["schema_write"], false);
    assert_eq!(payload["server_capabilities"]["schema_infer"], false);
    assert!(
        payload["resources"]
            .as_array()
            .expect("resources array")
            .iter()
            .any(|resource| resource["name"] == "users"
                && resource["mutation_capabilities"]["create_item"] == true)
    );
}

#[test]
fn overview_json_includes_many_to_many_edges_and_relations() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("students.json"),
        r#"[
  {"id": 1, "name": "Ada"}
]
"#,
    )
    .expect("write students");
    fs::write(
        temp.path().join("courses.json"),
        r#"[
  {"id": 10, "title": "Math"}
]
"#,
    )
    .expect("write courses");
    fs::write(
        temp.path().join("student_courses.json"),
        r#"[
  {"student_id": 1, "course_id": 10}
]
"#,
    )
    .expect("write relation");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let response = http_request_with_headers(&bind_addr, "GET", "/overview.json", None, None);
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");

    let payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&response)).expect("overview json body");

    let student_resource = payload["resources"]
        .as_array()
        .expect("resources array")
        .iter()
        .find(|resource| resource["name"] == "students")
        .expect("students resource");
    assert_eq!(student_resource["many_to_many_relations"][0]["through_table"], "student_courses");
    assert_eq!(student_resource["many_to_many_relations"][0]["target_table"], "courses");

    assert!(payload["edges"].as_array().expect("edges").iter().any(|edge| {
        edge["kind"] == "many_to_many"
            && edge["source_table"] == "courses"
            && edge["target_table"] == "students"
            && edge["through_table"] == "student_courses"
    }));
}
