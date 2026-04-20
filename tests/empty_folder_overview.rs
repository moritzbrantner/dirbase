mod support;

use support::http_get;

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
