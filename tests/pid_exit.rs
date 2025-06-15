use fuzmon::test_utils::create_config;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn exits_when_pid_disappears() {
    let mut child = Command::new("sleep")
        .arg("0.2")
        .spawn()
        .expect("spawn sleep");
    let pid = child.id();

    let logdir = tempdir().expect("logdir");
    let cfg = create_config(1000.0);
    let out = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args([
            "run",
            "-p",
            &pid.to_string(),
            "-o",
            logdir.path().to_str().unwrap(),
            "-c",
            cfg.path().to_str().unwrap(),
        ])
        .output()
        .expect("run fuzmon");

    let _ = child.wait();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("終了します"), "{}", stdout);
}
