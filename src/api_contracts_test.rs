use std::fs;

#[path = "test_support/mod.rs"]
mod support;

use support::{
    TestDatasetBuilder, TestServerBuilder, assert_status, http_get, parse_http_body,
    parse_json_body, request_json, request_text, request_with_headers, wait_for_event,
};

#[test]
fn rest_json_crud_and_object_routes_keep_response_contracts() {
    let data = TestDatasetBuilder::new()
        .json_file("users.json", serde_json::json!([{"id": 1, "name": "Ada"}]))
        .json_file("profile.json", serde_json::json!({"name": "Ada", "theme": "dark"}));
    let users_path = data.file_path("users.json");
    let profile_path = data.file_path("profile.json");
    let (_child, addr) = TestServerBuilder::folder(data.path()).spawn();

    let resources = request_text(&addr, "GET", "/", None);
    assert_status(&resources, "200 OK");
    assert_eq!(parse_json_body(&resources), serde_json::json!({"resources": ["profile", "users"]}));

    let list = request_json(&addr, "GET", "/users", None);
    assert_eq!(list, serde_json::json!([{"id": 1, "name": "Ada"}]));

    let item = request_json(&addr, "GET", "/users/1", None);
    assert_eq!(item, serde_json::json!({"id": 1, "name": "Ada"}));

    let created = request_text(&addr, "POST", "/users", Some(r#"{"name":"Grace"}"#));
    assert_status(&created, "201 Created");
    assert_eq!(parse_json_body(&created), serde_json::json!({"id": 2, "name": "Grace"}));

    let replaced = request_json(&addr, "PUT", "/users/2", Some(serde_json::json!({"name": "Lin"})));
    assert_eq!(replaced, serde_json::json!({"id": 2, "name": "Lin"}));

    let patched =
        request_json(&addr, "PATCH", "/users/2", Some(serde_json::json!({"role": "admin"})));
    assert_eq!(patched, serde_json::json!({"id": 2, "name": "Lin", "role": "admin"}));

    let deleted = request_text(&addr, "DELETE", "/users/2", None);
    assert_status(&deleted, "204 No Content");
    let persisted_users: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(users_path).expect("read users"))
            .expect("users json");
    assert_eq!(persisted_users, serde_json::json!([{"id": 1, "name": "Ada"}]));

    let object_put = request_json(
        &addr,
        "PUT",
        "/profile",
        Some(serde_json::json!({"name": "Grace", "theme": "light"})),
    );
    assert_eq!(object_put, serde_json::json!({"name": "Grace", "theme": "light"}));
    let object_patch =
        request_json(&addr, "PATCH", "/profile", Some(serde_json::json!({"theme": "dark"})));
    assert_eq!(object_patch, serde_json::json!({"name": "Grace", "theme": "dark"}));
    let persisted_profile: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(profile_path).expect("read profile"))
            .expect("profile json");
    assert_eq!(persisted_profile, serde_json::json!({"name": "Grace", "theme": "dark"}));
}

#[test]
fn readonly_auth_cors_xml_and_ops_contracts_are_stable() {
    let data = TestDatasetBuilder::new()
        .json_file("users.json", serde_json::json!([{"id": 1, "name": "Ada"}]));

    let (_readonly_child, readonly_addr) =
        TestServerBuilder::folder(data.path()).arg("--readonly").spawn();
    let readonly_post = request_text(&readonly_addr, "POST", "/users", Some(r#"{"name":"Grace"}"#));
    assert_status(&readonly_post, "405 Method Not Allowed");

    let (_auth_child, auth_addr) = TestServerBuilder::folder(data.path())
        .args(&["--auth-token", "secret", "--cors-origin", "http://example.com"])
        .spawn();
    let unauthorized = request_text(&auth_addr, "GET", "/users", None);
    assert_status(&unauthorized, "401 Unauthorized");
    assert_eq!(
        parse_json_body(&unauthorized),
        serde_json::json!({"error": "Missing or invalid bearer token", "code": "unauthorized"})
    );

    let authorized = request_with_headers(
        &auth_addr,
        "GET",
        "/users",
        "Authorization: Bearer secret\r\nOrigin: http://example.com\r\n",
        None,
    );
    assert_status(&authorized, "200 OK");
    assert!(authorized.contains("access-control-allow-origin: http://example.com"), "{authorized}");
    assert!(authorized.contains("content-type: application/json"), "{authorized}");

    let preflight = request_with_headers(
        &auth_addr,
        "OPTIONS",
        "/users",
        "Origin: http://example.com\r\nAccess-Control-Request-Method: POST\r\n",
        None,
    );
    assert_status(&preflight, "204 No Content");
    assert!(preflight.contains("access-control-allow-methods: GET,POST,PUT,PATCH,DELETE,OPTIONS"));

    let health = http_get(&auth_addr, "/healthz");
    assert_status(&health, "200 OK");
    let ready = http_get(&auth_addr, "/readyz");
    assert_status(&ready, "200 OK");
    let metrics = http_get(&auth_addr, "/metrics");
    assert_status(&metrics, "200 OK");
    assert!(metrics.contains("dirbase_requests_total"), "{metrics}");
    assert!(metrics.contains("dirbase_auth_failures_total"), "{metrics}");

    let (_protected_child, protected_addr) = TestServerBuilder::folder(data.path())
        .args(&["--auth-token", "secret", "--protect-ops"])
        .spawn();
    let protected_ready = request_text(&protected_addr, "GET", "/readyz", None);
    assert_status(&protected_ready, "401 Unauthorized");
    let protected_metrics = request_text(&protected_addr, "GET", "/metrics", None);
    assert_status(&protected_metrics, "401 Unauthorized");
    let public_health = request_text(&protected_addr, "GET", "/healthz", None);
    assert_status(&public_health, "200 OK");

    let (_xml_child, xml_addr) = TestServerBuilder::folder(data.path()).arg("--xml").spawn();
    let xml = request_text(&xml_addr, "GET", "/users", None);
    assert_status(&xml, "200 OK");
    assert!(xml.contains("content-type: application/xml; charset=utf-8"), "{xml}");
    assert!(parse_http_body(&xml).contains("<response"), "{xml}");
}

#[test]
fn events_contract_reports_resource_schema_and_overview_changes() {
    let data = TestDatasetBuilder::new()
        .json_file("users.json", serde_json::json!([{"id": 1, "name": "Ada"}]));
    let (_child, addr) = TestServerBuilder::folder(data.path()).spawn();
    let mut stream = support::open_sse_stream(&addr, "/events");

    let create = request_text(&addr, "POST", "/users", Some(r#"{"name":"Grace"}"#));
    assert_status(&create, "201 Created");

    let events = wait_for_event(&mut stream, "event: schema_changed");
    assert!(events.contains("event: resource_changed"), "{events}");
    assert!(events.contains("event: overview_changed"), "{events}");
}

#[test]
fn openapi_contract_describes_resources_readonly_auth_and_reserved_name() {
    let data = TestDatasetBuilder::new()
        .json_file(
            "users.json",
            serde_json::json!([{"user_id": "u1", "name": "Ada", "role": "admin"}]),
        )
        .json_file("profile.json", serde_json::json!({"name": "Ada", "theme": "dark"}))
        .json_file("openapi.json", serde_json::json!([{"id": 1}]))
        .json_file(
            "schema.json",
            serde_json::json!({
                "tables": {
                    "users": {
                        "primary_key": "user_id",
                        "columns": {
                            "user_id": { "column_type": "uuid", "nullable": false },
                            "name": { "column_type": "string", "nullable": false },
                            "role": {
                                "column_type": "string",
                                "nullable": false,
                                "enum_values": ["admin", "reader"]
                            }
                        }
                    }
                }
            }),
        );
    let (_child, addr) = TestServerBuilder::folder(data.path()).spawn();

    let raw = request_text(&addr, "GET", "/openapi.json", None);
    assert_status(&raw, "200 OK");
    assert!(raw.contains("content-type: application/json"), "{raw}");
    let payload = parse_json_body(&raw);
    assert_eq!(payload["openapi"], "3.1.0");
    assert_eq!(payload["info"]["title"], "dirbase");
    assert_eq!(
        payload["paths"]["/resources/{resource}"]["delete"]["operationId"],
        "deleteResource"
    );

    let paths = payload["paths"].as_object().expect("paths object");
    assert!(paths.contains_key("/openapi.json"));
    assert!(paths.contains_key("/users"));
    assert!(paths.contains_key("/users/{id}"));
    assert!(paths.contains_key("/profile"));
    assert!(!paths.contains_key("/profile/{id}"));
    assert_eq!(payload["paths"]["/users"]["get"]["operationId"], "getUsersCollection");
    assert_eq!(payload["paths"]["/users"]["post"]["operationId"], "createUsersItem");
    assert_eq!(payload["paths"]["/profile"]["put"]["operationId"], "replaceProfileObject");

    let id_description = payload["paths"]["/users/{id}"]["get"]["parameters"][0]["description"]
        .as_str()
        .expect("id description");
    assert!(id_description.contains("user_id"), "{id_description}");
    assert_eq!(
        payload["components"]["schemas"]["UsersResource"]["properties"]["user_id"]["format"],
        "uuid"
    );
    assert_eq!(
        payload["components"]["schemas"]["UsersResource"]["properties"]["role"]["enum_values"],
        serde_json::Value::Null
    );
    assert_eq!(
        payload["components"]["schemas"]["UsersResource"]["properties"]["role"]["enum"],
        serde_json::json!(["admin", "reader"])
    );

    let resources = request_json(&addr, "GET", "/resources", None);
    assert_eq!(resources, serde_json::json!({"resources": ["profile", "users"]}));
    let reserved_create =
        request_text(&addr, "POST", "/resources", Some(r#"{"name":"openapi","initial":[]}"#));
    assert_status(&reserved_create, "400 Bad Request");

    let (_readonly_child, readonly_addr) =
        TestServerBuilder::folder(data.path()).arg("--readonly").spawn();
    let readonly_payload = request_json(&readonly_addr, "GET", "/openapi.json", None);
    let readonly_paths = readonly_payload["paths"].as_object().expect("readonly paths");
    assert!(!readonly_paths.contains_key("/schema/infer"));
    assert!(
        readonly_payload["paths"]["/users"].as_object().expect("users path").get("post").is_none()
    );
    assert!(
        readonly_payload["paths"]["/users/{id}"]
            .as_object()
            .expect("item path")
            .get("delete")
            .is_none()
    );
    assert!(
        readonly_payload["paths"]["/profile"]
            .as_object()
            .expect("profile path")
            .get("put")
            .is_none()
    );
    assert!(readonly_payload["paths"]["/sql"].as_object().expect("sql path").get("post").is_none());

    let (_auth_child, auth_addr) =
        TestServerBuilder::folder(data.path()).args(&["--auth-token", "secret"]).spawn();
    let unauthorized = request_text(&auth_addr, "GET", "/openapi.json", None);
    assert_status(&unauthorized, "401 Unauthorized");
    let authorized = request_with_headers(
        &auth_addr,
        "GET",
        "/openapi.json",
        "Authorization: Bearer secret\r\n",
        None,
    );
    assert_status(&authorized, "200 OK");
    assert_eq!(parse_json_body(&authorized)["openapi"], "3.1.0");
}

#[test]
fn representative_error_bodies_remain_unchanged() {
    let data = TestDatasetBuilder::new()
        .json_file("users.json", serde_json::json!([{"id": 1, "name": "Ada"}]));
    let (_child, addr) = TestServerBuilder::folder(data.path()).spawn();

    let missing = request_text(&addr, "GET", "/users/404", None);
    assert_status(&missing, "404 Not Found");
    assert_eq!(parse_json_body(&missing), serde_json::json!({"error": "Item not found"}));

    let non_array_item_route = request_text(&addr, "GET", "/missing/1", None);
    assert_status(&non_array_item_route, "404 Not Found");
    assert_eq!(
        parse_json_body(&non_array_item_route),
        serde_json::json!({"error": "Resource 'missing' not found"})
    );
}
