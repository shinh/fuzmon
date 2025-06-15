use std::fs;
use std::process::Command;
use tempfile::tempdir;

use fuzmon::test_utils::{collect_log_content, create_config};
use fuzmon::utils::current_date_string;

#[test]
fn spawn_and_monitor_command() {
    let dir = tempdir().expect("dir");
    let cfg = create_config(0.0);
    let out = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args([
            "run",
            "-o",
            dir.path().to_str().unwrap(),
            "-c",
            cfg.path().to_str().unwrap(),
            "/bin/sleep",
            "1",
        ])
        .output()
        .expect("run");
    assert!(out.status.success());
    let date = current_date_string();
    let sub = dir.path().join(date);
    let log_content = collect_log_content(&dir);
    assert!(fs::read_dir(sub).unwrap().next().is_some(), "no log file");
    assert!(!log_content.is_empty(), "log empty");
}
