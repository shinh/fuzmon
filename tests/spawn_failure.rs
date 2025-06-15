use std::process::Command;

#[test]
fn spawn_invalid_command_prints_message() {
    let out = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args(["run", "/bin/hogeee"])
        .output()
        .expect("run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("failed to spawn"), "{}", stdout);
}
