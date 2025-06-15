use std::fs;
use std::process::{Command, Stdio};
use tempfile::tempdir;

use fuzmon::test_utils::run_fuzmon_and_check;

fn run_symbol_test(flags: &[&str], expected: &[&str]) {
    let dir = tempdir().expect("tempdir");
    let src_path = dir.path().join("testprog.c");
    fs::write(
        &src_path,
        r#"
#include <stdio.h>
#include <unistd.h>

__attribute__((noinline))
static void block_read() {
    char buf;
    int r = read(0, &buf, 1);
    if (r < 0) {
       fprintf(stderr, "Read error: %d\\n", r);
    }
}

__attribute__((noinline))
void target_function() {
    while (1) {
        block_read();
    }
}

__attribute__((noinline))
int main() {
    target_function();
    return 0;
}
"#,
    )
    .expect("write src");

    let exe_path = dir.path().join("testprog");
    let mut gcc_args: Vec<&str> = flags.to_vec();
    gcc_args.push("-fno-optimize-sibling-calls");
    gcc_args.push("-fno-omit-frame-pointer");
    gcc_args.push(src_path.to_str().unwrap());
    gcc_args.push("-o");
    gcc_args.push(exe_path.to_str().unwrap());

    let status = Command::new("gcc")
        .args(&gcc_args)
        .status()
        .expect("compile test program");
    assert!(status.success());

    let mut child = Command::new(&exe_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn test program");

    let pid = child.id();
    let logdir = tempdir().expect("logdir");
    run_fuzmon_and_check(env!("CARGO_BIN_EXE_fuzmon"), pid, &logdir, expected);

    unsafe {
        let _ = nix::libc::kill(child.id() as i32, nix::libc::SIGINT);
    }
    let _ = child.wait();
}

#[test]
fn symbolized_stack_trace_contains_function() {
    run_symbol_test(&["-g", "-O0"], &["target_function", "main", "testprog.c"]);
}

#[test]
fn symbolized_stack_trace_contains_function_no_pie() {
    run_symbol_test(
        &["-g", "-O0", "-no-pie"],
        &["target_function", "main", "testprog.c"],
    );
}

#[test]
fn symbolized_stack_trace_contains_function_no_debug() {
    run_symbol_test(&["-O0"], &["target_function", "main"]);
}

#[test]
fn symbolized_stack_trace_contains_function_g1() {
    run_symbol_test(&["-g1", "-O0"], &["target_function", "main", "testprog.c"]);
}

#[test]
fn symbolized_stack_trace_contains_function_o2() {
    run_symbol_test(&["-g", "-O2"], &["target_function", "main", "testprog.c"]);
}
