use std::{
    fs,
    io::Read,
    net::TcpStream,
    thread,
    time::{Duration, Instant},
};

#[path = "../test_support/mod.rs"]
mod support;

use support::{
    http_request, http_request_with_headers, open_sse_stream, spawn_folder_server,
    spawn_folder_server_with_args,
};

#[test]
fn supports_all_array_resource_routes() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("posts.json"),
        r#"[
  {"id": 1, "title": "hello"}
]
"#,
    )
    .expect("write posts");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let get_collection = http_request(&bind_addr, "GET", "/posts", None);
    assert!(get_collection.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(get_collection.contains("\"title\":\"hello\""));

    let get_item = http_request(&bind_addr, "GET", "/posts/1", None);
    assert!(get_item.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(get_item.contains("\"id\":1"));

    let post_item = http_request(&bind_addr, "POST", "/posts", Some(r#"{"title":"new post"}"#));
    assert!(post_item.starts_with("HTTP/1.1 201 Created\r\n"));
    assert!(post_item.contains("\"id\":2"));
    let after_post: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(temp.path().join("posts.json")).expect("read posts"),
    )
    .expect("posts json");
    assert_eq!(
        after_post,
        serde_json::json!([
            {"id": 1, "title": "hello"},
            {"id": 2, "title": "new post"}
        ])
    );

    let put_item = http_request(&bind_addr, "PUT", "/posts/2", Some(r#"{"title":"updated"}"#));
    assert!(put_item.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(put_item.contains("\"title\":\"updated\""));
    let after_put: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(temp.path().join("posts.json")).expect("read posts"),
    )
    .expect("posts json");
    assert_eq!(
        after_put,
        serde_json::json!([
            {"id": 1, "title": "hello"},
            {"id": 2, "title": "updated"}
        ])
    );

    let patch_item = http_request(&bind_addr, "PATCH", "/posts/2", Some(r#"{"status":"draft"}"#));
    assert!(patch_item.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(patch_item.contains("\"status\":\"draft\""));
    let after_patch: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(temp.path().join("posts.json")).expect("read posts"),
    )
    .expect("posts json");
    assert_eq!(
        after_patch,
        serde_json::json!([
            {"id": 1, "title": "hello"},
            {"id": 2, "title": "updated", "status": "draft"}
        ])
    );

    let delete_item = http_request(&bind_addr, "DELETE", "/posts/2", None);
    assert!(delete_item.starts_with("HTTP/1.1 204 No Content\r\n"));
    let after_delete: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(temp.path().join("posts.json")).expect("read posts"),
    )
    .expect("posts json");
    assert_eq!(after_delete, serde_json::json!([{ "id": 1, "title": "hello" }]));
}

#[test]
fn supports_all_object_resource_routes() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("profile.json"),
        r#"{"name":"Ada","theme":"dark"}
"#,
    )
    .expect("write profile");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let get_profile = http_request(&bind_addr, "GET", "/profile", None);
    assert!(get_profile.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(get_profile.contains("\"theme\":\"dark\""));

    let put_profile =
        http_request(&bind_addr, "PUT", "/profile", Some(r#"{"name":"Grace","theme":"light"}"#));
    assert!(put_profile.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(put_profile.contains("\"name\":\"Grace\""));
    let after_put: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(temp.path().join("profile.json")).expect("read profile"),
    )
    .expect("profile json");
    assert_eq!(after_put, serde_json::json!({"name": "Grace", "theme": "light"}));

    let patch_profile =
        http_request(&bind_addr, "PATCH", "/profile", Some(r#"{"theme":"solarized"}"#));
    assert!(patch_profile.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(patch_profile.contains("\"theme\":\"solarized\""));
    let after_patch: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(temp.path().join("profile.json")).expect("read profile"),
    )
    .expect("profile json");
    assert_eq!(after_patch, serde_json::json!({"name": "Grace", "theme": "solarized"}));
}

#[test]
fn edit_suffix_serves_patch_editor_for_resources_and_items() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("posts.json"),
        r#"[{"id":1,"title":"hello"}]
"#,
    )
    .expect("write posts");
    fs::write(temp.path().join("profile.json"), r#"{"name":"Ada","theme":"dark"}"#)
        .expect("write profile");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let item_editor = http_request(&bind_addr, "GET", "/posts/1/edit", None);
    assert!(item_editor.starts_with("HTTP/1.1 200 OK\r\n"), "{item_editor}");
    assert!(item_editor.contains("content-type: text/html; charset=utf-8"), "{item_editor}");
    assert!(item_editor.contains("<title>Edit /posts/1</title>"), "{item_editor}");
    assert!(item_editor.contains("const targetPath = \"/posts/1\";"), "{item_editor}");
    assert!(item_editor.contains("method: 'PATCH'"), "{item_editor}");

    let object_editor = http_request(&bind_addr, "GET", "/profile/edit", None);
    assert!(object_editor.starts_with("HTTP/1.1 200 OK\r\n"), "{object_editor}");
    assert!(object_editor.contains("<title>Edit /profile</title>"), "{object_editor}");
    assert!(object_editor.contains("const targetPath = \"/profile\";"), "{object_editor}");

    let item_route_still_returns_json = http_request(&bind_addr, "GET", "/posts/1", None);
    assert!(
        item_route_still_returns_json.starts_with("HTTP/1.1 200 OK\r\n"),
        "{item_route_still_returns_json}"
    );
    assert!(item_route_still_returns_json.contains("\"title\":\"hello\""));
}

#[test]
fn create_suffix_serves_item_creation_form_for_array_resources() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("schema.json"),
        r#"{
  "tables": {
    "posts": {
      "primary_key": "id",
      "columns": {
        "id": {"column_type": "integer", "nullable": false},
        "title": {"column_type": "string", "nullable": false},
        "published": {"column_type": "boolean", "nullable": true}
      }
    }
  }
}
"#,
    )
    .expect("write schema");
    fs::write(
        temp.path().join("posts.json"),
        r#"[{"id":1,"title":"hello","published":false}]
"#,
    )
    .expect("write posts");
    fs::write(temp.path().join("profile.json"), r#"{"name":"Ada","theme":"dark"}"#)
        .expect("write profile");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let create_form = http_request(&bind_addr, "GET", "/posts/create", None);
    assert!(create_form.starts_with("HTTP/1.1 200 OK\r\n"), "{create_form}");
    assert!(create_form.contains("content-type: text/html; charset=utf-8"), "{create_form}");
    assert!(create_form.contains("<title>Create /posts</title>"), "{create_form}");
    assert!(create_form.contains("POST /posts"), "{create_form}");
    assert!(create_form.contains("const targetPath = \"/posts\";"), "{create_form}");
    assert!(create_form.contains("\"name\":\"title\""), "{create_form}");
    assert!(create_form.contains("\"field_type\":\"boolean\""), "{create_form}");
    assert!(create_form.contains("method: 'POST'"), "{create_form}");

    let object_create = http_request(&bind_addr, "GET", "/profile/create", None);
    assert!(object_create.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{object_create}");
    assert!(
        object_create.contains("Create forms require a JSON array resource"),
        "{object_create}"
    );

    let post_create_item_route = http_request(&bind_addr, "GET", "/posts/create/edit", None);
    assert!(post_create_item_route.starts_with("HTTP/1.1 200 OK\r\n"), "{post_create_item_route}");
    assert!(post_create_item_route.contains("<title>Edit /posts/create</title>"));
}

#[test]
fn schema_aware_object_resources_validate_get_put_and_patch_routes() {
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
        r#"{"name":"Ada","age":37,"nickname":"Byte"}
"#,
    )
    .expect("write profile");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let get_profile = http_request(&bind_addr, "GET", "/profile", None);
    assert!(get_profile.starts_with("HTTP/1.1 200 OK\r\n"), "{get_profile}");
    assert!(get_profile.contains("\"name\":\"Ada\""), "{get_profile}");

    let misuse = http_request(&bind_addr, "GET", "/profile?page=1", None);
    assert!(misuse.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{misuse}");
    assert!(
        misuse.contains(
            "Filtering, sorting, pagination, and embedding require a JSON array resource"
        ),
        "{misuse}"
    );

    let invalid_put =
        http_request(&bind_addr, "PUT", "/profile", Some(r#"{"name":"Grace","age":"old"}"#));
    assert!(invalid_put.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{invalid_put}");
    assert!(invalid_put.contains("Resource 'profile' has invalid type for 'age'"), "{invalid_put}");

    let missing_put = http_request(&bind_addr, "PUT", "/profile", Some(r#"{"age":41}"#));
    assert!(missing_put.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{missing_put}");
    assert!(
        missing_put.contains("Resource 'profile' is missing non-null column 'name'"),
        "{missing_put}"
    );

    let invalid_patch = http_request(&bind_addr, "PATCH", "/profile", Some(r#"{"age":"old"}"#));
    assert!(invalid_patch.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{invalid_patch}");
    assert!(
        invalid_patch.contains("Resource 'profile' has invalid type for 'age'"),
        "{invalid_patch}"
    );

    let valid_patch = http_request(&bind_addr, "PATCH", "/profile", Some(r#"{"age":38}"#));
    assert!(valid_patch.starts_with("HTTP/1.1 200 OK\r\n"), "{valid_patch}");
    assert!(valid_patch.contains("\"age\":38"), "{valid_patch}");
    assert!(valid_patch.contains("\"nickname\":\"Byte\""), "{valid_patch}");
    let persisted: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(temp.path().join("profile.json")).expect("read profile"),
    )
    .expect("profile json");
    assert_eq!(persisted["age"], 38);
    assert_eq!(persisted["nickname"], "Byte");
}

#[test]
fn array_routes_reject_missing_ids_and_non_object_payloads_without_touching_disk() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let posts_path = temp.path().join("posts.json");
    fs::write(
        &posts_path,
        r#"[
  {"id": 1, "title": "hello"}
]
"#,
    )
    .expect("write posts");
    let original = fs::read_to_string(&posts_path).expect("read posts");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let bad_post = http_request(&bind_addr, "POST", "/posts", Some(r#"["bad"]"#));
    assert!(bad_post.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{bad_post}");
    assert!(bad_post.contains("Payload must be a JSON object"), "{bad_post}");

    let missing_put = http_request(&bind_addr, "PUT", "/posts/99", Some(r#"{"title":"ghost"}"#));
    assert!(missing_put.starts_with("HTTP/1.1 404 Not Found\r\n"), "{missing_put}");

    let bad_patch = http_request(&bind_addr, "PATCH", "/posts/1", Some(r#"["bad"]"#));
    assert!(bad_patch.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{bad_patch}");
    assert!(bad_patch.contains("Payload must be a JSON object"), "{bad_patch}");

    let missing_delete = http_request(&bind_addr, "DELETE", "/posts/99", None);
    assert!(missing_delete.starts_with("HTTP/1.1 404 Not Found\r\n"), "{missing_delete}");

    let final_posts = fs::read_to_string(&posts_path).expect("read posts");
    assert_eq!(final_posts, original);
}

#[test]
fn graphql_endpoint_serves_graphiql() {
    let temp = tempfile::tempdir().expect("create temp directory");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let graphql = http_request_with_headers(
        &bind_addr,
        "GET",
        "/graphql",
        Some("Accept: text/html,application/xhtml+xml\r\n"),
        None,
    );
    assert!(graphql.starts_with("HTTP/1.1 200 OK\r\n"), "{graphql}");
    assert!(graphql.contains("content-type: text/html; charset=utf-8"), "{graphql}");
    assert!(graphql.contains("GraphiQL"), "{graphql}");
}

#[test]
fn logging_writes_each_request_when_enabled() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("posts.json"), "[]\n").expect("write posts");
    fs::write(temp.path().join("users.json"), "[]\n").expect("write users");
    let log_path = temp.path().join("requests.log");

    let (_child, bind_addr) = spawn_folder_server_with_args(
        temp.path(),
        &["--log", "--logname", log_path.to_str().expect("utf-8 path")],
    );

    let _ = http_request(&bind_addr, "GET", "/posts", None);
    let _ = http_request(&bind_addr, "POST", "/posts", Some(r#"{"title":"logged"}"#));
    let _ = http_request(&bind_addr, "GET", "/sql?q=SELECT%20*%20FROM%20users%20LIMIT%201", None);
    let _ = http_request(
        &bind_addr,
        "GET",
        "/export.sql?q=SELECT%20*%20FROM%20users%20LIMIT%201",
        None,
    );

    thread::sleep(Duration::from_millis(150));

    let log_contents = fs::read_to_string(log_path).expect("read request log");
    assert!(log_contents.contains("GET /posts 200"), "{log_contents}");
    assert!(log_contents.contains("POST /posts 201"), "{log_contents}");
    assert!(log_contents.contains("GET /sql 200 query_hash="), "{log_contents}");
    assert!(log_contents.contains("GET /export.sql 200 query_hash="), "{log_contents}");
}

#[test]
fn auth_and_ops_endpoints_respect_bearer_token_and_cors() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("users.json"), "[{\"id\":1,\"name\":\"Ada\"}]\n")
        .expect("write users");

    let (_child, bind_addr) = spawn_folder_server_with_args(
        temp.path(),
        &["--auth-token", "secret", "--cors-origin", "http://example.com"],
    );

    let unauthorized = http_request(&bind_addr, "GET", "/users", None);
    assert!(unauthorized.starts_with("HTTP/1.1 401 Unauthorized\r\n"), "{unauthorized}");

    let authorized = http_request_with_headers(
        &bind_addr,
        "GET",
        "/users",
        Some("Authorization: Bearer secret\r\nOrigin: http://example.com\r\n"),
        None,
    );
    assert!(authorized.starts_with("HTTP/1.1 200 OK\r\n"), "{authorized}");
    assert!(authorized.contains("access-control-allow-origin: http://example.com"), "{authorized}");

    let health = http_request(&bind_addr, "GET", "/healthz", None);
    assert!(health.starts_with("HTTP/1.1 200 OK\r\n"), "{health}");

    let ready = http_request(&bind_addr, "GET", "/readyz", None);
    assert!(ready.starts_with("HTTP/1.1 200 OK\r\n"), "{ready}");

    let metrics = http_request(&bind_addr, "GET", "/metrics", None);
    assert!(metrics.starts_with("HTTP/1.1 200 OK\r\n"), "{metrics}");
    assert!(metrics.contains("dirbase_requests_total"), "{metrics}");
    assert!(metrics.contains("dirbase_auth_failures_total"), "{metrics}");
}

#[test]
fn events_endpoint_streams_resource_and_schema_changes() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("users.json"), "[{\"id\":1,\"name\":\"Ada\"}]\n")
        .expect("write users");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);
    let mut stream = open_sse_stream(&bind_addr, "/events");

    let create = http_request(&bind_addr, "POST", "/users", Some(r#"{"name":"Grace"}"#));
    assert!(create.starts_with("HTTP/1.1 201 Created\r\n"), "{create}");

    let payload = read_stream_until(&mut stream, "event: schema_changed", Duration::from_secs(5));
    assert!(payload.contains("event: resource_changed"), "{payload}");
    assert!(payload.contains("event: overview_changed"), "{payload}");
}

#[test]
fn readonly_mode_rejects_resource_mutation_routes() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("posts.json"), "[{\"id\":1,\"title\":\"hello\"}]\n")
        .expect("write posts");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), true);

    let scenarios = [
        ("POST", "/posts", Some(r#"{"title":"new"}"#)),
        ("PUT", "/posts", Some(r#"{"title":"replace"}"#)),
        ("PATCH", "/posts", Some(r#"{"title":"patch"}"#)),
        ("PUT", "/posts/1", Some(r#"{"title":"replace"}"#)),
        ("PATCH", "/posts/1", Some(r#"{"title":"patch"}"#)),
        ("DELETE", "/posts/1", None),
    ];

    for (method, path, body) in scenarios {
        let response = http_request(&bind_addr, method, path, body);
        assert!(
            response.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"),
            "{method} {path}: {response}"
        );
    }
}

#[test]
fn cors_preflight_respects_allowed_and_disallowed_origins() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("users.json"), "[]\n").expect("write users");

    let (_child, bind_addr) =
        spawn_folder_server_with_args(temp.path(), &["--cors-origin", "http://example.com"]);

    let allowed = http_request_with_headers(
        &bind_addr,
        "OPTIONS",
        "/users",
        Some("Origin: http://example.com\r\nAccess-Control-Request-Method: POST\r\n"),
        None,
    );
    assert!(allowed.starts_with("HTTP/1.1 204 No Content\r\n"), "{allowed}");
    assert!(allowed.contains("access-control-allow-origin: http://example.com"), "{allowed}");

    let denied = http_request_with_headers(
        &bind_addr,
        "OPTIONS",
        "/users",
        Some("Origin: http://evil.example\r\nAccess-Control-Request-Method: POST\r\n"),
        None,
    );
    assert!(denied.starts_with("HTTP/1.1 204 No Content\r\n"), "{denied}");
    assert!(!denied.contains("access-control-allow-origin"), "{denied}");
}

#[test]
fn body_limit_rejects_oversized_json_payload() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("posts.json"), "[{\"id\":1,\"title\":\"hello\"}]\n")
        .expect("write posts");
    fs::write(temp.path().join("profile.json"), "{\"theme\":\"dark\"}\n").expect("write profile");

    let (_child, bind_addr) =
        spawn_folder_server_with_args(temp.path(), &["--max-body-bytes", "32"]);

    for (method, path) in [("POST", "/posts"), ("PATCH", "/posts/1"), ("PUT", "/profile")] {
        let response = http_request(
            &bind_addr,
            method,
            path,
            Some(r#"{"title":"this payload is intentionally too large"}"#),
        );
        assert!(
            response.starts_with("HTTP/1.1 413 Payload Too Large\r\n"),
            "{method} {path}: {response}"
        );
    }
}

#[test]
fn max_per_page_rejects_excessive_collection_queries() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("posts.json"),
        r#"[
  {"id":1,"title":"one"},
  {"id":2,"title":"two"}
]
"#,
    )
    .expect("write posts");

    let (_child, bind_addr) = spawn_folder_server_with_args(temp.path(), &["--max-per-page", "1"]);

    let allowed = http_request(&bind_addr, "GET", "/posts?page=1&per_page=1", None);
    assert!(allowed.starts_with("HTTP/1.1 200 OK\r\n"), "{allowed}");

    let rejected = http_request(&bind_addr, "GET", "/posts?page=1&per_page=2", None);
    assert!(rejected.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{rejected}");
    assert!(rejected.contains("\"code\":\"limit_exceeded\""), "{rejected}");
    assert!(rejected.contains("per_page exceeds configured max of 1"), "{rejected}");
}

#[test]
fn object_and_array_route_misuse_returns_clear_errors() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let posts_path = temp.path().join("posts.json");
    fs::write(&posts_path, "[{\"id\":1,\"title\":\"hello\"}]\n").expect("write posts");
    fs::write(temp.path().join("profile.json"), "{\"name\":\"Ada\"}\n").expect("write profile");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let array_via_object_route =
        http_request(&bind_addr, "PUT", "/posts", Some(r#"{"title":"replace"}"#));
    assert!(
        array_via_object_route.starts_with("HTTP/1.1 400 Bad Request\r\n"),
        "{array_via_object_route}"
    );
    assert!(array_via_object_route.contains("Payload and resource must be JSON objects"));
    let persisted = fs::read_to_string(&posts_path).expect("read posts");
    assert!(persisted.contains("\"title\":\"hello\""), "{persisted}");

    let object_via_item_route = http_request(&bind_addr, "GET", "/profile/1", None);
    assert!(
        object_via_item_route.starts_with("HTTP/1.1 400 Bad Request\r\n"),
        "{object_via_item_route}"
    );
    assert!(
        object_via_item_route.contains("Resource is not a JSON array"),
        "{object_via_item_route}"
    );
}

fn read_stream_until(stream: &mut TcpStream, needle: &str, timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    let mut buffer = String::new();
    let mut chunk = [0u8; 4096];
    while Instant::now() < deadline {
        match stream.read(&mut chunk) {
            Ok(0) => thread::sleep(Duration::from_millis(25)),
            Ok(read) => {
                buffer.push_str(&String::from_utf8_lossy(&chunk[..read]));
                if buffer.contains(needle) {
                    return buffer;
                }
            }
            Err(err)
                if err.kind() == std::io::ErrorKind::WouldBlock
                    || err.kind() == std::io::ErrorKind::TimedOut =>
            {
                thread::sleep(Duration::from_millis(25));
            }
            Err(err) => panic!("failed to read stream: {err}"),
        }
    }
    panic!("stream did not contain '{needle}': {buffer}");
}
