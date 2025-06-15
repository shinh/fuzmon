use std::process::{Command, Stdio};
use std::{thread, time::Duration};
use std::fs;

pub fn run_fuzmon_and_check(args: &[&str], expected: &[&str]) {
    let log_dir = args.iter()
        .position(|&a| a == "-o")
        .and_then(|i| args.get(i + 1))
        .expect("args must contain -o <dir>");

    let mut mon = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args(args)
        .stdout(Stdio::null())
        .spawn()
        .expect("run fuzmon");

    thread::sleep(Duration::from_millis(800));
    let _ = mon.kill();
    let _ = mon.wait();

    let mut log_content = String::new();
    for entry in fs::read_dir(log_dir).expect("read_dir") {
        let path = entry.expect("entry").path();
        if let Ok(s) = fs::read_to_string(&path) {
            log_content.push_str(&s);
        }
    }

    for e in expected {
        assert!(log_content.contains(e), "expected '{}' in {}", e, log_content);
    }
}

