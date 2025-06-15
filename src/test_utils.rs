use crate::utils::current_date_string;
use std::fs;
use std::process::{Child, Command, Stdio};
use std::{thread, time::Duration};
use tempfile::{NamedTempFile, TempDir};
use zstd::stream;

fn build_fuzmon_command(
    bin: &str,
    pid: u32,
    log_dir: &TempDir,
    cfg_file: &NamedTempFile,
) -> Command {
    let pid_s = pid.to_string();
    let mut cmd = Command::new(bin);
    cmd.args([
        "run",
        "-p",
        &pid_s,
        "-o",
        log_dir.path().to_str().unwrap(),
        "-c",
        cfg_file.path().to_str().unwrap(),
    ]);
    cmd
}

pub fn wait_until_file_appears(logdir: &TempDir, pid: u32) {
    let date = current_date_string();
    let dir = logdir.path().join(&date);
    let plain = dir.join(format!("{pid}.jsonl"));
    let zst = dir.join(format!("{pid}.jsonl.zst"));
    for _ in 0..80 {
        if plain.exists() || zst.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn kill_with_sigint_and_wait(child: &mut Child) {
    unsafe {
        let _ = nix::libc::kill(child.id() as i32, nix::libc::SIGINT);
    }
    let _ = child.wait();
}

pub fn create_config(threshold: f64) -> NamedTempFile {
    let cfg_file = NamedTempFile::new().expect("cfg");
    fs::write(
        cfg_file.path(),
        format!(
            "[monitor]\nstacktrace_cpu_time_percent_threshold = {}",
            threshold
        ),
    )
    .expect("write cfg");
    cfg_file
}

pub fn run_fuzmon(bin: &str, pid: u32, log_dir: &TempDir) -> String {
    let cfg_file = create_config(0.0);

    let mut mon = build_fuzmon_command(bin, pid, log_dir, &cfg_file)
        .stdout(Stdio::null())
        .spawn()
        .expect("run fuzmon");

    wait_until_file_appears(log_dir, pid);
    kill_with_sigint_and_wait(&mut mon);

    collect_log_content(log_dir)
}

pub fn collect_log_content(log_dir: &TempDir) -> String {
    let mut log_content = String::new();
    for entry in fs::read_dir(log_dir.path()).expect("read_dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            for sub in fs::read_dir(&path).expect("read_dir") {
                let sub_path = sub.expect("subentry").path();
                append_file(&sub_path, &mut log_content);
            }
        } else {
            append_file(&path, &mut log_content);
        }
    }
    log_content
}

fn append_file(path: &std::path::Path, log_content: &mut String) {
    if let Some(ext) = path.extension() {
        if ext == "zst" {
            if let Ok(data) = fs::read(path) {
                if let Ok(decoded) = stream::decode_all(&*data) {
                    log_content.push_str(&String::from_utf8_lossy(&decoded));
                    return;
                }
            }
        }
    }
    if let Ok(s) = fs::read_to_string(path) {
        log_content.push_str(&s);
    }
}

pub fn run_fuzmon_output(
    bin: &str,
    pid: u32,
    log_dir: &TempDir,
    cfg_file: &NamedTempFile,
) -> std::process::Output {
    build_fuzmon_command(bin, pid, log_dir, cfg_file)
        .output()
        .expect("run fuzmon")
}

pub fn run_fuzmon_and_check(bin: &str, pid: u32, log_dir: &TempDir, expected: &[&str]) {
    let log_content = run_fuzmon(bin, pid, log_dir);

    for e in expected {
        assert!(
            log_content.contains(e),
            "expected '{}' in {}",
            e,
            log_content
        );
    }
}
