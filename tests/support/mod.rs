#![allow(dead_code)]

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub struct ChildGuard {
    child: Child,
}

impl ChildGuard {
    pub fn new(child: Child) -> Self {
        Self { child }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn next_addr() -> String {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .to_string()
}

pub fn spawn_folder_server(folder: &Path, readonly: bool) -> (ChildGuard, String) {
    if readonly {
        spawn_folder_server_with_args(folder, &["--readonly"])
    } else {
        spawn_folder_server_with_args(folder, &[])
    }
}

pub fn spawn_file_server(file: &Path) -> (ChildGuard, String) {
    spawn_file_server_with_args(file, &[])
}

pub fn spawn_folder_server_with_args(folder: &Path, extra_args: &[&str]) -> (ChildGuard, String) {
    spawn_with_retry(|bind_addr| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dirbase"));
        command
            .arg("--folder")
            .arg(folder)
            .arg("--bind")
            .arg(bind_addr)
            .args(extra_args)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    })
}

pub fn spawn_file_server_with_args(file: &Path, extra_args: &[&str]) -> (ChildGuard, String) {
    spawn_with_retry(|bind_addr| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_dirbase"));
        command
            .arg(file)
            .arg("--bind")
            .arg(bind_addr)
            .args(extra_args)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    })
}

pub fn wait_for_server(addr: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;

    loop {
        match TcpStream::connect(addr) {
            Ok(_) => return,
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            Err(err) => panic!("server at {addr} did not start in time: {err}"),
        }
    }
}

pub fn parse_http_body(response: &str) -> &str {
    response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .or_else(|| response.split_once("\n\n").map(|(_, body)| body))
        .expect("response should have body")
}

pub fn http_get(addr: &str, path: &str) -> String {
    http_request(addr, "GET", path, None)
}

pub fn http_request(addr: &str, method: &str, path: &str, body: Option<&str>) -> String {
    http_request_with_headers(addr, method, path, None, body)
}

pub fn http_request_with_headers(
    addr: &str,
    method: &str,
    path: &str,
    extra_headers: Option<&str>,
    body: Option<&str>,
) -> String {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut stream = loop {
        match TcpStream::connect(addr) {
            Ok(stream) => break stream,
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Err(err) => panic!("connect to server: {err}"),
        }
    };
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

pub fn http_post_json(addr: &str, path: &str, payload: serde_json::Value) -> String {
    http_request(addr, "POST", path, Some(&payload.to_string()))
}

pub fn wait_for_http<F>(
    addr: &str,
    method: &str,
    path: &str,
    body: Option<&str>,
    predicate: F,
) -> String
where
    F: Fn(&str) -> bool,
{
    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        let response = http_request(addr, method, path, body);
        if predicate(&response) {
            return response;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for HTTP response on {path}: {response}");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub fn wait_for_json<F>(
    addr: &str,
    method: &str,
    path: &str,
    body: Option<&str>,
    predicate: F,
) -> serde_json::Value
where
    F: Fn(&serde_json::Value) -> bool,
{
    let deadline = Instant::now() + Duration::from_secs(5);

    loop {
        let response = http_request(addr, method, path, body);
        let payload: serde_json::Value =
            serde_json::from_str(parse_http_body(&response)).expect("valid json body");
        if predicate(&payload) {
            return payload;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for JSON response on {path}: {payload}");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub fn open_sse_stream(addr: &str, path: &str) -> TcpStream {
    wait_for_server(addr, Duration::from_secs(5));
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stream = loop {
        match TcpStream::connect(addr) {
            Ok(stream) => break stream,
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            Err(err) => panic!("connect to events: {err}"),
        }
    };
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\nConnection: keep-alive\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).expect("write events request");
    stream.set_read_timeout(Some(Duration::from_millis(200))).expect("set timeout");
    stream
}

fn spawn_with_retry(build: impl Fn(&str) -> Command) -> (ChildGuard, String) {
    for _attempt in 0..5 {
        let bind_addr = next_addr();
        let mut child = build(&bind_addr).spawn().expect("start dirbase");
        if wait_for_server_start(&bind_addr, Duration::from_secs(5), &mut child) {
            return (ChildGuard::new(child), bind_addr);
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    panic!("failed to start dirbase after multiple bind attempts");
}

fn wait_for_server_start(addr: &str, timeout: Duration, child: &mut Child) -> bool {
    let deadline = Instant::now() + timeout;

    loop {
        if child.try_wait().expect("poll child exit").is_some() {
            return false;
        }
        match TcpStream::connect(addr) {
            Ok(_) => return true,
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            Err(_) => return false,
        }
    }
}
