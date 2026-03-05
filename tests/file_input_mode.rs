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
fn serves_json_server_style_single_file_input() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let db_path = temp.path().join("db.json");
    fs::write(
        &db_path,
        r#"{
  "users": [
    {"id": 1, "name": "Ada"}
  ],
  "settings": {"theme": "dark"}
}
"#,
    )
    .expect("write db file");

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--file")
        .arg(&db_path)
        .arg("--bind")
        .arg("127.0.0.1:3033")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3033", Duration::from_secs(5));

    let root_response = http_request("GET", "127.0.0.1:3033", "/", None);
    assert!(root_response.starts_with("HTTP/1.1 200 OK\r\n"), "{root_response}");
    let root_body: serde_json::Value =
        serde_json::from_str(parse_http_body(&root_response)).expect("root body json");
    let resources = root_body["resources"].as_array().expect("resources array");
    assert!(resources.iter().any(|resource| resource.as_str() == Some("users")));
    assert!(resources.iter().any(|resource| resource.as_str() == Some("settings")));

    let post_response = http_request("POST", "127.0.0.1:3033", "/users", Some(r#"{"name":"Lin"}"#));
    assert!(post_response.starts_with("HTTP/1.1 201 Created\r\n"), "{post_response}");

    let db_text = fs::read_to_string(&db_path).expect("read db file");
    let db: serde_json::Value = serde_json::from_str(&db_text).expect("db json");
    assert_eq!(db["users"].as_array().expect("users array").len(), 2);
    assert_eq!(db["settings"]["theme"], "dark");
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

fn http_request(method: &str, addr: &str, path: &str, body: Option<&str>) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect to server");

    let request = if let Some(body) = body {
        format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    } else {
        format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
    };

    stream.write_all(request.as_bytes()).expect("write request");

    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");

    response
}
