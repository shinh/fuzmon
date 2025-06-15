use fuzmon::test_utils::{create_config, run_fuzmon_output};
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
    let out = run_fuzmon_output(env!("CARGO_BIN_EXE_fuzmon"), pid, &logdir, &cfg);

    let _ = child.wait();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("disappeared, exiting"), "{}", stdout);
}
