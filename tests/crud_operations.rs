use std::{fs, path::Path, time::Duration};

mod support;

use support::{http_request, parse_http_body, spawn_folder_server, wait_for_server};

#[test]
fn school_examples_support_students_crud_end_to_end() {
    let temp = tempfile::tempdir().expect("create temp directory");
    copy_example_folder("school", temp.path());

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);
    wait_for_server(&bind_addr, Duration::from_secs(5));

    let get_existing = http_request(&bind_addr, "GET", "/students/1", None);
    assert!(get_existing.starts_with("HTTP/1.1 200 OK\r\n"), "{get_existing}");
    let existing_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&get_existing)).expect("valid json body");
    assert_eq!(existing_payload["name"], "Alice Johnson");

    let post_response = http_request(
        &bind_addr,
        "POST",
        "/students",
        Some(
            r#"{"name":"Dina Patel","email":"dina.patel@example.edu","year":4,"major":"Biology"}"#,
        ),
    );
    assert!(post_response.starts_with("HTTP/1.1 201 Created\r\n"), "{post_response}");
    let created_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&post_response)).expect("valid json body");
    let created_id = created_payload["id"].as_i64().expect("created id");

    let put_response = http_request(
        &bind_addr,
        "PUT",
        &format!("/students/{created_id}"),
        Some(&format!(
            r#"{{"id":{created_id},"name":"Dina Patel","email":"dina.patel@example.edu","year":4,"major":"Data Science"}}"#
        )),
    );
    assert!(put_response.starts_with("HTTP/1.1 200 OK\r\n"), "{put_response}");
    let put_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&put_response)).expect("valid json body");
    assert_eq!(put_payload["major"], "Data Science");

    let after_put: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(temp.path().join("students.json")).expect("read students file"),
    )
    .expect("students json");
    let updated_after_put = after_put
        .as_array()
        .expect("array")
        .iter()
        .find(|student| student["id"] == created_id)
        .expect("created student");
    assert_eq!(updated_after_put["major"], "Data Science");

    let patch_response = http_request(
        &bind_addr,
        "PATCH",
        &format!("/students/{created_id}"),
        Some(r#"{"year":5}"#),
    );
    assert!(patch_response.starts_with("HTTP/1.1 200 OK\r\n"), "{patch_response}");
    let patch_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&patch_response)).expect("valid json body");
    assert_eq!(patch_payload["year"], 5);

    let after_patch: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(temp.path().join("students.json")).expect("read students file"),
    )
    .expect("students json");
    let updated_after_patch = after_patch
        .as_array()
        .expect("array")
        .iter()
        .find(|student| student["id"] == created_id)
        .expect("created student");
    assert_eq!(updated_after_patch["major"], "Data Science");
    assert_eq!(updated_after_patch["year"], 5);

    let delete_response =
        http_request(&bind_addr, "DELETE", &format!("/students/{created_id}"), None);
    assert!(delete_response.starts_with("HTTP/1.1 204 No Content\r\n"), "{delete_response}");

    let get_deleted = http_request(&bind_addr, "GET", &format!("/students/{created_id}"), None);
    assert!(get_deleted.starts_with("HTTP/1.1 404 Not Found\r\n"), "{get_deleted}");

    let students_file =
        fs::read_to_string(temp.path().join("students.json")).expect("read students file");
    let students: serde_json::Value = serde_json::from_str(&students_file).expect("students json");
    assert_eq!(
        students,
        serde_json::from_str::<serde_json::Value>(
            &fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/school/students.json")
            )
            .expect("read example students")
        )
        .expect("example students json")
    );
}

fn copy_example_folder(example_name: &str, destination: &Path) {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples").join(example_name);

    for entry in fs::read_dir(source_root).expect("read example dir") {
        let entry = entry.expect("example entry");
        let source = entry.path();
        if source.is_file() {
            let target = destination.join(entry.file_name());
            fs::copy(source, target).expect("copy example file");
        }
    }
}
