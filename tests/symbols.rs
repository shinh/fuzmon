use std::process::{Command, Stdio};
use std::{thread, time::Duration};
use std::fs;
use tempfile::tempdir;

mod common;

fn run_symbol_test(flags: &[&str], expected: &[&str]) {
    let dir = tempdir().expect("tempdir");
    let src_path = dir.path().join("testprog.c");
    fs::write(
        &src_path,
        r#"
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
"#,
    )
    .expect("write src");

    let exe_path = dir.path().join("testprog");
    let mut gcc_args: Vec<&str> = flags.to_vec();
    gcc_args.push(src_path.to_str().unwrap());
    gcc_args.push("-o");
    gcc_args.push(exe_path.to_str().unwrap());

    let status = Command::new("gcc")
        .args(&gcc_args)
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
        expected,
    );

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn symbolized_stack_trace_contains_function() {
    run_symbol_test(
        &["-g", "-O0"],
        &["target_function", "main", "sleep", "testprog.c"],
    );
}


#[test]
fn symbolized_stack_trace_contains_function_no_pie() {
    run_symbol_test(
        &["-g", "-O0", "-no-pie"],
        &["target_function", "main", "sleep", "testprog.c"],
    );
}

#[test]
fn symbolized_stack_trace_contains_function_no_debug() {
    run_symbol_test(
        &["-O0"],
        &["target_function", "main", "sleep"],
    );
}

#[test]
fn symbolized_stack_trace_contains_function_g1() {
    run_symbol_test(
        &["-g1", "-O0"],
        &["target_function", "main", "sleep", "testprog.c"],
    );
}

#[test]
fn symbolized_stack_trace_contains_function_O2() {
    run_symbol_test(
        &["-g", "-O2"],
        &["target_function", "main", "sleep", "testprog.c"],
    );
}
