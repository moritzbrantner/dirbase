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
fn graphql_executes_basic_queries_and_serves_graphiql() {
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
  {"id": 1, "title": "Hello", "author_id": 1}
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
  author_id int [ref: > users.id]
}
"#,
    )
    .expect("write schema");

    let bind_addr = "127.0.0.1:3031".to_string();
    let child = spawn_server(temp.path(), &bind_addr, false);
    let _child = ChildGuard { child };

    wait_for_server(&bind_addr, Duration::from_secs(5));

    let html = http_request_with_headers(
        &bind_addr,
        "GET",
        "/graphql",
        Some("Accept: text/html,application/xhtml+xml\r\n"),
        None,
    );
    assert!(html.starts_with("HTTP/1.1 200 OK\r\n"), "{html}");
    assert!(html.contains("content-type: text/html; charset=utf-8"), "{html}");
    assert!(html.contains("GraphiQL"), "{html}");

    let payload = graphql_json(
        &bind_addr,
        r#"{ users { id name } posts { id title author_id author { id name } } postsById(id: "1") { id title author { name } } }"#,
    );
    assert_eq!(payload["data"]["users"][0]["name"], "Ada");
    assert_eq!(payload["data"]["posts"][0]["author"]["name"], "Ada");
    assert_eq!(payload["data"]["postsById"]["title"], "Hello");
}

#[test]
fn graphql_introspection_exposes_query_and_relation_fields() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("users.json"), "[{\"id\":1,\"name\":\"Ada\"}]\n")
        .expect("write users");
    fs::write(temp.path().join("posts.json"), "[{\"id\":1,\"title\":\"Hello\",\"author_id\":1}]\n")
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
  author_id int [ref: > users.id]
}
"#,
    )
    .expect("write schema");

    let bind_addr = "127.0.0.1:3032".to_string();
    let child = spawn_server(temp.path(), &bind_addr, false);
    let _child = ChildGuard { child };
    wait_for_server(&bind_addr, Duration::from_secs(5));

    let payload = graphql_json(
        &bind_addr,
        r#"{ __schema { queryType { fields { name } } } __type(name: "PostsRecord") { fields { name } } }"#,
    );
    let query_fields =
        payload["data"]["__schema"]["queryType"]["fields"].as_array().expect("query fields");
    assert!(query_fields.iter().any(|field| field["name"] == "posts"));
    assert!(query_fields.iter().any(|field| field["name"] == "postsById"));
    assert!(query_fields.iter().any(|field| field["name"] == "users"));
    assert!(query_fields.iter().any(|field| field["name"] == "usersById"));

    let type_fields = payload["data"]["__type"]["fields"].as_array().expect("type fields");
    assert!(type_fields.iter().any(|field| field["name"] == "author_id"));
    assert!(type_fields.iter().any(|field| field["name"] == "author"));
}

#[test]
fn graphql_collection_query_fields_support_filter_sort_and_pagination() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("users.json"),
        r#"[
  {"id": 1, "name": "Ada"},
  {"id": 2, "name": "Grace"},
  {"id": 3, "name": "Linus"}
]
"#,
    )
    .expect("write users");

    let bind_addr = "127.0.0.1:3040".to_string();
    let child = spawn_server(temp.path(), &bind_addr, false);
    let _child = ChildGuard { child };
    wait_for_server(&bind_addr, Duration::from_secs(5));

    let payload = graphql_json(
        &bind_addr,
        r#"{ usersQuery(filter: [{field: "name", operator: CONTAINS, value: "a"}], sort: [{field: "id", direction: DESC}], page: 1, perPage: 1) { page pages items data { id name } } }"#,
    );
    assert_eq!(payload["data"]["usersQuery"]["page"], 1);
    assert_eq!(payload["data"]["usersQuery"]["pages"], 2);
    assert_eq!(payload["data"]["usersQuery"]["items"], 2);
    assert_eq!(payload["data"]["usersQuery"]["data"][0]["id"], 2);
    assert_eq!(payload["data"]["usersQuery"]["data"][0]["name"], "Grace");
}

#[test]
fn graphql_respects_declared_primary_keys_and_manual_relations() {
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

    let bind_addr = "127.0.0.1:3033".to_string();
    let child = spawn_server(temp.path(), &bind_addr, false);
    let _child = ChildGuard { child };
    wait_for_server(&bind_addr, Duration::from_secs(5));

    let payload = graphql_json(
        &bind_addr,
        r#"{ usersById(id: "1") { user_id name } posts { author_ref author { user_id name } } }"#,
    );
    assert_eq!(payload["data"]["usersById"]["user_id"], 1);
    assert_eq!(payload["data"]["usersById"]["name"], "Ada");
    assert_eq!(payload["data"]["posts"][0]["author"]["user_id"], 1);
    assert_eq!(payload["data"]["posts"][0]["author"]["name"], "Ada");
}

#[test]
fn graphql_keeps_relation_tables_as_explicit_collections() {
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

    let bind_addr = "127.0.0.1:3034".to_string();
    let child = spawn_server(temp.path(), &bind_addr, false);
    let _child = ChildGuard { child };
    wait_for_server(&bind_addr, Duration::from_secs(5));

    let payload = graphql_json(
        &bind_addr,
        r#"{ student_courses { student_id course_id student { id name } course { id title } } }"#,
    );
    assert_eq!(payload["data"]["student_courses"][0]["student"]["name"], "Ada");
    assert_eq!(payload["data"]["student_courses"][0]["course"]["title"], "Math");
}

#[test]
fn graphql_types_top_level_object_resources() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("profile.json"),
        r#"{
  "name": "Ada",
  "theme": "dark",
  "settings": {"compact": true}
}
"#,
    )
    .expect("write profile");

    let bind_addr = "127.0.0.1:3035".to_string();
    let child = spawn_server(temp.path(), &bind_addr, false);
    let _child = ChildGuard { child };
    wait_for_server(&bind_addr, Duration::from_secs(5));

    let payload = graphql_json(&bind_addr, r#"{ profile { name theme settings } }"#);
    assert_eq!(payload["data"]["profile"]["name"], "Ada");
    assert_eq!(payload["data"]["profile"]["theme"], "dark");
    assert_eq!(payload["data"]["profile"]["settings"]["compact"], true);
}

#[test]
fn graphql_sanitizes_resource_and_field_names() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("schema.json"),
        r#"{
  "tables": {
    "users": {
      "primary_key": "user-id"
    },
    "blog-posts": {
      "foreign_keys": {
        "author_ref": {
          "target_table": "users",
          "target_column": "user-id"
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
  {"user-id": 1, "display-name": "Ada"}
]
"#,
    )
    .expect("write users");
    fs::write(
        temp.path().join("blog-posts.json"),
        r#"[
  {"id": 1, "post-title": "Hello", "author_ref": 1}
]
"#,
    )
    .expect("write posts");

    let bind_addr = "127.0.0.1:3036".to_string();
    let child = spawn_server(temp.path(), &bind_addr, false);
    let _child = ChildGuard { child };
    wait_for_server(&bind_addr, Duration::from_secs(5));

    let payload = graphql_json(
        &bind_addr,
        r#"{ blog_posts { id post_title author_ref author { user_id display_name } } }"#,
    );
    assert_eq!(payload["data"]["blog_posts"][0]["post_title"], "Hello");
    assert_eq!(payload["data"]["blog_posts"][0]["author"]["user_id"], 1);
    assert_eq!(payload["data"]["blog_posts"][0]["author"]["display_name"], "Ada");
}

#[test]
fn graphql_reports_name_collisions_with_clear_errors() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("team-a.json"), "[{\"id\":1}]\n").expect("write team-a");
    fs::write(temp.path().join("team_a.json"), "[{\"id\":2}]\n").expect("write team_a");

    let bind_addr = "127.0.0.1:3037".to_string();
    let child = spawn_server(temp.path(), &bind_addr, false);
    let _child = ChildGuard { child };
    wait_for_server(&bind_addr, Duration::from_secs(5));

    let response = graphql_raw(&bind_addr, r#"{ __schema { queryType { name } } }"#);
    assert!(response.starts_with("HTTP/1.1 500 Internal Server Error\r\n"), "{response}");
    assert!(
        response.contains("resource 'team-a'") && response.contains("resource 'team_a'"),
        "{response}"
    );
}

#[test]
fn graphql_schema_invalidates_after_rest_writes_and_file_watch_events() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let users_path = temp.path().join("users.json");
    fs::write(
        &users_path,
        r#"[
  {"id": 1, "name": "Ada"}
]
"#,
    )
    .expect("write users");

    let bind_addr = "127.0.0.1:3038".to_string();
    let child = spawn_server(temp.path(), &bind_addr, false);
    let _child = ChildGuard { child };
    wait_for_server(&bind_addr, Duration::from_secs(5));

    let before = graphql_json(&bind_addr, r#"{ __type(name: "UsersRecord") { fields { name } } }"#);
    assert!(
        !before["data"]["__type"]["fields"]
            .as_array()
            .expect("fields")
            .iter()
            .any(|field| field["name"] == "email")
    );

    let rest_create = http_request(
        &bind_addr,
        "POST",
        "/users",
        Some(r#"{"name":"Grace","email":"grace@example.com"}"#),
    );
    assert!(rest_create.starts_with("HTTP/1.1 201 Created\r\n"), "{rest_create}");

    let after_rest = graphql_json(&bind_addr, r#"{ users { id name email } }"#);
    assert_eq!(after_rest["data"]["users"][1]["email"], "grace@example.com");

    fs::write(
        &users_path,
        r#"[
  {"id": 1, "name": "Ada", "email": "ada@example.com", "city": "London"},
  {"id": 2, "name": "Grace", "email": "grace@example.com", "city": "New York"}
]
"#,
    )
    .expect("rewrite users");

    let payload =
        wait_for_graphql_json(&bind_addr, r#"{ users { id name email city } }"#, |payload| {
            payload["data"]["users"][0]["city"] == "London"
        });
    assert_eq!(payload["data"]["users"][1]["city"], "New York");
}

#[test]
fn graphql_queries_work_in_readonly_mode_and_mutations_are_rejected() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("users.json"), "[{\"id\":1,\"name\":\"Ada\"}]\n")
        .expect("write users");

    let bind_addr = "127.0.0.1:3039".to_string();
    let child = spawn_server(temp.path(), &bind_addr, true);
    let _child = ChildGuard { child };
    wait_for_server(&bind_addr, Duration::from_secs(5));

    let query_payload = graphql_json(&bind_addr, r#"{ users { id name } }"#);
    assert_eq!(query_payload["data"]["users"][0]["name"], "Ada");

    let mutation_payload =
        graphql_json(&bind_addr, r#"mutation { addUser(name: "Grace") { id } }"#);
    assert!(mutation_payload.get("errors").is_some(), "{mutation_payload}");
}

fn spawn_server(folder: &std::path::Path, bind_addr: &str, readonly: bool) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_folder-server"));
    command
        .arg("--folder")
        .arg(folder)
        .arg("--bind")
        .arg(bind_addr)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if readonly {
        command.arg("--readonly");
    }

    command.spawn().expect("start folder-server")
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

fn wait_for_graphql_json<F>(addr: &str, query: &str, predicate: F) -> serde_json::Value
where
    F: Fn(&serde_json::Value) -> bool,
{
    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        let payload = graphql_json(addr, query);
        if predicate(&payload) {
            return payload;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for graphql payload: {payload}");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn graphql_json(addr: &str, query: &str) -> serde_json::Value {
    let raw = graphql_raw(addr, query);
    assert!(raw.starts_with("HTTP/1.1 200 OK\r\n"), "{raw}");
    serde_json::from_str(parse_http_body(&raw)).expect("valid graphql json")
}

fn graphql_raw(addr: &str, query: &str) -> String {
    http_request(
        addr,
        "POST",
        "/graphql",
        Some(&format!(r#"{{"query":{}}}"#, serde_json::to_string(query).expect("query json"))),
    )
}

fn http_request(addr: &str, method: &str, path: &str, body: Option<&str>) -> String {
    http_request_with_headers(addr, method, path, None, body)
}

fn http_request_with_headers(
    addr: &str,
    method: &str,
    path: &str,
    extra_headers: Option<&str>,
    body: Option<&str>,
) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect to server");
    let payload = body.unwrap_or("");

    let request = if body.is_some() {
        format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
            extra_headers.unwrap_or(""),
            payload.len()
        )
    } else {
        format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{}\r\n",
            extra_headers.unwrap_or("")
        )
    };

    stream.write_all(request.as_bytes()).expect("write request");

    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    response
}

fn parse_http_body(response: &str) -> &str {
    response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or_else(|| panic!("missing HTTP body in response: {response}"))
}
