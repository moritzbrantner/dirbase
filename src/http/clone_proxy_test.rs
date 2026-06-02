use std::{
    collections::{HashMap, VecDeque},
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

#[path = "../test_support/mod.rs"]
mod support;

use serde_json::Value;
use support::{
    assert_status, http_get, http_request, parse_http_body, spawn_folder_server_with_args,
};

#[test]
fn item_cache_miss_fetches_remote_persists_and_next_hit_is_local() {
    let remote = TestRemote::start();
    remote.enqueue_json("GET", "/employee/3", 200, r#"{"name":"Ada"}"#);
    let temp = tempfile::tempdir().expect("tempdir");
    let base = remote.base_url();
    let args = ["--clone-from", base.as_str()];
    let (_child, addr) = spawn_folder_server_with_args(temp.path(), &args);

    let first = http_get(&addr, "/employee/3");
    assert_status(&first, "200 OK");
    assert_eq!(remote.request_count(), 1);

    let cached: Value =
        serde_json::from_str(&fs::read_to_string(temp.path().join("employee.json")).expect("read"))
            .expect("json");
    assert_eq!(cached, serde_json::json!([{"id": 3, "name": "Ada"}]));

    let second = http_get(&addr, "/employee/3");
    assert_status(&second, "200 OK");
    assert_eq!(remote.request_count(), 1);
    let payload: Value = serde_json::from_str(parse_http_body(&second)).expect("body json");
    assert_eq!(payload, serde_json::json!({"id": 3, "name": "Ada"}));
}

#[test]
fn collection_cache_miss_persists_remote_array() {
    let remote = TestRemote::start();
    remote.enqueue_json("GET", "/employee", 200, r#"[{"id":1,"name":"Ada"}]"#);
    let temp = tempfile::tempdir().expect("tempdir");
    let base = remote.base_url();
    let args = ["--clone-from", base.as_str()];
    let (_child, addr) = spawn_folder_server_with_args(temp.path(), &args);

    let response = http_get(&addr, "/employee");
    assert_status(&response, "200 OK");

    let cached: Value =
        serde_json::from_str(&fs::read_to_string(temp.path().join("employee.json")).expect("read"))
            .expect("json");
    assert_eq!(cached, serde_json::json!([{"id": 1, "name": "Ada"}]));
}

#[test]
fn query_requests_proxy_without_overwriting_local_cache() {
    let remote = TestRemote::start();
    remote.enqueue_json("GET", "/employee?active=true", 200, r#"[{"id":2,"name":"Grace"}]"#);
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("employee.json"), r#"[{"id":1,"name":"Ada"}]"#)
        .expect("write local");
    let base = remote.base_url();
    let args = ["--clone-from", base.as_str()];
    let (_child, addr) = spawn_folder_server_with_args(temp.path(), &args);

    let response = http_get(&addr, "/employee?active=true");
    assert_status(&response, "200 OK");
    let payload: Value = serde_json::from_str(parse_http_body(&response)).expect("body json");
    assert_eq!(payload, serde_json::json!([{"id": 2, "name": "Grace"}]));

    let cached: Value =
        serde_json::from_str(&fs::read_to_string(temp.path().join("employee.json")).expect("read"))
            .expect("json");
    assert_eq!(cached, serde_json::json!([{"id": 1, "name": "Ada"}]));
}

#[test]
fn refresh_query_bypasses_local_cache_and_updates_file() {
    let remote = TestRemote::start();
    remote.enqueue_json("GET", "/employee/1", 200, r#"{"name":"Grace"}"#);
    let temp = tempfile::tempdir().expect("tempdir");
    fs::write(temp.path().join("employee.json"), r#"[{"id":1,"name":"Ada"}]"#)
        .expect("write local");
    let base = remote.base_url();
    let args = ["--clone-from", base.as_str()];
    let (_child, addr) = spawn_folder_server_with_args(temp.path(), &args);

    let response = http_get(&addr, "/employee/1?_refresh=true");
    assert_status(&response, "200 OK");
    assert_eq!(remote.last_path(), Some("/employee/1".to_string()));

    let cached: Value =
        serde_json::from_str(&fs::read_to_string(temp.path().join("employee.json")).expect("read"))
            .expect("json");
    assert_eq!(cached, serde_json::json!([{"id": 1, "name": "Grace"}]));
}

#[test]
fn non_get_resource_requests_proxy_without_local_writes() {
    let remote = TestRemote::start();
    remote.enqueue_json("POST", "/employee", 201, r#"{"id":7,"name":"Lin"}"#);
    let temp = tempfile::tempdir().expect("tempdir");
    let base = remote.base_url();
    let args = ["--clone-from", base.as_str()];
    let (_child, addr) = spawn_folder_server_with_args(temp.path(), &args);

    let response = http_request(&addr, "POST", "/employee", Some(r#"{"name":"Lin"}"#));
    assert_status(&response, "201 Created");
    assert!(!temp.path().join("employee.json").exists());
    let request = remote.last_request().expect("remote request");
    assert_eq!(request.method, "POST");
    assert_eq!(request.body, r#"{"name":"Lin"}"#);
}

#[test]
fn configured_clone_headers_are_forwarded_but_inbound_authorization_is_not() {
    let remote = TestRemote::start();
    remote.enqueue_json("GET", "/employee/1", 200, r#"{"id":1,"name":"Ada"}"#);
    let temp = tempfile::tempdir().expect("tempdir");
    let base = remote.base_url();
    let args =
        ["--clone-from", base.as_str(), "--clone-header", "Authorization=Bearer remote-token"];
    let (_child, addr) = spawn_folder_server_with_args(temp.path(), &args);

    let response = support::request_with_headers(
        &addr,
        "GET",
        "/employee/1",
        "Authorization: Bearer local-token\r\nX-Request-Id: req-1\r\n",
        None,
    );
    assert_status(&response, "200 OK");
    let request = remote.last_request().expect("remote request");
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Bearer remote-token")
    );
    assert_eq!(request.headers.get("x-request-id").map(String::as_str), Some("req-1"));
}

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

struct TestRemote {
    addr: String,
    routes: Arc<Mutex<HashMap<String, VecDeque<ResponseSpec>>>>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    running: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

#[derive(Clone)]
struct ResponseSpec {
    status: u16,
    body: String,
}

impl TestRemote {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind remote");
        listener.set_nonblocking(true).expect("nonblocking");
        let addr = listener.local_addr().expect("addr").to_string();
        let routes = Arc::new(Mutex::new(HashMap::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let running = Arc::new(AtomicBool::new(true));
        let thread_routes = routes.clone();
        let thread_requests = requests.clone();
        let thread_running = running.clone();
        let thread = thread::spawn(move || {
            while thread_running.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        handle_remote_connection(&mut stream, &thread_routes, &thread_requests);
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Self { addr, routes, requests, running, thread: Some(thread) }
    }

    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn enqueue_json(&self, method: &str, path: &str, status: u16, body: &str) {
        let key = route_key(method, path);
        self.routes
            .lock()
            .expect("routes")
            .entry(key)
            .or_default()
            .push_back(ResponseSpec { status, body: body.to_string() });
    }

    fn request_count(&self) -> usize {
        self.requests.lock().expect("requests").len()
    }

    fn last_path(&self) -> Option<String> {
        self.last_request().map(|request| request.path)
    }

    fn last_request(&self) -> Option<RecordedRequest> {
        self.requests.lock().expect("requests").last().cloned()
    }
}

impl Drop for TestRemote {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        let _ = TcpStream::connect(&self.addr);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn handle_remote_connection(
    stream: &mut TcpStream,
    routes: &Arc<Mutex<HashMap<String, VecDeque<ResponseSpec>>>>,
    requests: &Arc<Mutex<Vec<RecordedRequest>>>,
) {
    let mut buffer = [0; 8192];
    let bytes_read = stream.read(&mut buffer).expect("read request");
    let raw = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let mut lines = head.lines();
    let first = lines.next().expect("request line");
    let mut request_parts = first.split_whitespace();
    let method = request_parts.next().unwrap_or_default().to_string();
    let path = request_parts.next().unwrap_or_default().to_string();
    let headers = lines
        .filter_map(|line| {
            line.split_once(':')
                .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect::<HashMap<_, _>>();
    let body = body.to_string();
    requests.lock().expect("requests").push(RecordedRequest {
        method: method.clone(),
        path: path.clone(),
        headers,
        body,
    });

    let spec = routes
        .lock()
        .expect("routes")
        .get_mut(&route_key(&method, &path))
        .and_then(VecDeque::pop_front)
        .unwrap_or_else(|| ResponseSpec {
            status: 404,
            body: serde_json::json!({"error": "not found"}).to_string(),
        });
    let status_text = match spec.status {
        200 => "OK",
        201 => "Created",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        spec.status,
        status_text,
        spec.body.len(),
        spec.body
    );
    stream.write_all(response.as_bytes()).expect("write response");
}

fn route_key(method: &str, path: &str) -> String {
    format!("{method} {path}")
}
