use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    path::Path,
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
fn school_examples_support_students_crud_end_to_end() {
    let temp = tempfile::tempdir().expect("create temp directory");
    copy_example_folder("school", temp.path());

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg("127.0.0.1:3020")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3020", Duration::from_secs(5));

    let get_existing = http_request("GET", "127.0.0.1:3020", "/students/1", None);
    assert!(
        get_existing.starts_with("HTTP/1.1 200 OK\r\n"),
        "{get_existing}"
    );
    let existing_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&get_existing)).expect("valid json body");
    assert_eq!(existing_payload["name"], "Alice Johnson");

    let post_response = http_request(
        "POST",
        "127.0.0.1:3020",
        "/students",
        Some(
            r#"{"name":"Dina Patel","email":"dina.patel@example.edu","year":4,"major":"Biology"}"#,
        ),
    );
    assert!(
        post_response.starts_with("HTTP/1.1 201 Created\r\n"),
        "{post_response}"
    );
    let created_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&post_response)).expect("valid json body");
    let created_id = created_payload["id"].as_i64().expect("created id");

    let put_response = http_request(
        "PUT",
        "127.0.0.1:3020",
        &format!("/students/{created_id}"),
        Some(&format!(
            r#"{{"id":{created_id},"name":"Dina Patel","email":"dina.patel@example.edu","year":4,"major":"Data Science"}}"#
        )),
    );
    assert!(
        put_response.starts_with("HTTP/1.1 200 OK\r\n"),
        "{put_response}"
    );
    let put_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&put_response)).expect("valid json body");
    assert_eq!(put_payload["major"], "Data Science");

    let patch_response = http_request(
        "PATCH",
        "127.0.0.1:3020",
        &format!("/students/{created_id}"),
        Some(r#"{"year":5}"#),
    );
    assert!(
        patch_response.starts_with("HTTP/1.1 200 OK\r\n"),
        "{patch_response}"
    );
    let patch_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&patch_response)).expect("valid json body");
    assert_eq!(patch_payload["year"], 5);

    let delete_response = http_request(
        "DELETE",
        "127.0.0.1:3020",
        &format!("/students/{created_id}"),
        None,
    );
    assert!(
        delete_response.starts_with("HTTP/1.1 204 No Content\r\n"),
        "{delete_response}"
    );

    let get_deleted = http_request(
        "GET",
        "127.0.0.1:3020",
        &format!("/students/{created_id}"),
        None,
    );
    assert!(
        get_deleted.starts_with("HTTP/1.1 404 Not Found\r\n"),
        "{get_deleted}"
    );

    let students_file =
        fs::read_to_string(temp.path().join("students.json")).expect("read students file");
    let students: serde_json::Value = serde_json::from_str(&students_file).expect("students json");
    assert_eq!(students.as_array().expect("array").len(), 4);
}

fn copy_example_folder(example_name: &str, destination: &Path) {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(example_name);

    for entry in fs::read_dir(source_root).expect("read example dir") {
        let entry = entry.expect("example entry");
        let source = entry.path();
        if source.is_file() {
            let target = destination.join(entry.file_name());
            fs::copy(source, target).expect("copy example file");
        }
    }
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
