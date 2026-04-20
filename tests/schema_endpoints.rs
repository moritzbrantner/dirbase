use std::fs;

mod support;

use support::{http_request, parse_http_body, spawn_folder_server};

#[test]
fn schema_endpoint_infers_tables_and_can_persist_schema_json() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("students.json"),
        r#"[
  {"id": 1, "name": "Ada"},
  {"id": 2, "name": "Grace"}
]
"#,
    )
    .expect("write students");
    fs::write(
        temp.path().join("courses.json"),
        r#"[
  {"id": 10, "title": "Math"},
  {"id": 11, "title": "CS"}
]
"#,
    )
    .expect("write courses");
    fs::write(
        temp.path().join("student_courses.json"),
        r#"[
  {"student_id": 1, "course_id": 10},
  {"student_id": 2, "course_id": 11}
]
"#,
    )
    .expect("write relation table");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let schema_response = http_request(&bind_addr, "GET", "/schema", None);
    assert!(schema_response.starts_with("HTTP/1.1 200 OK\r\n"), "{schema_response}");
    let schema_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&schema_response)).expect("schema json");
    assert_eq!(schema_payload["tables"]["students"]["kind"], "object");
    assert_eq!(schema_payload["tables"]["students"]["primary_key"], "id");
    assert_eq!(schema_payload["tables"]["student_courses"]["kind"], "relation");
    assert_eq!(
        schema_payload["tables"]["student_courses"]["foreign_keys"]["student_id"]["target_table"],
        "students"
    );
    assert_eq!(
        schema_payload["tables"]["student_courses"]["foreign_keys"]["course_id"]["target_table"],
        "courses"
    );

    let save_response = http_request(&bind_addr, "POST", "/schema", None);
    assert!(save_response.starts_with("HTTP/1.1 200 OK\r\n"), "{save_response}");

    let saved = temp.path().join("schema.json");
    assert!(saved.exists(), "schema.json should be written");
    let saved_payload: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&saved).expect("read schema.json"))
            .expect("saved schema json");
    assert_eq!(saved_payload["tables"]["student_courses"]["kind"], "relation");

    let root_response = http_request(&bind_addr, "GET", "/", None);
    assert!(root_response.starts_with("HTTP/1.1 200 OK\r\n"), "{root_response}");
    let root_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&root_response)).expect("root json");
    let resources = root_payload["resources"].as_array().expect("resources array");
    assert!(
        !resources.iter().any(|value| value == "schema"),
        "saved schema.json must not become a normal resource: {root_payload}"
    );
}

#[test]
fn schema_json_overlay_merges_inferred_columns_and_manual_relations() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("schema.json"),
        r#"{
  "tables": {
    "users": {
      "primary_key": "user_id"
    },
    "posts": {
      "foreign_keys": {
        "author_ref": {
          "target_table": "users",
          "target_column": "user_id"
        }
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
    fs::write(
        temp.path().join("posts.json"),
        r#"[
  {"id": 1, "title": "Hello", "author_ref": 1}
]
"#,
    )
    .expect("write posts");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let schema_response = http_request(&bind_addr, "GET", "/schema", None);
    assert!(schema_response.starts_with("HTTP/1.1 200 OK\r\n"), "{schema_response}");
    let schema_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&schema_response)).expect("schema json");
    assert_eq!(schema_payload["tables"]["users"]["primary_key"], "user_id");
    assert_eq!(
        schema_payload["tables"]["posts"]["foreign_keys"]["author_ref"]["target_column"],
        "user_id"
    );
    assert_eq!(schema_payload["tables"]["posts"]["columns"]["title"]["column_type"], "string");

    let embed_response = http_request(&bind_addr, "GET", "/posts?embed=author_ref", None);
    assert!(embed_response.starts_with("HTTP/1.1 200 OK\r\n"), "{embed_response}");
    let embed_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&embed_response)).expect("embed json");
    assert_eq!(embed_payload[0]["author_ref"]["user_id"], 1);
    assert_eq!(embed_payload[0]["author_ref"]["name"], "Ada");

    let save_response = http_request(&bind_addr, "POST", "/schema", None);
    assert!(save_response.starts_with("HTTP/1.1 200 OK\r\n"), "{save_response}");
    let saved_payload: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(temp.path().join("schema.json")).expect("read"))
            .expect("saved schema");
    assert_eq!(saved_payload["tables"]["users"]["columns"]["name"]["column_type"], "string");
    assert_eq!(
        saved_payload["tables"]["posts"]["foreign_keys"]["author_ref"]["target_column"],
        "user_id"
    );
}

#[test]
fn schema_json_takes_precedence_over_schema_dbml() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("schema.json"),
        r#"{
  "tables": {
    "users": {
      "primary_key": "user_id"
    },
    "posts": {
      "foreign_keys": {
        "author_ref": {
          "target_table": "users",
          "target_column": "user_id"
        }
      }
    }
  }
}
"#,
    )
    .expect("write schema json");
    fs::write(
        temp.path().join("schema.dbml"),
        r#"
Table users {
  id int [pk]
}

Table posts {
  id int [pk]
  author_id int [ref: > users.id]
}
"#,
    )
    .expect("write schema dbml");
    fs::write(
        temp.path().join("users.json"),
        r#"[
  {"user_id": 1, "name": "Ada"}
]
"#,
    )
    .expect("write users");
    fs::write(
        temp.path().join("posts.json"),
        r#"[
  {"id": 1, "author_ref": 1}
]
"#,
    )
    .expect("write posts");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let schema_response = http_request(&bind_addr, "GET", "/schema", None);
    assert!(schema_response.starts_with("HTTP/1.1 200 OK\r\n"), "{schema_response}");
    let schema_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&schema_response)).expect("schema json");
    assert_eq!(schema_payload["tables"]["users"]["primary_key"], "user_id");
    assert!(
        schema_payload["tables"]["posts"]["foreign_keys"].get("author_ref").is_some(),
        "{schema_payload}"
    );
}

#[test]
fn schema_infer_endpoint_writes_fresh_inferred_schema_json() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("schema.json"),
        r#"{
  "tables": {
    "users": {
      "primary_key": "user_id"
    },
    "posts": {
      "foreign_keys": {
        "author_ref": {
          "target_table": "users",
          "target_column": "user_id"
        }
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
    fs::write(
        temp.path().join("posts.json"),
        r#"[
  {"id": 1, "title": "Hello", "author_ref": 1}
]
"#,
    )
    .expect("write posts");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let infer_response = http_request(&bind_addr, "POST", "/schema/infer", None);
    assert!(infer_response.starts_with("HTTP/1.1 200 OK\r\n"), "{infer_response}");

    let saved_payload: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(temp.path().join("schema.json")).expect("read"))
            .expect("saved schema");
    assert_eq!(saved_payload["tables"]["users"]["primary_key"], "user_id");
    assert!(
        saved_payload["tables"]["posts"]["foreign_keys"].get("author_ref").is_none(),
        "{saved_payload}"
    );
    assert_eq!(saved_payload["tables"]["posts"]["columns"]["author_ref"]["column_type"], "integer");
}

#[test]
fn schema_put_validates_and_persists_declared_schema() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let schema_path = temp.path().join("schema.json");
    let original_schema = r#"{
  "tables": {
    "users": {
      "primary_key": "id"
    }
  }
}
"#;
    fs::write(&schema_path, original_schema).expect("write original schema");
    fs::write(temp.path().join("users.json"), "[{\"id\":1,\"name\":\"Ada\"}]\n")
        .expect("write users");
    fs::write(temp.path().join("posts.json"), "[{\"id\":1,\"author_id\":1,\"title\":\"Hello\"}]\n")
        .expect("write posts");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let invalid = http_request(
        &bind_addr,
        "PUT",
        "/schema",
        Some(
            r#"{
  "tables": {
    "posts": {
      "foreign_keys": {
        "author_id": {
          "target_table": "missing",
          "target_column": "id"
        }
      }
    }
  }
}"#,
        ),
    );
    assert!(invalid.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{invalid}");
    let after_invalid = fs::read_to_string(&schema_path).expect("read schema after invalid put");
    assert_eq!(after_invalid, original_schema);

    let valid = http_request(
        &bind_addr,
        "PUT",
        "/schema",
        Some(
            r#"{
  "tables": {
    "posts": {
      "foreign_keys": {
        "author_id": {
          "target_table": "users",
          "target_column": "id"
        }
      }
    }
  }
}"#,
        ),
    );
    assert!(valid.starts_with("HTTP/1.1 200 OK\r\n"), "{valid}");

    let saved = fs::read_to_string(temp.path().join("schema.json")).expect("read schema");
    let saved_payload: serde_json::Value = serde_json::from_str(&saved).expect("schema json");
    assert_eq!(
        saved_payload["tables"]["posts"]["foreign_keys"]["author_id"]["target_table"],
        "users"
    );

    let schema_response = http_request(&bind_addr, "GET", "/schema", None);
    assert!(schema_response.starts_with("HTTP/1.1 200 OK\r\n"), "{schema_response}");
    let schema_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&schema_response)).expect("schema json");
    assert_eq!(
        schema_payload["tables"]["posts"]["foreign_keys"]["author_id"]["target_table"],
        "users"
    );
}
