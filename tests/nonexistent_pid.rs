use std::process::Command;
use std::time::Instant;
use tempfile::tempdir;

#[test]
fn nonexistent_pid_exits_immediately() {
    let dir = tempdir().expect("dir");
    let start = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args(["run", "-p", "9999999", "-o", dir.path().to_str().unwrap()])
        .output()
        .expect("run");
    assert!(start.elapsed().as_secs() < 2, "took {:?}", start.elapsed());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("not found"), "{}", stdout);
}
