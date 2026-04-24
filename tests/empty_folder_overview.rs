mod support;

use support::http_get;
use support::http_request_with_headers;

#[test]
fn empty_folder_returns_empty_overview_on_root() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let (_child, bind_addr) = support::spawn_folder_server(temp.path(), false);

    let response = http_get(&bind_addr, "/");

    assert!(
        response.starts_with("HTTP/1.1 200 OK\r\n"),
        "expected 200 OK response, got: {response}"
    );
    assert!(
        response.contains("{\"resources\":[]}"),
        "expected empty overview body, got: {response}"
    );
}

#[test]
fn empty_folder_html_overview_shows_bootstrap_examples() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let (_child, bind_addr) = support::spawn_folder_server(temp.path(), false);

    let response = http_request_with_headers(
        &bind_addr,
        "GET",
        "/",
        Some("Accept: text/html,application/xhtml+xml\r\n"),
        None,
    );

    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(response.contains("Create your first resource"), "{response}");
    assert!(response.contains(&format!("{}/users.json", temp.path().display())), "{response}");
    assert!(response.contains("&quot;name&quot;: &quot;Ada&quot;"), "{response}");
    assert!(response.contains("Use the example files above, then reload the page."), "{response}");
}
