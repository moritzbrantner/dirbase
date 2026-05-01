use std::fs;

#[path = "../test_support/mod.rs"]
mod support;

use support::{http_get, parse_http_body, spawn_folder_server_with_args};

#[test]
fn xml_mode_returns_collection_responses_as_xml() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("users.json"),
        r#"[{"id":1,"name":"Ada & Grace","active":true,"score":9.5,"nickname":null,"tags":["admin",2]}]"#,
    )
    .expect("write users");

    let (_child, bind_addr) = spawn_folder_server_with_args(temp.path(), &["--xml"]);

    let response = http_get(&bind_addr, "/users");
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(response.contains("content-type: application/xml; charset=utf-8"), "{response}");

    let body = parse_http_body(&response);
    assert!(body.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"), "{body}");
    assert_xml_fragment(body, r#"<response type="array">"#);
    assert_xml_fragment(body, r#"<item type="object">"#);
    assert_xml_fragment(body, r#"<id type="integer">1</id>"#);
    assert_xml_fragment(body, r#"<name type="string">Ada &amp; Grace</name>"#);
    assert_xml_fragment(body, r#"<active type="boolean">true</active>"#);
    assert_xml_fragment(body, r#"<score type="float">9.5</score>"#);
    assert_xml_fragment(body, r#"<nickname type="null"></nickname>"#);
    assert_xml_fragment(
        body,
        r#"<tags type="array"><item type="string">admin</item><item type="integer">2</item></tags>"#,
    );
}

#[test]
fn xml_mode_returns_object_responses_with_typed_nested_values() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("settings.json"),
        r#"{"title":"Dirbase","limits":{"max":10,"ratio":0.75},"features":["xml",true],"deleted_at":null}"#,
    )
    .expect("write settings");

    let (_child, bind_addr) = spawn_folder_server_with_args(temp.path(), &["--xml"]);

    let response = http_get(&bind_addr, "/settings");
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(response.contains("content-type: application/xml; charset=utf-8"), "{response}");

    let body = parse_http_body(&response);
    assert_xml_fragment(body, r#"<response type="object">"#);
    assert_xml_fragment(body, r#"<title type="string">Dirbase</title>"#);
    assert_xml_fragment(
        body,
        r#"<limits type="object"><max type="integer">10</max><ratio type="float">0.75</ratio></limits>"#,
    );
    assert_xml_fragment(
        body,
        r#"<features type="array"><item type="string">xml</item><item type="boolean">true</item></features>"#,
    );
    assert_xml_fragment(body, r#"<deleted_at type="null"></deleted_at>"#);
}

#[test]
fn xml_mode_uses_field_elements_for_json_keys_that_are_not_xml_names() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("records.json"),
        r#"[{"1 bad key":"quoted \"value\"","xmlField":"reserved prefix","safe_key":"safe"}]"#,
    )
    .expect("write records");

    let (_child, bind_addr) = spawn_folder_server_with_args(temp.path(), &["--xml"]);

    let response = http_get(&bind_addr, "/records");
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");

    let body = parse_http_body(&response);
    assert_xml_fragment(body, r#"<field name="1 bad key" type="string">quoted "value"</field>"#);
    assert_xml_fragment(body, r#"<field name="xmlField" type="string">reserved prefix</field>"#);
    assert_xml_fragment(body, r#"<safe_key type="string">safe</safe_key>"#);
}

#[test]
fn xml_mode_returns_error_responses_as_xml() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("users.json"), r#"[{"id":1,"name":"Ada"}]"#).expect("write users");

    let (_child, bind_addr) = spawn_folder_server_with_args(temp.path(), &["--xml"]);

    let response = http_get(&bind_addr, "/users/999");
    assert!(response.starts_with("HTTP/1.1 404 Not Found\r\n"), "{response}");
    assert!(response.contains("content-type: application/xml; charset=utf-8"), "{response}");

    let body = parse_http_body(&response);
    assert_xml_fragment(body, r#"<response type="object">"#);
    assert_xml_fragment(body, r#"<error type="string">Item not found</error>"#);
}

#[test]
fn xml_mode_returns_auth_errors_as_xml() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(temp.path().join("users.json"), r#"[{"id":1,"name":"Ada"}]"#).expect("write users");

    let (_child, bind_addr) =
        spawn_folder_server_with_args(temp.path(), &["--xml", "--auth-token", "secret"]);

    let response = http_get(&bind_addr, "/users");
    assert!(response.starts_with("HTTP/1.1 401 Unauthorized\r\n"), "{response}");
    assert!(response.contains("content-type: application/xml; charset=utf-8"), "{response}");

    let body = parse_http_body(&response);
    assert_xml_fragment(body, r#"<response type="object">"#);
    assert_xml_fragment(body, r#"<error type="string">Missing or invalid bearer token</error>"#);
    assert_xml_fragment(body, r#"<code type="string">unauthorized</code>"#);
}

fn assert_xml_fragment(body: &str, expected: &str) {
    assert!(body.contains(expected), "missing {expected:?} in {body}");
}
