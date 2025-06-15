use std::process::{Command, Stdio};
use std::{thread, time::Duration};
use std::fs;
use tempfile::tempdir;

#[test]
fn symbolized_stack_trace_contains_function() {
    let dir = tempdir().expect("tempdir");
    let src_path = dir.path().join("testprog.c");
    fs::write(&src_path, r#"
#include <unistd.h>

void target_function() {
    while (1) {
        sleep(1);
    }
}

int main() {
    target_function();
    return 0;
}
"#).expect("write src");
    let exe_path = dir.path().join("testprog");
    let status = Command::new("gcc")
        .args([
            "-g",
            "-O0",
            "-fno-omit-frame-pointer",
            "-no-pie",
            src_path.to_str().unwrap(),
            "-o",
            exe_path.to_str().unwrap(),
        ])
        .status()
        .expect("compile test program");
    assert!(status.success());

    let mut child = Command::new(&exe_path)
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn test program");

    thread::sleep(Duration::from_millis(500));

    let pid = child.id();
    let logdir = tempdir().expect("logdir");
    let mut mon = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args(["-p", &pid.to_string(), "-o", logdir.path().to_str().unwrap()])
        .stdout(Stdio::null())
        .spawn()
        .expect("run fuzmon");

    thread::sleep(Duration::from_millis(800));
    let _ = mon.kill();
    let _ = mon.wait();

    let _ = child.kill();
    let _ = child.wait();

    let log_path = logdir.path().join(format!("{}.log", pid));
    let log = fs::read_to_string(log_path).expect("read log");
    assert!(log.contains("target_function"), "{}", log);
    assert!(log.contains("main"), "{}", log);
    assert!(log.contains("testprog.c"), "{}", log);
}

