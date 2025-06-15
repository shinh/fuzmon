use std::process::{Command, Stdio};
use std::{thread, time::Duration};
use std::fs;
use tempfile::tempdir;

mod common;

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
    common::run_fuzmon_and_check(
        &["-p", &pid.to_string(), "-o", logdir.path().to_str().unwrap()],
        &["target_function", "main", "testprog.c"],
    );

    let _ = child.kill();
    let _ = child.wait();
}


#[test]
fn symbolized_stack_trace_contains_function_no_pie() {
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
    common::run_fuzmon_and_check(
        &["-p", &pid.to_string(), "-o", logdir.path().to_str().unwrap()],
        &["target_function", "main", "testprog.c"],
    );

    let _ = child.kill();
    let _ = child.wait();
}
