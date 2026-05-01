use std::{
    fs,
    process::{Command, Stdio},
    time::Duration,
};

#[path = "test_support/mod.rs"]
mod support;

use support::{ChildGuard, http_request, parse_http_body, wait_for_server};

#[test]
fn starts_from_dirbase_conf_in_current_directory() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let data_dir = temp.path().join("configured");
    fs::create_dir(&data_dir).expect("create data directory");
    fs::write(
        data_dir.join("users.json"),
        r#"[
  {"id": 1, "name": "Config"}
]
"#,
    )
    .expect("write users file");

    let bind_addr = support::next_addr();
    fs::write(
        temp.path().join("dirbase.conf"),
        format!("--folder configured\n--bind {bind_addr}\n--readonly\n"),
    )
    .expect("write config file");

    let _child = ChildGuard::new(
        Command::new(env!("CARGO_BIN_EXE_dirbase"))
            .current_dir(temp.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start dirbase"),
    );

    wait_for_server(&bind_addr, Duration::from_secs(5));

    let users_response = http_request(&bind_addr, "GET", "/users", None);
    assert!(users_response.starts_with("HTTP/1.1 200 OK\r\n"), "{users_response}");
    let users: serde_json::Value =
        serde_json::from_str(parse_http_body(&users_response)).expect("users json");
    assert_eq!(users[0]["name"], "Config");

    let post_response = http_request(&bind_addr, "POST", "/users", Some(r#"{"name":"Blocked"}"#));
    assert!(post_response.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"), "{post_response}");
}

#[test]
fn command_line_args_override_dirbase_conf() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let config_dir = temp.path().join("from-config");
    let cli_dir = temp.path().join("from-cli");
    fs::create_dir(&config_dir).expect("create config directory");
    fs::create_dir(&cli_dir).expect("create cli directory");

    fs::write(
        config_dir.join("users.json"),
        r#"[
  {"id": 1, "name": "Config"}
]
"#,
    )
    .expect("write config users file");
    fs::write(
        cli_dir.join("users.json"),
        r#"[
  {"id": 1, "name": "CLI"}
]
"#,
    )
    .expect("write cli users file");

    let config_bind = support::next_addr();
    let cli_bind = support::next_addr();
    fs::write(
        temp.path().join("dirbase.conf"),
        format!("--folder from-config\n--bind {config_bind}\n"),
    )
    .expect("write config file");

    let _child = ChildGuard::new(
        Command::new(env!("CARGO_BIN_EXE_dirbase"))
            .current_dir(temp.path())
            .arg("--folder")
            .arg("from-cli")
            .arg("--bind")
            .arg(&cli_bind)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start dirbase"),
    );

    wait_for_server(&cli_bind, Duration::from_secs(5));

    let users_response = http_request(&cli_bind, "GET", "/users", None);
    assert!(users_response.starts_with("HTTP/1.1 200 OK\r\n"), "{users_response}");
    let users: serde_json::Value =
        serde_json::from_str(parse_http_body(&users_response)).expect("users json");
    assert_eq!(users[0]["name"], "CLI");
}

#[test]
fn starts_from_dirbase_conf_port_when_bind_is_not_set() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let data_dir = temp.path().join("configured");
    fs::create_dir(&data_dir).expect("create data directory");
    fs::write(
        data_dir.join("users.json"),
        r#"[
  {"id": 1, "name": "Port"}
]
"#,
    )
    .expect("write users file");

    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port();
    let bind_addr = format!("127.0.0.1:{port}");
    fs::write(temp.path().join("dirbase.conf"), format!("--folder configured\n--port {port}\n"))
        .expect("write config file");

    let _child = ChildGuard::new(
        Command::new(env!("CARGO_BIN_EXE_dirbase"))
            .current_dir(temp.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start dirbase"),
    );

    wait_for_server(&bind_addr, Duration::from_secs(5));

    let users_response = http_request(&bind_addr, "GET", "/users", None);
    assert!(users_response.starts_with("HTTP/1.1 200 OK\r\n"), "{users_response}");
    let users: serde_json::Value =
        serde_json::from_str(parse_http_body(&users_response)).expect("users json");
    assert_eq!(users[0]["name"], "Port");
}
