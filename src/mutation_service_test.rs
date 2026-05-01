use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    sync::{Arc, Barrier},
    thread,
    time::Duration,
};

#[path = "test_support/mod.rs"]
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
            r#"{"name":"Dina Patel","email":"dina.patel@example.edu","year":4,"major":"Biology","enrollment_status":"active"}"#,
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
            r#"{{"id":{created_id},"name":"Dina Patel","email":"dina.patel@example.edu","year":4,"major":"Data Science","enrollment_status":"active"}}"#
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

#[test]
fn school_example_schema_covers_many_to_many_and_nullable_cases() {
    let temp = tempfile::tempdir().expect("create temp directory");
    copy_example_folder("school", temp.path());

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);
    wait_for_server(&bind_addr, Duration::from_secs(5));

    let schema_response = http_request(&bind_addr, "GET", "/schema", None);
    assert!(schema_response.starts_with("HTTP/1.1 200 OK\r\n"), "{schema_response}");
    let schema_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&schema_response)).expect("schema json");

    assert_eq!(schema_payload["tables"]["enrollments"]["kind"], "relation");
    assert_eq!(
        schema_payload["tables"]["students"]["many_to_many"]["classes"]["through_table"],
        "enrollments"
    );
    assert_eq!(
        schema_payload["tables"]["classes"]["many_to_many"]["students"]["through_table"],
        "enrollments"
    );
    assert_eq!(
        schema_payload["tables"]["students"]["columns"]["advisor_professor_id"]["nullable"],
        true
    );
    assert_eq!(schema_payload["tables"]["students"]["columns"]["gpa"]["nullable"], true);
    assert_eq!(schema_payload["tables"]["classes"]["columns"]["professor_id"]["nullable"], true);

    let student_response = http_request(&bind_addr, "GET", "/students/5", None);
    assert!(student_response.starts_with("HTTP/1.1 200 OK\r\n"), "{student_response}");
    let student_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&student_response)).expect("student json");
    assert!(student_payload["gpa"].is_null(), "{student_payload}");
    assert!(student_payload["advisor_professor_id"].is_null(), "{student_payload}");

    let class_response = http_request(&bind_addr, "GET", "/classes/499", None);
    assert!(class_response.starts_with("HTTP/1.1 200 OK\r\n"), "{class_response}");
    let class_payload: serde_json::Value =
        serde_json::from_str(parse_http_body(&class_response)).expect("class json");
    assert!(class_payload["professor_id"].is_null(), "{class_payload}");
    assert_eq!(class_payload["student_ids"], serde_json::json!([]));
}

#[test]
fn concurrent_posts_to_same_collection_do_not_lose_writes() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("users.json"),
        r#"[
  {"id": 1, "name": "Ada"}
]
"#,
    )
    .expect("write users");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);
    wait_for_server(&bind_addr, Duration::from_secs(5));

    let barrier = Arc::new(Barrier::new(9));
    let mut handles = Vec::new();
    for index in 0..8 {
        let bind_addr = bind_addr.clone();
        let barrier = barrier.clone();
        handles.push(thread::spawn(move || {
            barrier.wait();
            http_request(
                &bind_addr,
                "POST",
                "/users",
                Some(&format!(r#"{{"name":"User {index}"}}"#)),
            )
        }));
    }

    barrier.wait();
    let mut created_ids = BTreeSet::new();
    for handle in handles {
        let response = handle.join().expect("join request thread");
        assert!(response.starts_with("HTTP/1.1 201 Created\r\n"), "{response}");
        let payload: serde_json::Value =
            serde_json::from_str(parse_http_body(&response)).expect("created json");
        assert!(created_ids.insert(payload["id"].as_i64().expect("numeric id")));
    }

    let persisted: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(temp.path().join("users.json")).expect("read users"),
    )
    .expect("users json");
    let rows = persisted.as_array().expect("users array");
    let persisted_ids = rows
        .iter()
        .map(|row| row["id"].as_i64().expect("persisted numeric id"))
        .collect::<BTreeSet<_>>();

    assert_eq!(created_ids, BTreeSet::from([2, 3, 4, 5, 6, 7, 8, 9]));
    assert_eq!(persisted_ids, BTreeSet::from([1, 2, 3, 4, 5, 6, 7, 8, 9]));
    assert_eq!(rows.len(), 9);
}

#[test]
fn concurrent_update_and_delete_requests_leave_valid_collection_state() {
    let temp = tempfile::tempdir().expect("create temp directory");
    fs::write(
        temp.path().join("posts.json"),
        r#"[
  {"id": 1, "title": "hello", "status": "draft"}
]
"#,
    )
    .expect("write posts");

    let (_child, bind_addr) = spawn_folder_server(temp.path(), false);
    wait_for_server(&bind_addr, Duration::from_secs(5));

    let barrier = Arc::new(Barrier::new(3));
    let patch_addr = bind_addr.clone();
    let patch_barrier = barrier.clone();
    let patch_handle = thread::spawn(move || {
        patch_barrier.wait();
        http_request(&patch_addr, "PATCH", "/posts/1", Some(r#"{"status":"published"}"#))
    });

    let delete_addr = bind_addr.clone();
    let delete_barrier = barrier.clone();
    let delete_handle = thread::spawn(move || {
        delete_barrier.wait();
        http_request(&delete_addr, "DELETE", "/posts/1", None)
    });

    barrier.wait();
    let patch_response = patch_handle.join().expect("join patch request");
    let delete_response = delete_handle.join().expect("join delete request");

    assert!(delete_response.starts_with("HTTP/1.1 204 No Content\r\n"), "{delete_response}");
    assert!(
        patch_response.starts_with("HTTP/1.1 200 OK\r\n")
            || patch_response.starts_with("HTTP/1.1 404 Not Found\r\n"),
        "{patch_response}"
    );

    let persisted: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(temp.path().join("posts.json")).expect("read posts"),
    )
    .expect("posts json");
    assert_eq!(persisted, serde_json::json!([]));
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
