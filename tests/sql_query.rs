use std::{
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
fn sql_get_works_in_readonly_mode() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let users_path = temp.path().join("users.json");

    let users = serde_json::json!([
        {"id": 1, "name": "Ada", "role": "admin", "age": 30},
        {"id": 2, "name": "Bob", "role": "member", "age": 20},
        {"id": 3, "name": "Cara", "role": "admin", "age": 25}
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
        .arg("127.0.0.1:3010")
        .arg("--readonly")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3010", Duration::from_secs(5));

    let response = http_get(
        "127.0.0.1:3010",
        "/sql?q=SELECT%20id,name%20FROM%20users%20WHERE%20role%20=%20'admin'%20ORDER%20BY%20id%20DESC%20LIMIT%201",
    );

    assert!(response.contains("200 OK"), "{response}");
    let body = parse_http_body(&response);
    let payload: serde_json::Value = serde_json::from_str(body).expect("json payload");

    assert_eq!(payload["dialect"], "generic");
    assert_eq!(payload["row_count"], 1);
    assert_eq!(payload["rows"][0]["id"], 3);
    assert_eq!(payload["rows"][0]["name"], "Cara");
}

#[test]
fn sql_post_rejects_non_select_and_unsupported_constructs() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let users_path = temp.path().join("users.json");

    let users = serde_json::json!([
        {"id": 1, "name": "Ada", "role": "admin"}
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
        .arg("127.0.0.1:3011")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3011", Duration::from_secs(5));

    let delete_response = http_post_json(
        "127.0.0.1:3011",
        "/sql",
        serde_json::json!({"query": "DELETE FROM users"}),
    );
    assert!(
        delete_response.starts_with("HTTP/1.1 400 Bad Request\r\n"),
        "{delete_response}"
    );

    let join_response = http_post_json(
        "127.0.0.1:3011",
        "/sql",
        serde_json::json!({"query": "SELECT * FROM users u JOIN teams t ON u.id=t.user_id"}),
    );
    assert!(
        join_response.starts_with("HTTP/1.1 400 Bad Request\r\n"),
        "{join_response}"
    );
}

fn wait_for_server(addr: &str, timeout: Duration) {
    let start = Instant::now();
    loop {
        if TcpStream::connect(addr).is_ok() {
            return;
        }

        if start.elapsed() >= timeout {
            panic!("server did not start in time at {addr}");
        }

        thread::sleep(Duration::from_millis(25));
    }
}

fn http_get(addr: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect to test server");
    let request = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .expect("write GET request");

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read GET response");
    response
}

fn http_post_json(addr: &str, path: &str, payload: serde_json::Value) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect to test server");
    let body = payload.to_string();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .expect("write POST request");

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read POST response");
    response
}

fn parse_http_body(response: &str) -> &str {
    response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .expect("response should contain header/body separator")
}

#[test]
fn sql_supports_is_null_and_projection_and_coercion() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let users_path = temp.path().join("users.json");
    let schema_path = temp.path().join("schema.dbml");

    let users = serde_json::json!([
        {"id": 1, "name": "Ada", "age": 30, "nickname": null},
        {"id": 2, "name": "Bob", "age": 20, "nickname": "B"}
    ]);

    std::fs::write(
        users_path,
        serde_json::to_string_pretty(&users).expect("serialize users"),
    )
    .expect("write users json");

    std::fs::write(
        schema_path,
        r#"
        Table users {
          id int [pk]
          name varchar [not null]
          age int
          nickname varchar
        }
        "#,
    )
    .expect("write schema");

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg("127.0.0.1:3012")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3012", Duration::from_secs(5));

    let response = http_get(
        "127.0.0.1:3012",
        "/sql?q=SELECT%20id,name%20FROM%20users%20WHERE%20age%20%3E%2025%20AND%20nickname%20IS%20NULL",
    );

    assert!(response.contains("200 OK"), "{response}");
    let body = parse_http_body(&response);
    let payload: serde_json::Value = serde_json::from_str(body).expect("json payload");
    assert_eq!(payload["row_count"], 1);
    assert_eq!(
        payload["rows"][0],
        serde_json::json!({"id": 1, "name": "Ada"})
    );
}

#[test]
fn sql_rejects_ambiguous_null_and_invalid_identifiers() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let users_path = temp.path().join("users.json");

    std::fs::write(users_path, r#"[{"id":1,"name":"Ada"}]"#).expect("write users");

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

    let null_cmp = http_get(
        "127.0.0.1:3013",
        "/sql?q=SELECT%20*%20FROM%20users%20WHERE%20name%20=%20NULL",
    );
    assert!(null_cmp.contains("400 Bad Request"), "{null_cmp}");

    let bad_identifier = http_get("127.0.0.1:3013", "/sql?q=SELECT%20*%20FROM%20users$");
    assert!(
        bad_identifier.contains("400 Bad Request"),
        "{bad_identifier}"
    );
}
