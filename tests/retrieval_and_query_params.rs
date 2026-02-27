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

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg("127.0.0.1:3013")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3013", Duration::from_secs(5));

    let get_existing = http_get("127.0.0.1:3013", "/users/user-1");
    assert!(get_existing.starts_with("HTTP/1.1 200 OK\r\n"), "{get_existing}");
    let existing_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&get_existing)).expect("valid json body");
    assert_eq!(existing_payload["name"], "Ada");

    let get_missing = http_get("127.0.0.1:3013", "/users/user-99");
    assert!(
        get_missing.starts_with("HTTP/1.1 404 Not Found\r\n"),
        "{get_missing}"
    );
    assert!(
        get_missing.contains("\"error\":\"Item not found\""),
        "{get_missing}"
    );
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

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg("127.0.0.1:3014")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3014", Duration::from_secs(5));

    let per_page_only = http_get("127.0.0.1:3014", "/posts?_per_page=2");
    assert!(per_page_only.starts_with("HTTP/1.1 200 OK\r\n"), "{per_page_only}");
    let per_page_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&per_page_only)).expect("valid json body");
    assert_eq!(per_page_payload["first"], 1);
    assert_eq!(per_page_payload["last"], 2);
    assert_eq!(per_page_payload["prev"], serde_json::Value::Null);
    assert_eq!(per_page_payload["next"], 2);

    let page_only = http_get("127.0.0.1:3014", "/posts?_page=2");
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

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg("127.0.0.1:3015")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3015", Duration::from_secs(5));

    let bad_page = http_get("127.0.0.1:3015", "/posts?_page=0");
    assert!(bad_page.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{bad_page}");
    assert!(bad_page.contains("\"error\":\"'_page' must be greater than 0\""));

    let bad_operator = http_get("127.0.0.1:3015", "/posts?title:unknown=value");
    assert!(
        bad_operator.starts_with("HTTP/1.1 400 Bad Request\r\n"),
        "{bad_operator}"
    );
    assert!(
        bad_operator.contains("\"error\":\"Unsupported filter operator 'unknown' in 'title:unknown'\""),
        "{bad_operator}"
    );

    let bad_per_page = http_get("127.0.0.1:3015", "/posts?per_page=abc");
    assert!(
        bad_per_page.starts_with("HTTP/1.1 400 Bad Request\r\n"),
        "{bad_per_page}"
    );
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

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg("127.0.0.1:3016")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3016", Duration::from_secs(5));

    let page_two = http_get("127.0.0.1:3016", "/posts?page=2&per_page=2");
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
fn retrieval_returns_400_for_invalid_resource_name() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("users.json"), "[]\n").expect("write users");

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg("127.0.0.1:3017")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3017", Duration::from_secs(5));

    let response = http_get("127.0.0.1:3017", "/users..bad/1");
    assert!(
        response.starts_with("HTTP/1.1 400 Bad Request\r\n"),
        "{response}"
    );
    assert!(
        response.contains("\"error\":\"Resource name must only contain letters, numbers, underscore, and dash\""),
        "{response}"
    );
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

fn parse_http_body(response: &str) -> &str {
    response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .or_else(|| response.split_once("\n\n").map(|(_, body)| body))
        .expect("response should have body")
}

fn http_get(addr: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect to server");
    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");

    stream.write_all(request.as_bytes()).expect("write request");

    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");

    response
}
