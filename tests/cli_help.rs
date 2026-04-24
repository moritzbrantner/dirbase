use std::process::Command;

#[test]
fn prints_help_when_no_arguments_are_provided() {
    let output = Command::new(env!("CARGO_BIN_EXE_dirbase"))
        .output()
        .expect("run dirbase without arguments");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage:"), "stdout was: {stdout}");
    assert!(stdout.contains("[PATH]"), "stdout was: {stdout}");
    assert!(stdout.contains("--folder <FOLDER>"), "stdout was: {stdout}");
    assert!(stdout.contains("--file <FILE>"), "stdout was: {stdout}");
    assert!(stdout.contains("--port <PORT>"), "stdout was: {stdout}");
    assert!(
        stdout.contains(
            "Path to a folder of *.json files or a single json-server-style database file."
        ),
        "stdout was: {stdout}"
    );
    assert!(stdout.contains("Examples:"), "stdout was: {stdout}");
    assert!(stdout.contains("dirbase ./data"), "stdout was: {stdout}");
    assert!(stdout.contains("dirbase.conf"), "stdout was: {stdout}");
    assert!(stdout.contains("Use one of [PATH], --folder, or --file."), "stdout was: {stdout}");
}
