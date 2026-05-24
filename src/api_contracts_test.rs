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
