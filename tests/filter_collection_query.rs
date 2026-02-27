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

fn http_get(addr: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect to server");
    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");

    stream.write_all(request.as_bytes()).expect("write request");

    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read response");

    response
}
