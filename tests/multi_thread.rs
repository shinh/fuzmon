use serde_json::Value;
use std::fs;
use std::process::{Command, Stdio};
use std::{thread, time::Duration};
use tempfile::tempdir;

#[test]
fn multi_thread_stacktrace_has_multiple_entries() {
    let dir = tempdir().expect("tempdir");
    let src = dir.path().join("prog.c");
    fs::write(
        &src,
        r#"
#include <pthread.h>
#include <unistd.h>
void* worker(void* arg) {
    while (1) { sleep(1); }
    return NULL;
}
int main() {
    pthread_t t;
    pthread_create(&t, NULL, worker, NULL);
    while (1) { sleep(1); }
    return 0;
}
"#,
    )
    .expect("write src");
    let exe = dir.path().join("prog");
    assert!(
        Command::new("gcc")
            .args([
                "-g",
                "-pthread",
                src.to_str().unwrap(),
                "-o",
                exe.to_str().unwrap()
            ])
            .status()
            .expect("compile")
            .success()
    );

    let mut child = Command::new(&exe)
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn");

    thread::sleep(Duration::from_millis(500));
    let pid = child.id();

    let logdir = tempdir().expect("logdir");
    let mut mon = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args([
            "run",
            "-p",
            &pid.to_string(),
            "-o",
            logdir.path().to_str().unwrap(),
        ])
        .stdout(Stdio::null())
        .spawn()
        .expect("run");

    thread::sleep(Duration::from_millis(800));
    let _ = mon.kill();
    let _ = mon.wait();

    let _ = child.kill();
    let _ = child.wait();

    let plain = logdir.path().join(format!("{}.jsonl", pid));
    let path = if plain.exists() {
        plain
    } else {
        logdir.path().join(format!("{}.jsonl.zst", pid))
    };
    let log = if path.extension().and_then(|e| e.to_str()) == Some("zst") {
        let data = fs::read(&path).expect("read log");
        String::from_utf8_lossy(&zstd::stream::decode_all(&*data).expect("decompress")).into_owned()
    } else {
        fs::read_to_string(&path).expect("read log")
    };
    let line = log.lines().next().expect("line");
    let entry: Value = serde_json::from_str(line).expect("json");
    let stack = entry
        .get("stacktrace")
        .and_then(|v| v.as_array())
        .expect("array");
    assert!(stack.len() >= 2, "len {}", stack.len());
}
