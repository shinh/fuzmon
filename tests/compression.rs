use std::process::{Command, Stdio};
use std::{thread, time::Duration};
use tempfile::{tempdir, NamedTempFile};
use std::fs;

mod common;

#[test]
fn log_files_are_compressed_when_enabled() {
    let logdir = tempdir().expect("logdir");
    let cfg_file = NamedTempFile::new().expect("cfg");
    fs::write(
        cfg_file.path(),
        format!("[output]\npath='{}'\ncompress=true", logdir.path().display()),
    )
    .expect("write cfg");

    let mut child = Command::new("sleep")
        .arg("2")
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn sleep");

    thread::sleep(Duration::from_millis(200));
    let pid = child.id();

    common::run_fuzmon_and_check(
        &[
            "-p",
            &pid.to_string(),
            "-o",
            logdir.path().to_str().unwrap(),
            "-c",
            cfg_file.path().to_str().unwrap(),
        ],
        &["sleep"],
    );

    let _ = child.kill();
    let _ = child.wait();

    // check file extension
    let files: Vec<_> = fs::read_dir(logdir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert!(files.iter().any(|p| p.extension().map_or(false, |e| e == "zst")));
}

