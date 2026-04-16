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
fn overview_json_returns_machine_readable_metadata_for_folder_mode() {
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
  name varchar
}

Table posts {
  id int [pk]
  title varchar
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

    let response = http_request("GET", &bind_addr, "/overview.json", None);
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(response.contains("content-type: application/json"), "{response}");

    let payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&response)).expect("overview json body");
    assert_eq!(payload["data_source_kind"], "folder");
    assert_eq!(payload["server_capabilities"]["readonly"], false);
    assert_eq!(payload["server_capabilities"]["resource_write"], true);
    assert_eq!(payload["server_capabilities"]["schema_write"], true);
    assert_eq!(payload["server_capabilities"]["schema_infer"], true);
    assert_eq!(payload["stats"]["resource_count"], 2);
    assert_eq!(payload["stats"]["relation_count"], 1);
    assert_eq!(payload["stats"]["total_rows"], 3);
    assert!(payload["resources"].as_array().expect("resources array").iter().any(
        |resource| resource["name"] == "posts"
            && resource["query_capabilities"]["pagination"] == true
            && resource["mutation_capabilities"]["create_item"] == true
            && resource["mutation_capabilities"]["update_item"] == true
            && resource["mutation_capabilities"]["delete_item"] == true
    ));
    assert!(payload["resources"].as_array().expect("resources array").iter().any(
        |resource| resource["name"] == "users"
            && resource["sample_item_id"] == "1"
            && resource["mutation_capabilities"]["replace_object"] == false
    ));
    assert_eq!(payload["edges"][0]["source_table"], "posts");
    assert_eq!(payload["edges"][0]["target_table"], "users");
}

#[test]
fn overview_json_describes_file_mode_and_assets_are_served() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let db_path = temp.path().join("db.json");
    fs::write(
        &db_path,
        r#"{
  "users": [
    {"id": 1, "name": "Ada"}
  ],
  "settings": {
    "theme": "warm"
  }
}
"#,
    )
    .expect("write db file");

    let bind_addr = reserve_bind_addr();
    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--file")
        .arg(&db_path)
        .arg("--bind")
        .arg(&bind_addr)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server(&bind_addr, Duration::from_secs(5));

    let overview_response = http_request("GET", &bind_addr, "/overview.json", None);
    assert!(overview_response.starts_with("HTTP/1.1 200 OK\r\n"), "{overview_response}");
    let payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&overview_response)).expect("overview body");
    assert_eq!(payload["data_source_kind"], "file");
    assert_eq!(
        payload["source_rule"],
        "Each valid top-level key in the JSON file becomes `/{resource}`."
    );
    assert!(payload["resources"].as_array().expect("resources array").iter().any(
        |resource| resource["name"] == "settings"
            && resource["kind"] == "object"
            && resource["mutation_capabilities"]["patch_object"] == true
            && resource["mutation_capabilities"]["replace_object"] == true
    ));

    let css_response = http_request("GET", &bind_addr, "/assets/overview.css", None);
    assert!(css_response.starts_with("HTTP/1.1 200 OK\r\n"), "{css_response}");
    assert!(css_response.contains("content-type: text/css; charset=utf-8"), "{css_response}");
    assert!(parse_http_body(&css_response).contains(".overview-page"), "{css_response}");

    let js_response = http_request("GET", &bind_addr, "/assets/overview.js", None);
    assert!(js_response.starts_with("HTTP/1.1 200 OK\r\n"), "{js_response}");
    assert!(js_response.contains("content-type: text/javascript; charset=utf-8"), "{js_response}");
    assert!(parse_http_body(&js_response).contains("overview-root"), "{js_response}");
}

#[test]
fn overview_json_reflects_readonly_capabilities() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("users.json"),
        r#"[
  {"id": 1, "name": "Ada"}
]
"#,
    )
    .expect("write users");
    fs::write(
        temp.path().join("settings.json"),
        r#"{"theme":"warm"}
"#,
    )
    .expect("write settings");

    let bind_addr = reserve_bind_addr();
    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg(&bind_addr)
        .arg("--readonly")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server(&bind_addr, Duration::from_secs(5));

    let response = http_request("GET", &bind_addr, "/overview.json", None);
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");

    let payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&response)).expect("overview json body");
    assert_eq!(payload["server_capabilities"]["readonly"], true);
    assert_eq!(payload["server_capabilities"]["resource_write"], false);
    assert_eq!(payload["server_capabilities"]["schema_write"], false);
    assert_eq!(payload["server_capabilities"]["schema_infer"], false);
    assert!(
        payload["resources"]
            .as_array()
            .expect("resources array")
            .iter()
            .any(|resource| resource["name"] == "users"
                && resource["mutation_capabilities"]["create_item"] == true)
    );
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

fn parse_http_body(response: &str) -> &str {
    response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .or_else(|| response.split_once("\n\n").map(|(_, body)| body))
        .expect("response should have body")
}

fn http_request(method: &str, addr: &str, path: &str, extra_headers: Option<&str>) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect to server");
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{}\r\n",
        extra_headers.unwrap_or("")
    );

    stream.write_all(request.as_bytes()).expect("write request");

    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");
    response
}
