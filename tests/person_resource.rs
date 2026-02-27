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
fn serves_person_collection_and_item_from_person_json() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::copy("tests/fixtures/person.json", temp.path().join("person.json"))
        .expect("copy person fixture");

    let bind = reserve_bind_addr();

    let child = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg(&bind)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start folder-server");
    let _child = ChildGuard { child };

    wait_for_server(&bind, Duration::from_secs(5));

    let root_response = http_get(&bind, "/");
    assert!(
        root_response.starts_with("HTTP/1.1 200 OK\r\n"),
        "expected 200 OK for root response, got: {root_response}"
    );
    assert!(
        root_response.contains("\"resources\":[\"person\"]"),
        "expected root to list person resource, got: {root_response}"
    );

    let collection_response = http_get(&bind, "/person");
    assert!(
        collection_response.starts_with("HTTP/1.1 200 OK\r\n"),
        "expected 200 OK for /person, got: {collection_response}"
    );
    assert!(
        collection_response.contains("\"lastName\":\"Lovelace\""),
        "expected /person to return seeded data, got: {collection_response}"
    );
    assert!(
        collection_response.contains("\"jobTitle\":\"Computer Scientist\""),
        "expected /person to include job title data, got: {collection_response}"
    );

    let item_response = http_get(&bind, "/person/2");
    assert!(
        item_response.starts_with("HTTP/1.1 200 OK\r\n"),
        "expected 200 OK for /person/2, got: {item_response}"
    );
    assert!(
        item_response.contains("\"id\":2"),
        "expected /person/2 response to include id 2, got: {item_response}"
    );
    assert!(
        item_response.contains("\"lastName\":\"Hopper\""),
        "expected /person/2 response to include Hopper, got: {item_response}"
    );
}

fn reserve_bind_addr() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("read local addr");
    drop(listener);
    addr.to_string()
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
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    );

    stream
        .write_all(request.as_bytes())
        .expect("write request");

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read response");

    response
}
