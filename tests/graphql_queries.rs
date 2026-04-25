use std::{
    fs, thread,
    time::{Duration, Instant},
};

mod support;

use support::{
    ChildGuard, http_request, http_request_with_headers, parse_http_body, wait_for_http,
};

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

    let (_child, bind_addr) = spawn_server(temp.path(), false);

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

    let (_child, bind_addr) = spawn_server(temp.path(), false);

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

    let (_child, bind_addr) = spawn_server(temp.path(), false);

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

    let (_child, bind_addr) = spawn_server(temp.path(), false);

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

    let (_child, bind_addr) = spawn_server(temp.path(), false);

    let payload = graphql_json(
        &bind_addr,
        r#"{ student_courses { student_id course_id student { id name } course { id title } } }"#,
    );
    assert_eq!(payload["data"]["student_courses"][0]["student"]["name"], "Ada");
    assert_eq!(payload["data"]["student_courses"][0]["course"]["title"], "Math");
}

#[test]
fn graphql_exposes_derived_many_to_many_fields_and_deduplicates_results() {
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
  {"student_id": 1, "course_id": 10},
  {"student_id": 1, "course_id": 11},
  {"student_id": 2, "course_id": 11}
]
"#,
    )
    .expect("write relation");

    let (_child, bind_addr) = spawn_server(temp.path(), false);

    let payload = graphql_json(
        &bind_addr,
        r#"{ students { id name courses { id title } } courses { id title students { id name } } __type(name: "StudentsRecord") { fields { name } } }"#,
    );

    let student_fields = payload["data"]["__type"]["fields"].as_array().expect("student fields");
    assert!(student_fields.iter().any(|field| field["name"] == "courses"));

    let ada_courses = payload["data"]["students"][0]["courses"].as_array().expect("ada courses");
    assert_eq!(ada_courses.len(), 2, "{payload}");
    assert_eq!(ada_courses[0]["id"], 10);
    assert_eq!(ada_courses[1]["id"], 11);

    let math_students =
        payload["data"]["courses"][0]["students"].as_array().expect("math students");
    assert_eq!(math_students.len(), 1, "{payload}");
    assert_eq!(math_students[0]["name"], "Ada");
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

    let (_child, bind_addr) = spawn_server(temp.path(), false);

    let payload = graphql_json(&bind_addr, r#"{ profile { name theme settings } }"#);
    assert_eq!(payload["data"]["profile"]["name"], "Ada");
    assert_eq!(payload["data"]["profile"]["theme"], "dark");
    assert_eq!(payload["data"]["profile"]["settings"]["compact"], true);
}

#[test]
fn graphql_treats_schema_aware_object_resources_as_objects() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("schema.json"),
        r#"{
  "tables": {
    "profile": {
      "kind": "object",
      "columns": {
        "name": {"column_type": "string", "nullable": false},
        "age": {"column_type": "integer", "nullable": true}
      }
    }
  }
}
"#,
    )
    .expect("write schema");
    fs::write(
        temp.path().join("profile.json"),
        r#"{
  "name": "Ada",
  "age": 37,
  "nickname": "Byte",
  "settings": {"compact": true}
}
"#,
    )
    .expect("write profile");

    let (_child, bind_addr) = spawn_server(temp.path(), false);

    let payload = graphql_json(
        &bind_addr,
        r#"{ profile { name age nickname settings } __schema { queryType { fields { name } } } __type(name: "ProfileObject") { fields { name type { kind name ofType { kind name } } } } }"#,
    );
    assert_eq!(payload["data"]["profile"]["name"], "Ada");
    assert_eq!(payload["data"]["profile"]["age"], 37);
    assert_eq!(payload["data"]["profile"]["nickname"], "Byte");
    assert_eq!(payload["data"]["profile"]["settings"]["compact"], true);

    let query_fields =
        payload["data"]["__schema"]["queryType"]["fields"].as_array().expect("query fields");
    assert!(query_fields.iter().any(|field| field["name"] == "profile"));
    assert!(!query_fields.iter().any(|field| field["name"] == "profileQuery"));
    assert!(!query_fields.iter().any(|field| field["name"] == "profileById"));

    let type_fields = payload["data"]["__type"]["fields"].as_array().expect("type fields");
    let name_field = type_fields.iter().find(|field| field["name"] == "name").expect("name field");
    assert_eq!(name_field["type"]["kind"], "NON_NULL");
    assert_eq!(name_field["type"]["ofType"]["name"], "String");

    let age_field = type_fields.iter().find(|field| field["name"] == "age").expect("age field");
    assert_eq!(age_field["type"]["kind"], "SCALAR");
    assert_eq!(age_field["type"]["name"], "Int");
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

    let (_child, bind_addr) = spawn_server(temp.path(), false);

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

    let (_child, bind_addr) = spawn_server(temp.path(), false);

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

    let (_child, bind_addr) = spawn_server(temp.path(), false);

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
fn watcher_handles_file_add_delete_invalid_json_and_recovery() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let users_path = temp.path().join("users.json");
    let teams_path = temp.path().join("teams.json");
    fs::write(&users_path, "[{\"id\":1,\"name\":\"Ada\"}]\n").expect("write users");

    let (_child, bind_addr) = spawn_server(temp.path(), false);

    let initial_root = http_request(&bind_addr, "GET", "/", None);
    assert!(initial_root.contains("\"users\""), "{initial_root}");
    assert!(!initial_root.contains("\"teams\""), "{initial_root}");

    fs::write(&teams_path, "[{\"id\":10,\"name\":\"Core\"}]\n").expect("write teams");
    let added_root = wait_for_http(&bind_addr, "GET", "/", None, |response| {
        response.starts_with("HTTP/1.1 200 OK\r\n") && response.contains("\"teams\"")
    });
    assert!(added_root.contains("\"users\""), "{added_root}");

    fs::remove_file(&users_path).expect("delete users");
    let users_missing = wait_for_http(&bind_addr, "GET", "/users", None, |response| {
        response.starts_with("HTTP/1.1 404 Not Found\r\n")
    });
    assert!(users_missing.starts_with("HTTP/1.1 404 Not Found\r\n"), "{users_missing}");

    fs::write(&teams_path, "[{\"id\":10").expect("write invalid teams");
    let not_ready = wait_for_http(&bind_addr, "GET", "/readyz", None, |response| {
        response.starts_with("HTTP/1.1 503 Service Unavailable\r\n")
    });
    assert!(not_ready.contains("\"ready\":false"), "{not_ready}");

    fs::write(&teams_path, "[{\"id\":10,\"name\":\"Core\",\"city\":\"Berlin\"}]\n")
        .expect("fix teams");
    let recovered = wait_for_http(&bind_addr, "GET", "/readyz", None, |response| {
        response.starts_with("HTTP/1.1 200 OK\r\n")
    });
    assert!(recovered.contains("\"ready\":true"), "{recovered}");

    let teams_response = wait_for_http(&bind_addr, "GET", "/teams", None, |response| {
        response.starts_with("HTTP/1.1 200 OK\r\n") && response.contains("\"city\":\"Berlin\"")
    });
    assert!(teams_response.contains("\"name\":\"Core\""), "{teams_response}");
}

#[test]
fn graphql_queries_work_in_readonly_mode_and_mutations_are_rejected() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("users.json"), "[{\"id\":1,\"name\":\"Ada\"}]\n")
        .expect("write users");

    let (_child, bind_addr) = spawn_server(temp.path(), true);

    let query_payload = graphql_json(&bind_addr, r#"{ users { id name } }"#);
    assert_eq!(query_payload["data"]["users"][0]["name"], "Ada");

    let mutation_payload =
        graphql_json(&bind_addr, r#"mutation { addUser(name: "Grace") { id } }"#);
    assert!(mutation_payload.get("errors").is_some(), "{mutation_payload}");
}

fn spawn_server(folder: &std::path::Path, readonly: bool) -> (ChildGuard, String) {
    support::spawn_folder_server(folder, readonly)
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
