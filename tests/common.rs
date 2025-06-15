use std::process::{Command, Stdio};
use std::{thread, time::Duration};
use std::fs;
use std::path::PathBuf;
use zstd::stream;

pub fn wait_until_file_appears(path: std::path::PathBuf) {
    let timeout = Duration::from_secs(5);
    let start = std::time::Instant::now();
    while !path.exists() {
        if start.elapsed() > timeout {
            panic!("Timeout waiting for file to appear: {}", path.display());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[allow(dead_code)]
pub fn run_fuzmon_and_check(args: &[&str], expected: &[&str]) {
    let log_dir = args.iter()
        .position(|&a| a == "-o")
        .and_then(|i| args.get(i + 1))
        .expect("args must contain -o <dir>");

    let mut cmd_args = vec!["run"];
    cmd_args.extend_from_slice(args);

    let mut mon = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args(&cmd_args)
        .stdout(Stdio::null())
        .spawn()
        .expect("run fuzmon");

    thread::sleep(Duration::from_millis(800));
    let _ = mon.kill();
    let _ = mon.wait();

    let mut log_content = String::new();
    for entry in fs::read_dir(log_dir).expect("read_dir") {
        let path = entry.expect("entry").path();
        if let Some(ext) = path.extension() {
            if ext == "zst" {
                if let Ok(data) = fs::read(&path) {
                    if let Ok(decoded) = stream::decode_all(&*data) {
                        log_content.push_str(&String::from_utf8_lossy(&decoded));
                        continue;
                    }
                }
            }
        }
        if let Ok(s) = fs::read_to_string(&path) {
            log_content.push_str(&s);
        }
    }

    for e in expected {
        assert!(log_content.contains(e), "expected '{}' in {}", e, log_content);
    }
}

