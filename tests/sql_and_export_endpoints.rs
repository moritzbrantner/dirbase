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
fn sql_happy_paths_cover_select_projection_and_pagination() {
    let temp = tempfile::tempdir().expect("create temp directory");
    std::fs::write(
        temp.path().join("users.json"),
        serde_json::to_string_pretty(&serde_json::json!([
            {"id": 1, "name": "Ada", "role": "admin", "age": 30},
            {"id": 2, "name": "Bob", "role": "member", "age": 20},
            {"id": 3, "name": "Cara", "role": "admin", "age": 25},
            {"id": 4, "name": "Drew", "role": "admin", "age": 40}
        ]))
        .expect("serialize users"),
    )
    .expect("write users");

    let _child = start_server(temp.path(), 3020, false);

    let select_all = http_get(
        "127.0.0.1:3020",
        "/sql?q=SELECT%20*%20FROM%20users%20ORDER%20BY%20id%20ASC%20LIMIT%204",
    );
    assert!(select_all.starts_with("HTTP/1.1 200 OK\r\n"), "{select_all}");
    let all_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&select_all)).expect("all json");
    assert_eq!(all_payload["row_count"], 4);
    assert_eq!(all_payload["rows"][0]["name"], "Ada");
    assert_eq!(all_payload["rows"][3]["name"], "Drew");

    let projected = http_get(
        "127.0.0.1:3020",
        "/sql?q=SELECT%20id,name%20FROM%20users%20WHERE%20role%20=%20'admin'%20ORDER%20BY%20id%20DESC%20LIMIT%202%20OFFSET%202",
    );
    assert!(projected.starts_with("HTTP/1.1 200 OK\r\n"), "{projected}");
    let projected_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&projected)).expect("projected json");
    assert_eq!(projected_payload["row_count"], 1);
    assert_eq!(projected_payload["rows"], serde_json::json!([{"id": 1, "name": "Ada"}]));
}

#[test]
fn sql_error_paths_cover_non_select_unsupported_and_unknowns() {
    let temp = tempfile::tempdir().expect("create temp directory");
    std::fs::write(temp.path().join("users.json"), r#"[{"id":1,"name":"Ada"}]"#)
        .expect("write users");
    std::fs::write(
        temp.path().join("schema.dbml"),
        r#"
        Table users {
          id int [pk]
          name varchar
        }
        "#,
    )
    .expect("write schema");

    let _child = start_server(temp.path(), 3021, false);

    let non_select =
        http_post_json("127.0.0.1:3021", "/sql", serde_json::json!({"query": "DELETE FROM users"}));
    assert!(non_select.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{non_select}");
    let non_select_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&non_select)).expect("non-select payload");
    assert_eq!(non_select_payload["code"], "unsupported_feature");

    let unsupported =
        http_get("127.0.0.1:3021", "/sql?q=SELECT%20DISTINCT%20name%20FROM%20users%20LIMIT%201");
    assert!(unsupported.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{unsupported}");
    let unsupported_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&unsupported)).expect("unsupported payload");
    assert_eq!(unsupported_payload["code"], "unsupported_feature");

    let unknown_table = http_get("127.0.0.1:3021", "/sql?q=SELECT%20*%20FROM%20missing");
    assert!(unknown_table.starts_with("HTTP/1.1 404 Not Found\r\n"), "{unknown_table}");
    let unknown_table_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&unknown_table)).expect("unknown table payload");
    assert_eq!(unknown_table_payload["code"], "unknown_table");

    let unknown_column =
        http_get("127.0.0.1:3021", "/sql?q=SELECT%20email%20FROM%20users%20LIMIT%201");
    assert!(unknown_column.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{unknown_column}");
    let unknown_column_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&unknown_column)).expect("unknown column payload");
    assert!(
        unknown_column_payload["error"]
            .as_str()
            .expect("error string")
            .contains("Unknown column 'email'"),
        "{unknown_column}"
    );
}

#[test]
fn export_sql_is_stable_and_escapes_and_handles_nulls_without_schema() {
    let temp = tempfile::tempdir().expect("create temp directory");
    std::fs::write(
        temp.path().join("a_users.json"),
        serde_json::to_string_pretty(&serde_json::json!([
            {"id": 2, "name": "O'Hara", "meta": {"a": 1}, "note": null},
            {"id": 1, "name": "Ada", "meta": [1, 2], "note": "hello"}
        ]))
        .expect("serialize users"),
    )
    .expect("write users");
    std::fs::write(
        temp.path().join("z_projects.json"),
        serde_json::to_string_pretty(&serde_json::json!([
            {"id": 7, "title": "zeta"}
        ]))
        .expect("serialize projects"),
    )
    .expect("write projects");

    let _child = start_server(temp.path(), 3022, false);

    let response = http_get("127.0.0.1:3022", "/export.sql");
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    let body = parse_http_body(&response);

    let a_idx = body.find("-- Resource: a_users").expect("a_users block");
    let z_idx = body.find("-- Resource: z_projects").expect("z_projects block");
    assert!(a_idx < z_idx, "tables should be stable and sorted: {body}");

    assert!(
        body.contains("CREATE TABLE \"a_users\" (\n  \"id\" INTEGER NOT NULL,\n  \"meta\" JSONB NOT NULL,\n  \"name\" TEXT NOT NULL,\n  \"note\" TEXT\n);"),
        "{body}"
    );
    assert!(
        body.contains("INSERT INTO \"a_users\" (\"id\", \"meta\", \"name\", \"note\") VALUES (2, '{\"a\":1}'::jsonb, 'O''Hara', NULL);")
    );
    assert!(
        body.contains("INSERT INTO \"a_users\" (\"id\", \"meta\", \"name\", \"note\") VALUES (1, '[1,2]'::jsonb, 'Ada', 'hello');")
    );
}

#[test]
fn export_sql_respects_schema_when_present() {
    let temp = tempfile::tempdir().expect("create temp directory");
    std::fs::write(
        temp.path().join("users.json"),
        serde_json::to_string_pretty(&serde_json::json!([
            {"id": 1, "name": "Ada"}
        ]))
        .expect("serialize users"),
    )
    .expect("write users");
    std::fs::write(
        temp.path().join("schema.dbml"),
        r#"
        Table users {
          id int [pk]
          name varchar [not null]
          nickname varchar
        }
        "#,
    )
    .expect("write schema");

    let _child = start_server(temp.path(), 3023, false);

    let response = http_get("127.0.0.1:3023", "/export.sql");
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    let body = parse_http_body(&response);

    assert!(
        body.contains("CREATE TABLE \"users\" (\n  \"id\" INTEGER NOT NULL,\n  \"name\" TEXT NOT NULL,\n  \"nickname\" TEXT\n);"),
        "{body}"
    );
    assert!(
        body.contains(
            "INSERT INTO \"users\" (\"id\", \"name\", \"nickname\") VALUES (1, 'Ada', NULL);"
        ),
        "{body}"
    );
}

#[test]
fn readonly_mode_allows_sql_and_export_and_blocks_mutation_sql() {
    let temp = tempfile::tempdir().expect("create temp directory");
    std::fs::write(temp.path().join("users.json"), r#"[{"id":1,"name":"Ada"}]"#)
        .expect("write users");

    let _child = start_server(temp.path(), 3024, true);

    let sql_get = http_get("127.0.0.1:3024", "/sql?q=SELECT%20*%20FROM%20users%20LIMIT%201");
    assert!(sql_get.starts_with("HTTP/1.1 200 OK\r\n"), "{sql_get}");

    let export = http_get("127.0.0.1:3024", "/export.sql");
    assert!(export.starts_with("HTTP/1.1 200 OK\r\n"), "{export}");
    assert!(parse_http_body(&export).contains("INSERT INTO \"users\""), "{export}");

    let mutation_sql = http_post_json(
        "127.0.0.1:3024",
        "/sql",
        serde_json::json!({"query": "INSERT INTO users (id, name) VALUES (2, 'Bob')"}),
    );
    assert!(mutation_sql.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{mutation_sql}");
    let mutation_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&mutation_sql)).expect("mutation payload");
    assert_eq!(mutation_payload["code"], "unsupported_feature");
}

fn start_server(folder: &std::path::Path, port: u16, readonly: bool) -> ChildGuard {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_folder-server"));
    cmd.arg("--folder").arg(folder).arg("--bind").arg(format!("127.0.0.1:{port}"));

    if readonly {
        cmd.arg("--readonly");
    }

    let child =
        cmd.stdout(Stdio::null()).stderr(Stdio::null()).spawn().expect("start folder-server");

    wait_for_server(&format!("127.0.0.1:{port}"), Duration::from_secs(5));

    ChildGuard { child }
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
    stream.write_all(request.as_bytes()).expect("write GET request");

    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read GET response");
    response
}

fn http_post_json(addr: &str, path: &str, payload: serde_json::Value) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect to test server");
    let body = payload.to_string();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).expect("write POST request");

    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read POST response");
    response
}

fn parse_http_body(response: &str) -> &str {
    response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .expect("response should contain header/body separator")
}
