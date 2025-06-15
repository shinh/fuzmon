use std::process::{Command, Stdio};
use std::{thread, time::Duration};
use tempfile::tempdir;
use std::fs;

mod common;

#[test]
fn detect_fd_open_close() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("testfile");
    let script = dir.path().join("script.py");
    fs::write(&script,
"import time, sys\n\
time.sleep(0.2)\n\
f=open(sys.argv[1],'w')\n\
time.sleep(0.2)\n\
f.close()\n\
time.sleep(1)\n").expect("write script");

    let mut child = Command::new("python3")
        .arg(&script)
        .arg(&file_path)
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn python");

    thread::sleep(Duration::from_millis(100));
    let pid = child.id();

    let logdir = tempdir().expect("logdir");
    common::run_fuzmon_and_check(
        &["-p", &pid.to_string(), "-o", logdir.path().to_str().unwrap()],
        &["\"event\":\"open\"", "\"event\":\"close\""]);

    let _ = child.kill();
    let _ = child.wait();
}

