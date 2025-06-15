use std::fs;
use std::process::{Command, Stdio};
use std::{thread, time::Duration};
use tempfile::TempDir;
use zstd::stream;

#[allow(dead_code)]
pub fn wait_until_file_appears(logdir: &TempDir, pid: u32) {
    let plain = logdir.path().join(format!("{pid}.jsonl"));
    let zst = logdir.path().join(format!("{pid}.jsonl.zst"));
    for _ in 0..80 {
        if plain.exists() || zst.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[allow(dead_code)]
pub fn run_fuzmon_and_check(pid: u32, log_dir: &TempDir, expected: &[&str]) {
    let pid_s = pid.to_string();

    let mut mon = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args(["run", "-p", &pid_s, "-o", log_dir.path().to_str().unwrap()])
        .stdout(Stdio::null())
        .spawn()
        .expect("run fuzmon");

    wait_until_file_appears(log_dir, pid);
    unsafe {
        let _ = nix::libc::kill(mon.id() as i32, nix::libc::SIGINT);
    }
    let _ = mon.wait();

    let mut log_content = String::new();
    for entry in fs::read_dir(log_dir.path()).expect("read_dir") {
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
        assert!(
            log_content.contains(e),
            "expected '{}' in {}",
            e,
            log_content
        );
    }
}
