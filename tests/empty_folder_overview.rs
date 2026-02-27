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
fn empty_folder_returns_empty_overview_on_root() {
    let temp = tempfile::tempdir().expect("create temp directory");

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg("127.0.0.1:3000")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server("127.0.0.1:3000", Duration::from_secs(5));

    let response = http_get("127.0.0.1:3000", "/");

    assert!(
        response.starts_with("HTTP/1.1 200 OK\r\n"),
        "expected 200 OK response, got: {response}"
    );
    assert!(
        response.contains("{\"resources\":[]}"),
        "expected empty overview body, got: {response}"
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
