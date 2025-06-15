use fuzmon::test_utils::run_fuzmon;
use serde_json::Value;
use std::process::{Command, Stdio};
use tempfile::tempdir;

#[test]
fn first_entry_contains_cmdline_and_env() {
    let logdir = tempdir().expect("logdir");
    let mut child = Command::new("sleep")
        .arg("1")
        .env("META_VAR", "xyz")
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn sleep");
    let pid = child.id();
    let log = run_fuzmon(env!("CARGO_BIN_EXE_fuzmon"), pid, &logdir);
    unsafe {
        let _ = nix::libc::kill(child.id() as i32, nix::libc::SIGINT);
    }
    let _ = child.wait();
    let first = log.lines().next().expect("line");
    let v: Value = serde_json::from_str(first).expect("json");
    assert_eq!(v.get("cmdline").and_then(|s| s.as_str()), Some("sleep 1"));
    let env = v.get("env").and_then(|s| s.as_str()).unwrap_or("");
    assert!(env.contains("META_VAR=xyz"), "{}", env);
}
