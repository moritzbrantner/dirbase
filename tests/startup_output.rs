mod support;

use std::{
    process::{Command, Stdio},
    thread,
    time::Duration,
};

#[test]
fn startup_prints_clickable_server_url() {
    let temp = tempfile::tempdir().expect("create temp directory");
    let bind_addr = support::next_addr();

    let mut child = Command::new(env!("CARGO_BIN_EXE_dirbase"))
        .arg("--folder")
        .arg(temp.path())
        .arg("--bind")
        .arg(&bind_addr)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start dirbase");

    support::wait_for_server(&bind_addr, Duration::from_secs(5));
    thread::sleep(Duration::from_millis(100));

    let _ = child.kill();
    let output = child.wait_with_output().expect("wait for dirbase output");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");

    assert!(combined.contains(&format!("http://{bind_addr}/")), "startup output was: {combined}");
}
