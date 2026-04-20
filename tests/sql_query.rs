mod support;

use support::{
    http_get, http_post_json, parse_http_body, spawn_folder_server, spawn_folder_server_with_args,
};

#[test]
fn sql_get_works_in_readonly_mode() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let users_path = temp.path().join("users.json");

    let users = serde_json::json!([
        {"id": 1, "name": "Ada", "role": "admin", "age": 30},
        {"id": 2, "name": "Bob", "role": "member", "age": 20},
        {"id": 3, "name": "Cara", "role": "admin", "age": 25}
    ]);

    std::fs::write(users_path, serde_json::to_string_pretty(&users).expect("serialize users"))
        .expect("write users json");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), true);

    let response = http_get(
        &bind_addr,
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
    let teams_path = temp.path().join("teams.json");

    let users = serde_json::json!([
        {"id": 1, "name": "Ada", "role": "admin"}
    ]);
    let teams = serde_json::json!([
        {"id": 1, "user_id": 1, "name": "Core"}
    ]);

    std::fs::write(users_path, serde_json::to_string_pretty(&users).expect("serialize users"))
        .expect("write users json");
    std::fs::write(teams_path, serde_json::to_string_pretty(&teams).expect("serialize teams"))
        .expect("write teams json");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let delete_response =
        http_post_json(&bind_addr, "/sql", serde_json::json!({"query": "DELETE FROM users"}));
    assert!(delete_response.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{delete_response}");
    let delete_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&delete_response)).expect("delete error json");
    assert_eq!(delete_payload["code"], "unsupported_feature");

    let join_response = http_post_json(
        &bind_addr,
        "/sql",
        serde_json::json!({"query": "SELECT * FROM users u LEFT JOIN teams t ON u.id=t.user_id"}),
    );
    assert!(join_response.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{join_response}");
    let join_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&join_response)).expect("join error json");
    assert_eq!(join_payload["code"], "unsupported_feature");
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

    std::fs::write(users_path, serde_json::to_string_pretty(&users).expect("serialize users"))
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

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let response = http_get(
        &bind_addr,
        "/sql?q=SELECT%20id,name%20FROM%20users%20WHERE%20age%20%3E%2025%20AND%20nickname%20IS%20NULL",
    );

    assert!(response.contains("200 OK"), "{response}");
    let body = parse_http_body(&response);
    let payload: serde_json::Value = serde_json::from_str(body).expect("json payload");
    assert_eq!(payload["row_count"], 1);
    assert_eq!(payload["rows"][0], serde_json::json!({"id": 1, "name": "Ada"}));
}

#[test]
fn sql_rejects_ambiguous_null_and_invalid_identifiers() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let users_path = temp.path().join("users.json");

    std::fs::write(users_path, r#"[{"id":1,"name":"Ada"}]"#).expect("write users");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let null_cmp =
        http_get(&bind_addr, "/sql?q=SELECT%20*%20FROM%20users%20WHERE%20name%20=%20NULL");
    assert!(null_cmp.contains("400 Bad Request"), "{null_cmp}");

    let bad_identifier = http_get(&bind_addr, "/sql?q=SELECT%20*%20FROM%20users$");
    assert!(bad_identifier.contains("400 Bad Request"), "{bad_identifier}");
}

#[test]
fn sql_returns_structured_codes_for_invalid_and_unknown_table() {
    let temp = tempfile::tempdir().expect("create temp directory");
    std::fs::write(temp.path().join("users.json"), r#"[{"id":1,"name":"Ada"}]"#)
        .expect("write users");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let invalid_sql = http_get(&bind_addr, "/sql?q=SELECT%20FROM");
    assert!(invalid_sql.contains("400 Bad Request"), "{invalid_sql}");
    let invalid_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&invalid_sql)).expect("invalid sql payload");
    assert_eq!(invalid_payload["code"], "invalid_sql");

    let unknown_table = http_get(&bind_addr, "/sql?q=SELECT%20*%20FROM%20missing");
    assert!(unknown_table.contains("404 Not Found"), "{unknown_table}");
    let unknown_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&unknown_table)).expect("unknown table payload");
    assert_eq!(unknown_payload["code"], "unknown_table");
}

#[test]
fn sql_enforces_limit_guards() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let users_path = temp.path().join("users.json");

    let users = serde_json::json!(
        (1..=1005)
            .map(|id| serde_json::json!({"id": id, "name": format!("user-{id}")}))
            .collect::<Vec<_>>()
    );
    std::fs::write(users_path, serde_json::to_string_pretty(&users).expect("serialize users"))
        .expect("write users json");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let over_limit = http_get(&bind_addr, "/sql?q=SELECT%20*%20FROM%20users%20LIMIT%201001");
    assert!(over_limit.contains("400 Bad Request"), "{over_limit}");
    let over_limit_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&over_limit)).expect("over limit payload");
    assert_eq!(over_limit_payload["code"], "unsupported_feature");

    let no_limit = http_get(&bind_addr, "/sql?q=SELECT%20*%20FROM%20users");
    assert!(no_limit.contains("400 Bad Request"), "{no_limit}");
    let no_limit_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&no_limit)).expect("row count guard payload");
    assert_eq!(no_limit_payload["code"], "unsupported_feature");
}

#[test]
fn sql_custom_limit_guards_apply_to_get_and_post() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let users = serde_json::json!([
        {"id": 1, "name": "Ada"},
        {"id": 2, "name": "Grace"},
        {"id": 3, "name": "Linus"}
    ]);
    std::fs::write(
        temp.path().join("users.json"),
        serde_json::to_string_pretty(&users).expect("serialize users"),
    )
    .expect("write users");

    let (_child, scan_addr) =
        spawn_folder_server_with_args(temp.path(), &["--max-sql-scan-rows", "2"]);
    let scan_guard = http_get(&scan_addr, "/sql?q=SELECT%20*%20FROM%20users%20LIMIT%201");
    assert!(scan_guard.starts_with("HTTP/1.1 413 Payload Too Large\r\n"), "{scan_guard}");
    let scan_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&scan_guard)).expect("scan guard payload");
    assert_eq!(scan_payload["code"], "unsupported_feature");

    let (_child, selected_addr) = spawn_folder_server_with_args(
        temp.path(),
        &["--max-sql-scan-rows", "10", "--max-sql-selected-rows", "1"],
    );
    let selected_guard = http_post_json(
        &selected_addr,
        "/sql",
        serde_json::json!({"query": "SELECT * FROM users LIMIT 2"}),
    );
    assert!(selected_guard.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{selected_guard}");
    let selected_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&selected_guard)).expect("selected guard payload");
    assert_eq!(selected_payload["code"], "unsupported_feature");
    assert!(
        selected_payload["error"]
            .as_str()
            .expect("error")
            .contains("LIMIT exceeds max selected rows (1)"),
        "{selected_guard}"
    );
}
