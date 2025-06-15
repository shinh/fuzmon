use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn dump_outputs_entries() {
    let dir = tempdir().expect("tempdir");
    let log_path = dir.path().join("1.jsonl");
    fs::write(&log_path, "{\"timestamp\":\"0\",\"pid\":1,\"process_name\":\"t\",\"cpu_time_sec\":0,\"memory\":{\"rss_kb\":0,\"vsz_kb\":0,\"swap_kb\":0}}\n").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args(["dump", log_path.to_str().unwrap()])
        .output()
        .expect("run fuzmon dump");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("1.jsonl"));
    assert!(stdout.contains("process_name"));
}

#[test]
fn help_subcommand_shows_usage() {
    let out = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .arg("help")
        .output()
        .expect("run fuzmon help");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("fuzmon"));
    assert!(stdout.contains("run"));
}
