use std::fs;
use std::process::{Command, Stdio};
use tempfile::{NamedTempFile, tempdir};

use fuzmon::test_utils::wait_until_file_appears;
use fuzmon::utils::current_date_string;

fn run_with_format(fmt: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let logdir = tempdir().expect("logdir");
    let cfg_file = NamedTempFile::new().expect("cfg");
    let compress = if fmt.ends_with(".zst") {
        "true"
    } else {
        "false"
    };
    fs::write(
        cfg_file.path(),
        format!(
            "[output]\npath='{}'\nformat='{}'\ncompress={}",
            logdir.path().display(),
            fmt,
            compress
        ),
    )
    .expect("write cfg");

    let mut child = Command::new("python3")
        .arg("-c")
        .arg("import sys; sys.stdin.read()")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn python");

    let pid = child.id();

    let pid_s = pid.to_string();
    let cfg_path = cfg_file.path().to_str().unwrap().to_string();
    let logdir_s = logdir.path().to_str().unwrap().to_string();
    let cmd_args = vec!["run", "-p", &pid_s, "-o", &logdir_s, "-c", &cfg_path];
    let mut mon = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args(&cmd_args)
        .stdout(Stdio::null())
        .spawn()
        .expect("run fuzmon");

    wait_until_file_appears(&logdir, pid);
    fuzmon::test_utils::kill_with_sigint_and_wait(&mut mon);

    fuzmon::test_utils::kill_with_sigint_and_wait(&mut child);

    let date = current_date_string();
    let subdir = logdir.path().join(date);
    let path = fs::read_dir(&subdir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    (logdir, path)
}

fn run_default() -> (tempfile::TempDir, std::path::PathBuf) {
    let logdir = tempdir().expect("logdir");
    let mut child = Command::new("python3")
        .arg("-c")
        .arg("import sys; sys.stdin.read()")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn python");

    let pid = child.id();

    let pid_s = pid.to_string();
    let logdir_s = logdir.path().to_str().unwrap().to_string();
    let cmd_args = vec!["run", "-p", &pid_s, "-o", &logdir_s];
    let mut mon = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args(&cmd_args)
        .stdout(Stdio::null())
        .spawn()
        .expect("run fuzmon");

    wait_until_file_appears(&logdir, pid);
    fuzmon::test_utils::kill_with_sigint_and_wait(&mut mon);

    fuzmon::test_utils::kill_with_sigint_and_wait(&mut child);

    let date = current_date_string();
    let subdir = logdir.path().join(date);
    let path = fs::read_dir(&subdir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    (logdir, path)
}

fn dump_file(path: &std::path::Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args(["dump", path.to_str().unwrap()])
        .output()
        .expect("run dump");
    let mut s = String::new();
    s.push_str(&String::from_utf8_lossy(&out.stdout));
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    s
}

#[test]
fn default_is_jsonl_zst() {
    let (dir, path) = run_default();
    assert_eq!(path.extension().and_then(|e| e.to_str()), Some("zst"));
    let out = dump_file(&path);
    println!("out: {}", out);
    assert!(out.contains("process_name"));
    drop(dir);
}

#[test]
fn jsonl_output_and_dump() {
    let (dir, path) = run_with_format("jsonl");
    assert_eq!(path.extension().and_then(|e| e.to_str()), Some("jsonl"));
    let out = dump_file(&path);
    println!("out: {}", out);
    assert!(out.contains("process_name"));
    drop(dir);
}

#[test]
fn msgpacks_output_and_dump() {
    let (dir, path) = run_with_format("msgpacks");
    assert_eq!(path.extension().and_then(|e| e.to_str()), Some("msgpacks"));
    let out = dump_file(&path);
    println!("out: {}", out);
    assert!(out.contains("process_name"));
    drop(dir);
}

#[test]
fn msgpacks_zst_output_and_dump() {
    let (dir, path) = run_with_format("msgpacks.zst");
    assert_eq!(path.extension().and_then(|e| e.to_str()), Some("zst"));
    let out = dump_file(&path);
    println!("out: {}", out);
    assert!(out.contains("process_name"));
    drop(dir);
}
