use std::{
    io::{Read, Write},
    net::TcpStream,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use fake::{Fake, faker::name::en::Name};

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

    std::fs::write(
        users_path,
        serde_json::to_string_pretty(&users).expect("serialize fake users"),
    )
    .expect("write users json");

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg("127.0.0.1:3001")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3001", Duration::from_secs(5));

    let response = http_get("127.0.0.1:3001", "/users?role=admin&active=true");

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

    std::fs::write(
        users_path,
        serde_json::to_string_pretty(&users).expect("serialize users"),
    )
    .expect("write users json");

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg("127.0.0.1:3004")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3004", Duration::from_secs(5));

    let response = http_get("127.0.0.1:3004", "/users?sort=role,name");

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

    std::fs::write(
        posts_path,
        serde_json::to_string_pretty(&posts).expect("serialize posts"),
    )
    .expect("write posts json");

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg("127.0.0.1:3005")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3005", Duration::from_secs(5));

    let response = http_get(
        "127.0.0.1:3005",
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
