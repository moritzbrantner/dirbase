#[path = "../test_support/mod.rs"]
mod support;

use support::{ChildGuard, http_get, http_post_json, parse_http_body};

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

    let (_child, bind_addr) = start_server(temp.path(), false);

    let select_all = http_get(
        &bind_addr,
        "/sql?q=SELECT%20*%20FROM%20users%20ORDER%20BY%20id%20ASC%20LIMIT%204",
    );
    assert!(select_all.starts_with("HTTP/1.1 200 OK\r\n"), "{select_all}");
    let all_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&select_all)).expect("all json");
    assert_eq!(all_payload["row_count"], 4);
    assert_eq!(all_payload["rows"][0]["name"], "Ada");
    assert_eq!(all_payload["rows"][3]["name"], "Drew");

    let projected = http_get(
        &bind_addr,
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

    let (_child, bind_addr) = start_server(temp.path(), false);

    let unsupported =
        http_get(&bind_addr, "/sql?q=SELECT%20DISTINCT%20name%20FROM%20users%20LIMIT%201");
    assert!(unsupported.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{unsupported}");
    let unsupported_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&unsupported)).expect("unsupported payload");
    assert_eq!(unsupported_payload["code"], "unsupported_feature");

    let unknown_column = http_get(&bind_addr, "/sql?q=SELECT%20email%20FROM%20users%20LIMIT%201");
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

    let (_child, bind_addr) = start_server(temp.path(), false);

    let response = http_get(&bind_addr, "/export.sql");
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

    let (_child, bind_addr) = start_server(temp.path(), false);

    let response = http_get(&bind_addr, "/export.sql");
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
fn export_sql_maps_extended_schema_types() {
    let temp = tempfile::tempdir().expect("create temp directory");
    std::fs::write(
        temp.path().join("appointments.json"),
        r#"[
  {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "starts_on": "2026-04-29",
    "starts_at": "2026-04-29T12:30:00Z",
    "counter": "9223372036854775808",
    "amount": "123.45"
  }
]"#,
    )
    .expect("write appointments");
    std::fs::write(
        temp.path().join("schema.json"),
        r#"{
  "tables": {
    "appointments": {
      "columns": {
        "id": {"column_type": "uuid", "nullable": false},
        "starts_on": {"column_type": "date", "nullable": false},
        "starts_at": {"column_type": "datetime", "nullable": false},
        "counter": {"column_type": "big_integer", "nullable": false},
        "amount": {"column_type": "decimal", "nullable": false}
      }
    }
  }
}"#,
    )
    .expect("write schema");

    let (_child, bind_addr) = start_server(temp.path(), false);
    let pg = http_get(&bind_addr, "/export.sql");
    let pg_body = parse_http_body(&pg);
    assert!(pg_body.contains("\"id\" UUID NOT NULL"), "{pg_body}");
    assert!(pg_body.contains("\"starts_on\" DATE NOT NULL"), "{pg_body}");
    assert!(pg_body.contains("\"starts_at\" TIMESTAMPTZ NOT NULL"), "{pg_body}");
    assert!(pg_body.contains("\"counter\" BIGINT NOT NULL"), "{pg_body}");
    assert!(pg_body.contains("\"amount\" NUMERIC NOT NULL"), "{pg_body}");

    let sqlite = http_get(&bind_addr, "/export.sql?dialect=sqlite");
    let sqlite_body = parse_http_body(&sqlite);
    assert!(sqlite_body.contains("\"id\" TEXT NOT NULL"), "{sqlite_body}");
    assert!(sqlite_body.contains("\"starts_on\" TEXT NOT NULL"), "{sqlite_body}");
    assert!(sqlite_body.contains("\"starts_at\" TEXT NOT NULL"), "{sqlite_body}");
    assert!(sqlite_body.contains("\"counter\" INTEGER NOT NULL"), "{sqlite_body}");
    assert!(sqlite_body.contains("\"amount\" TEXT NOT NULL"), "{sqlite_body}");
}

#[test]
fn readonly_mode_allows_sql_and_export_and_rejects_post_sql() {
    let temp = tempfile::tempdir().expect("create temp directory");
    std::fs::write(temp.path().join("users.json"), r#"[{"id":1,"name":"Ada"}]"#)
        .expect("write users");

    let (_child, bind_addr) = start_server(temp.path(), true);

    let sql_get = http_get(&bind_addr, "/sql?q=SELECT%20*%20FROM%20users%20LIMIT%201");
    assert!(sql_get.starts_with("HTTP/1.1 200 OK\r\n"), "{sql_get}");

    let export = http_get(&bind_addr, "/export.sql");
    assert!(export.starts_with("HTTP/1.1 200 OK\r\n"), "{export}");
    assert!(parse_http_body(&export).contains("INSERT INTO \"users\""), "{export}");

    let post_sql = http_post_json(
        &bind_addr,
        "/sql",
        serde_json::json!({"query": "SELECT * FROM users LIMIT 1"}),
    );
    assert!(post_sql.starts_with("HTTP/1.1 405 Method Not Allowed\r\n"), "{post_sql}");
}

#[test]
fn sql_inner_join_supports_schema_backed_relations() {
    let temp = tempfile::tempdir().expect("create temp directory");
    std::fs::write(
        temp.path().join("users.json"),
        r#"[
  {"id": 1, "name": "Ada"},
  {"id": 2, "name": "Grace"}
]
"#,
    )
    .expect("write users");
    std::fs::write(
        temp.path().join("teams.json"),
        r#"[
  {"id": 10, "user_id": 1, "name": "Core"},
  {"id": 11, "user_id": 2, "name": "Infra"}
]
"#,
    )
    .expect("write teams");
    std::fs::write(
        temp.path().join("schema.dbml"),
        r#"
        Table users {
          id int [pk]
          name varchar
        }

        Table teams {
          id int [pk]
          user_id int [ref: > users.id]
          name varchar
        }
        "#,
    )
    .expect("write schema");

    let (_child, bind_addr) = start_server(temp.path(), false);

    let response = http_get(
        &bind_addr,
        "/sql?q=SELECT%20u.name,t.name%20FROM%20users%20u%20JOIN%20teams%20t%20ON%20u.id%20=%20t.user_id%20ORDER%20BY%20u.id%20ASC%20LIMIT%202",
    );
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    let payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&response)).expect("join payload");
    assert_eq!(
        payload["rows"],
        serde_json::json!([
            {"u.name": "Ada", "t.name": "Core"},
            {"u.name": "Grace", "t.name": "Infra"}
        ])
    );
}

#[test]
fn sql_alias_projection_filter_order_and_invalid_join_paths_are_reported() {
    let temp = tempfile::tempdir().expect("create temp directory");
    std::fs::write(
        temp.path().join("users.json"),
        r#"[
  {"id": 1, "name": "Ada"},
  {"id": 2, "name": "Grace"}
]
"#,
    )
    .expect("write users");
    std::fs::write(
        temp.path().join("posts.json"),
        r#"[
  {"id": 10, "author_id": 1, "title": "First"},
  {"id": 11, "author_id": 2, "title": "Second"}
]
"#,
    )
    .expect("write posts");
    std::fs::write(
        temp.path().join("schema.dbml"),
        r#"
        Table users {
          id int [pk]
          name varchar
        }

        Table posts {
          id int [pk]
          author_id int [ref: > users.id]
          title varchar
        }
        "#,
    )
    .expect("write schema");

    let (_child, bind_addr) = start_server(temp.path(), false);

    let aliased = http_get(
        &bind_addr,
        "/sql?q=SELECT%20p.title,u.name%20FROM%20posts%20p%20JOIN%20users%20u%20ON%20p.author_id%20=%20u.id%20WHERE%20u.name%20=%20'Grace'%20ORDER%20BY%20p.id%20DESC%20LIMIT%201",
    );
    assert!(aliased.starts_with("HTTP/1.1 200 OK\r\n"), "{aliased}");
    let aliased_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&aliased)).expect("aliased payload");
    assert_eq!(
        aliased_payload["rows"],
        serde_json::json!([{"p.title": "Second", "u.name": "Grace"}])
    );

    let unknown_alias = http_get(
        &bind_addr,
        "/sql?q=SELECT%20x.name%20FROM%20posts%20p%20JOIN%20users%20u%20ON%20p.author_id%20=%20u.id%20LIMIT%201",
    );
    assert!(unknown_alias.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{unknown_alias}");
    assert!(unknown_alias.contains("Unknown table alias 'x'"), "{unknown_alias}");

    let invalid_join = http_get(
        &bind_addr,
        "/sql?q=SELECT%20p.title,u.name%20FROM%20posts%20p%20JOIN%20users%20u%20ON%20p.id%20=%20u.id%20LIMIT%201",
    );
    assert!(invalid_join.starts_with("HTTP/1.1 400 Bad Request\r\n"), "{invalid_join}");
    let invalid_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&invalid_join)).expect("invalid join payload");
    assert_eq!(invalid_payload["code"], "unsupported_feature");
    assert!(
        invalid_payload["error"].as_str().expect("error").contains("not backed by schema metadata"),
        "{invalid_join}"
    );
}

fn start_server(folder: &std::path::Path, readonly: bool) -> (ChildGuard, String) {
    if readonly {
        support::spawn_folder_server(folder, true)
    } else {
        support::spawn_folder_server(folder, false)
    }
}
