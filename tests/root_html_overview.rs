use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
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
fn root_serves_html_overview_for_browser_requests() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("users.json"),
        r#"[
  {"id": 1, "name": "Ada"},
  {"id": 2, "name": "Linus"}
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
  name varchar [not null]
}

Table posts {
  id int [pk]
  title varchar [not null]
  user_id int [ref: > users.id]
}
"#,
    )
    .expect("write schema");

    let bind_addr = reserve_bind_addr();
    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg(&bind_addr)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server(&bind_addr, Duration::from_secs(5));

    let response = http_get(&bind_addr, "/", Some("Accept: text/html,application/xhtml+xml\r\n"));

    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(response.contains("content-type: text/html; charset=utf-8"), "{response}");
    assert!(response.contains("<h1>Visual overview of your data</h1>"), "{response}");
    assert!(response.contains("Rules of paths"), "{response}");
    assert!(response.contains("id=\"overview-root\""), "{response}");
    assert!(response.contains("data-overview-endpoint=\"/overview.json\""), "{response}");
    assert!(response.contains("href=\"/assets/overview.css\""), "{response}");
    assert!(response.contains("src=\"/assets/overview.js\""), "{response}");
    assert!(response.contains("Each valid `*.json` filename becomes `/{resource}`."), "{response}");
    assert!(response.contains("data-resource=\"posts\""), "{response}");
    assert!(response.contains("data-resource=\"users\""), "{response}");
}

fn reserve_bind_addr() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind to an ephemeral local port");
    let addr = listener.local_addr().expect("read ephemeral bind address");
    format!("127.0.0.1:{}", addr.port())
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

fn http_get(addr: &str, path: &str, extra_headers: Option<&str>) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect to server");
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{}\r\n",
        extra_headers.unwrap_or("")
    );

    stream.write_all(request.as_bytes()).expect("write request");

    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");

    response
}
