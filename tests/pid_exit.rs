use fuzmon::test_utils::{create_config, run_fuzmon_output};
use std::process::{Command, Stdio};
use tempfile::tempdir;

#[test]
fn exits_when_pid_disappears() {
    let mut child = Command::new("sh")
        .args(["-c", "read dummy"])
        .stdin(Stdio::piped())
        .spawn()
        .expect("spawn read");
    let pid = child.id();

    let logdir = tempdir().expect("logdir");
    let cfg = create_config(1000.0);

    let handle = std::thread::spawn(move || {
        run_fuzmon_output(env!("CARGO_BIN_EXE_fuzmon"), pid, &logdir, &cfg)
    });

    // Close stdin so the script exits
    drop(child.stdin.take());

    let out = handle.join().expect("join fuzmon");

    let _ = child.wait();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("disappeared, exiting"), "{}", stdout);
}
