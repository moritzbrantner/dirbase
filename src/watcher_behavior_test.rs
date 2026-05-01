use std::fs;

#[path = "test_support/mod.rs"]
mod support;

use support::{
    http_request, parse_http_body, spawn_file_server, spawn_folder_server, wait_for_http,
    wait_for_json,
};

#[test]
fn watcher_adds_and_deletes_folder_resources_after_startup() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let users_path = temp.path().join("users.json");
    let teams_path = temp.path().join("teams.json");
    fs::write(&users_path, "[{\"id\":1,\"name\":\"Ada\"}]\n").expect("write users");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let initial_root = http_request(&bind_addr, "GET", "/", None);
    assert!(initial_root.contains("\"users\""), "{initial_root}");
    assert!(!initial_root.contains("\"teams\""), "{initial_root}");

    fs::write(&teams_path, "[{\"id\":10,\"name\":\"Core\"}]\n").expect("write teams");
    let added = wait_for_http(&bind_addr, "GET", "/", None, |response| {
        response.starts_with("HTTP/1.1 200 OK\r\n") && response.contains("\"teams\"")
    });
    assert!(added.contains("\"users\""), "{added}");

    fs::remove_file(&users_path).expect("delete users");
    let deleted = wait_for_http(&bind_addr, "GET", "/users", None, |response| {
        response.starts_with("HTTP/1.1 404 Not Found\r\n")
    });
    assert!(deleted.contains("Resource 'users' not found"), "{deleted}");
}

#[test]
fn watcher_renames_folder_resources() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let users_path = temp.path().join("users.json");
    let members_path = temp.path().join("members.json");
    fs::write(&users_path, "[{\"id\":1,\"name\":\"Ada\"}]\n").expect("write users");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    fs::rename(&users_path, &members_path).expect("rename file");

    let members = wait_for_http(&bind_addr, "GET", "/members", None, |response| {
        response.starts_with("HTTP/1.1 200 OK\r\n") && response.contains("\"name\":\"Ada\"")
    });
    assert!(members.contains("\"id\":1"), "{members}");

    let users_missing = wait_for_http(&bind_addr, "GET", "/users", None, |response| {
        response.starts_with("HTTP/1.1 404 Not Found\r\n")
    });
    assert!(users_missing.contains("Resource 'users' not found"), "{users_missing}");
}

#[test]
fn watcher_invalid_json_toggles_readyz_and_recovers() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let teams_path = temp.path().join("teams.json");
    fs::write(&teams_path, "[{\"id\":10,\"name\":\"Core\"}]\n").expect("write teams");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    fs::write(&teams_path, "[{\"id\":10").expect("write invalid json");
    let not_ready = wait_for_http(&bind_addr, "GET", "/readyz", None, |response| {
        response.starts_with("HTTP/1.1 503 Service Unavailable\r\n")
    });
    assert!(not_ready.contains("\"ready\":false"), "{not_ready}");

    fs::write(&teams_path, "[{\"id\":10,\"name\":\"Core\",\"city\":\"Berlin\"}]\n")
        .expect("repair teams");
    let recovered = wait_for_http(&bind_addr, "GET", "/readyz", None, |response| {
        response.starts_with("HTTP/1.1 200 OK\r\n")
    });
    assert!(recovered.contains("\"ready\":true"), "{recovered}");

    let teams = wait_for_http(&bind_addr, "GET", "/teams", None, |response| {
        response.starts_with("HTTP/1.1 200 OK\r\n") && response.contains("\"city\":\"Berlin\"")
    });
    assert!(teams.contains("\"name\":\"Core\""), "{teams}");
}

#[test]
fn watcher_file_mode_updates_top_level_resources() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let db_path = temp.path().join("db.json");
    fs::write(&db_path, "{\"users\":[{\"id\":1,\"name\":\"Ada\"}]}\n").expect("write db");

    let (_child, bind_addr) = spawn_file_server(&db_path);

    fs::write(
        &db_path,
        "{\"users\":[{\"id\":1,\"name\":\"Ada\"}],\"teams\":[{\"id\":10,\"name\":\"Core\"}]}\n",
    )
    .expect("add teams");
    let added = wait_for_http(&bind_addr, "GET", "/", None, |response| {
        response.starts_with("HTTP/1.1 200 OK\r\n") && response.contains("\"teams\"")
    });
    assert!(added.contains("\"users\""), "{added}");

    fs::write(&db_path, "{\"teams\":[{\"id\":10,\"name\":\"Core\"}]}\n").expect("remove users");
    let users_missing = wait_for_http(&bind_addr, "GET", "/users", None, |response| {
        response.starts_with("HTTP/1.1 404 Not Found\r\n")
    });
    assert!(users_missing.contains("Resource 'users' not found"), "{users_missing}");
}

#[test]
fn watcher_schema_rewrite_updates_effective_schema_and_reserved_files_stay_hidden() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let schema_path = temp.path().join("schema.json");
    let users_path = temp.path().join("users.json");
    let posts_path = temp.path().join("posts.json");
    let metrics_path = temp.path().join("metrics.json");

    fs::write(&schema_path, "{\"tables\":{}}\n").expect("write schema");
    fs::write(&users_path, "[{\"user_id\":1,\"name\":\"Ada\"}]\n").expect("write users");
    fs::write(&posts_path, "[{\"id\":1,\"author_ref\":1,\"title\":\"Hello\"}]\n")
        .expect("write posts");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);

    let initial_schema = http_request(&bind_addr, "GET", "/schema", None);
    let initial_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&initial_schema)).expect("schema json");
    assert!(
        initial_payload["tables"]["posts"]["foreign_keys"]
            .as_object()
            .is_none_or(|fks| fks.is_empty())
    );

    fs::write(
        &schema_path,
        r#"{
  "tables": {
    "users": {
      "primary_key": "user_id"
    },
    "posts": {
      "foreign_keys": {
        "author_ref": {
          "target_table": "users",
          "target_column": "user_id"
        }
      }
    }
  }
}
"#,
    )
    .expect("rewrite schema");
    fs::write(&metrics_path, "{\"ignored\":true}\n").expect("write reserved file");

    let schema_payload = wait_for_json(&bind_addr, "GET", "/schema", None, |payload| {
        payload["tables"]["posts"]["foreign_keys"]["author_ref"]["target_table"] == "users"
            && payload["tables"]["posts"]["foreign_keys"]["author_ref"]["target_column"]
                == "user_id"
    });
    assert_eq!(
        schema_payload["tables"]["posts"]["foreign_keys"]["author_ref"]["target_table"],
        "users"
    );

    let root = http_request(&bind_addr, "GET", "/", None);
    assert!(!root.contains("\"metrics\""), "{root}");

    let declared_pk_lookup = http_request(&bind_addr, "GET", "/users/1", None);
    assert!(declared_pk_lookup.starts_with("HTTP/1.1 200 OK\r\n"), "{declared_pk_lookup}");
    assert!(declared_pk_lookup.contains("\"user_id\":1"), "{declared_pk_lookup}");

    let embedded = http_request(&bind_addr, "GET", "/posts?embed=author_ref", None);
    assert!(embedded.starts_with("HTTP/1.1 200 OK\r\n"), "{embedded}");
    let embedded_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&embedded)).expect("embedded json");
    assert_eq!(embedded_payload[0]["author_ref"]["name"], "Ada");

    let graphql = http_request(
        &bind_addr,
        "POST",
        "/graphql",
        Some(r#"{"query":"{ usersById(id: \"1\") { user_id name } posts { author { name } } }"}"#),
    );
    assert!(graphql.starts_with("HTTP/1.1 200 OK\r\n"), "{graphql}");
    let graphql_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&graphql)).expect("graphql json");
    assert_eq!(graphql_payload["data"]["usersById"]["name"], "Ada");
    assert_eq!(graphql_payload["data"]["posts"][0]["author"]["name"], "Ada");

    let sql = http_request(
        &bind_addr,
        "GET",
        "/sql?q=SELECT%20u.name,p.title%20FROM%20posts%20p%20JOIN%20users%20u%20ON%20p.author_ref%20=%20u.user_id%20LIMIT%201",
        None,
    );
    assert!(sql.starts_with("HTTP/1.1 200 OK\r\n"), "{sql}");
    let sql_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&sql)).expect("sql json");
    assert_eq!(sql_payload["rows"][0]["u.name"], "Ada");
    assert_eq!(sql_payload["rows"][0]["p.title"], "Hello");
}
