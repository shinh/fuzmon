use std::process::{Command, Stdio};
use std::io::Write;
use std::{thread, time::Duration};
use tempfile::tempdir;
use std::fs;

mod common;

#[test]
fn detect_fd_open_close() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("testfile");
    let script = dir.path().join("script.py");
    fs::write(
        &script,
        r#"import sys
sys.stdin.readline()
f=open(sys.argv[1], 'w')
sys.stdin.readline()
f.close()
sys.stdin.readline()
"#,
    )
    .expect("write script");

    let mut child = Command::new("python3")
        .arg(&script)
        .arg(&file_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn python");

    let pid = child.id();
    let mut child_in = child.stdin.take().expect("stdin");

    let logdir = tempdir().expect("logdir");
    let mut mon = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args(["run", "-p", &pid.to_string(), "-o", logdir.path().to_str().unwrap()])
        .stdout(Stdio::null())
        .spawn()
        .expect("run fuzmon");

    let log_path = logdir.path().join(format!("{}.log", pid));
    for _ in 0..50 {
        if log_path.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    child_in.write_all(b"\n").unwrap();
    child_in.flush().unwrap();

    for _ in 0..50 {
        if log_path.exists() {
            if let Ok(s) = fs::read_to_string(&log_path) {
                if s.contains("\"event\":\"open\"") && s.contains(file_path.to_str().unwrap()) {
                    break;
                }
            }
        }
        thread::sleep(Duration::from_millis(10));
    }

    child_in.write_all(b"\n").unwrap();
    child_in.flush().unwrap();

    for _ in 0..50 {
        if let Ok(s) = fs::read_to_string(&log_path) {
            if s.contains("\"event\":\"close\"") {
                break;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }

    child_in.write_all(b"\n").unwrap();
    drop(child_in);

    let _ = child.wait();
    let _ = mon.kill();
    let _ = mon.wait();

    let log_content = fs::read_to_string(&log_path).unwrap_or_default();
    let logfile = file_path.to_str().unwrap();
    assert!(log_content.contains("\"event\":\"open\""), "{}", log_content);
    assert!(log_content.contains(logfile), "{}", log_content);
    assert!(log_content.contains("\"event\":\"close\""), "{}", log_content);
}

