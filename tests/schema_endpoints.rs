use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct ChildGuard {
    child: Child,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

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

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg("127.0.0.1:3025")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3025", Duration::from_secs(5));

    let schema_response = http_request("127.0.0.1:3025", "GET", "/schema", None);
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

    let save_response = http_request("127.0.0.1:3025", "POST", "/schema", None);
    assert!(save_response.starts_with("HTTP/1.1 200 OK\r\n"), "{save_response}");

    let saved = temp.path().join("schema.json");
    assert!(saved.exists(), "schema.json should be written");
    let saved_payload: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&saved).expect("read schema.json"))
            .expect("saved schema json");
    assert_eq!(saved_payload["tables"]["student_courses"]["kind"], "relation");

    let root_response = http_request("127.0.0.1:3025", "GET", "/", None);
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

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg("127.0.0.1:3026")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3026", Duration::from_secs(5));

    let schema_response = http_request("127.0.0.1:3026", "GET", "/schema", None);
    assert!(schema_response.starts_with("HTTP/1.1 200 OK\r\n"), "{schema_response}");
    let schema_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&schema_response)).expect("schema json");
    assert_eq!(schema_payload["tables"]["users"]["primary_key"], "user_id");
    assert_eq!(
        schema_payload["tables"]["posts"]["foreign_keys"]["author_ref"]["target_column"],
        "user_id"
    );
    assert_eq!(schema_payload["tables"]["posts"]["columns"]["title"]["column_type"], "string");

    let embed_response = http_request("127.0.0.1:3026", "GET", "/posts?embed=author_ref", None);
    assert!(embed_response.starts_with("HTTP/1.1 200 OK\r\n"), "{embed_response}");
    let embed_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&embed_response)).expect("embed json");
    assert_eq!(embed_payload[0]["author_ref"]["user_id"], 1);
    assert_eq!(embed_payload[0]["author_ref"]["name"], "Ada");

    let save_response = http_request("127.0.0.1:3026", "POST", "/schema", None);
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

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg("127.0.0.1:3027")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3027", Duration::from_secs(5));

    let schema_response = http_request("127.0.0.1:3027", "GET", "/schema", None);
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

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg("127.0.0.1:3029")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3029", Duration::from_secs(5));

    let infer_response = http_request("127.0.0.1:3029", "POST", "/schema/infer", None);
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

fn wait_for_server(addr: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;

    loop {
        match TcpStream::connect(addr) {
            Ok(_) => return,
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            Err(err) => panic!("server at {addr} did not start in time: {err}"),
        }
    }
}

fn http_request(addr: &str, method: &str, path: &str, body: Option<&str>) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect to server");
    let payload = body.unwrap_or("");

    let request = if body.is_some() {
        format!(
            "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            payload.len(),
            payload
        )
    } else {
        format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n")
    };

    stream.write_all(request.as_bytes()).expect("write request");

    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    response
}

fn parse_http_body(response: &str) -> &str {
    response.split("\r\n\r\n").nth(1).expect("http body")
}
