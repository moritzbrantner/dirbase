use std::process::Command;

#[test]
fn prints_help_when_no_arguments_are_provided() {
    let output = Command::new(env!("CARGO_BIN_EXE_folder-server"))
        .output()
        .expect("run folder-server without arguments");

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Usage:"), "stdout was: {stdout}");
    assert!(stdout.contains("--folder <FOLDER>"), "stdout was: {stdout}");
}
