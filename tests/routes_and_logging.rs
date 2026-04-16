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

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg("127.0.0.1:3010")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3010", Duration::from_secs(5));

    let get_collection = http_request("127.0.0.1:3010", "GET", "/posts", None);
    assert!(get_collection.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(get_collection.contains("\"title\":\"hello\""));

    let get_item = http_request("127.0.0.1:3010", "GET", "/posts/1", None);
    assert!(get_item.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(get_item.contains("\"id\":1"));

    let post_item =
        http_request("127.0.0.1:3010", "POST", "/posts", Some(r#"{"title":"new post"}"#));
    assert!(post_item.starts_with("HTTP/1.1 201 Created\r\n"));
    assert!(post_item.contains("\"id\":2"));

    let put_item =
        http_request("127.0.0.1:3010", "PUT", "/posts/2", Some(r#"{"title":"updated"}"#));
    assert!(put_item.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(put_item.contains("\"title\":\"updated\""));

    let patch_item =
        http_request("127.0.0.1:3010", "PATCH", "/posts/2", Some(r#"{"status":"draft"}"#));
    assert!(patch_item.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(patch_item.contains("\"status\":\"draft\""));

    let delete_item = http_request("127.0.0.1:3010", "DELETE", "/posts/2", None);
    assert!(delete_item.starts_with("HTTP/1.1 204 No Content\r\n"));
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

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg("127.0.0.1:3011")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3011", Duration::from_secs(5));

    let get_profile = http_request("127.0.0.1:3011", "GET", "/profile", None);
    assert!(get_profile.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(get_profile.contains("\"theme\":\"dark\""));

    let put_profile = http_request(
        "127.0.0.1:3011",
        "PUT",
        "/profile",
        Some(r#"{"name":"Grace","theme":"light"}"#),
    );
    assert!(put_profile.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(put_profile.contains("\"name\":\"Grace\""));

    let patch_profile =
        http_request("127.0.0.1:3011", "PATCH", "/profile", Some(r#"{"theme":"solarized"}"#));
    assert!(patch_profile.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(patch_profile.contains("\"theme\":\"solarized\""));
}

#[test]
fn graphql_endpoint_serves_graphiql() {
    let temp = tempfile::tempdir().expect("create temp directory");

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

    let graphql = http_request_with_headers(
        "127.0.0.1:3013",
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
fn readonly_mode_rejects_post_for_sql() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("users.json"), "[]\n").expect("write users");

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg("127.0.0.1:3014")
        .arg("--readonly")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3014", Duration::from_secs(5));

    let post_sql =
        http_request("127.0.0.1:3014", "POST", "/sql", Some(r#"{"query":"SELECT * FROM users"}"#));
    assert!(post_sql.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"), "{post_sql}");
}

#[test]
fn logging_writes_each_request_when_enabled() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("posts.json"), "[]\n").expect("write posts");
    fs::write(temp.path().join("users.json"), "[]\n").expect("write users");
    let log_path = temp.path().join("requests.log");

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg("127.0.0.1:3012")
        .arg("--log")
        .arg("--logname")
        .arg(&log_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3012", Duration::from_secs(5));

    let _ = http_request("127.0.0.1:3012", "GET", "/posts", None);
    let _ = http_request("127.0.0.1:3012", "POST", "/posts", Some(r#"{"title":"logged"}"#));
    let _ =
        http_request("127.0.0.1:3012", "GET", "/sql?q=SELECT%20*%20FROM%20users%20LIMIT%201", None);
    let _ = http_request(
        "127.0.0.1:3012",
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
