use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use std::{thread, time::Duration};
use tempfile::tempdir;
mod common;

#[test]
fn python_stack_trace_contains_functions() {
    let dir = tempdir().expect("tempdir");
    let script = dir.path().join("test.py");
    fs::write(
        &script,
        r#"
import sys

def foo():
    bar()

def bar():
    sys.stdin.readline()

if __name__ == '__main__':
    foo()
"#,
    )
    .expect("write script");

    let mut child = Command::new("python3")
        .arg(&script)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn python");
    let mut child_in = child.stdin.take().expect("child stdin");

    thread::sleep(Duration::from_millis(500));
    let pid = child.id();

    let logdir = tempdir().expect("logdir");
    let cfg_file = common::create_config(0.0);

    let mut mon = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args([
            "run",
            "-p",
            &pid.to_string(),
            "-o",
            logdir.path().to_str().unwrap(),
            "-c",
            cfg_file.path().to_str().unwrap(),
        ])
        .stdout(Stdio::null())
        .spawn()
        .expect("run fuzmon");

    common::wait_until_file_appears(&logdir, pid);
    unsafe {
        let _ = nix::libc::kill(mon.id() as i32, nix::libc::SIGINT);
    }
    let _ = mon.wait();

    child_in.write_all(b"\n").unwrap();
    drop(child_in);
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
    assert!(log.contains("foo"), "{}", log);
    assert!(log.contains("bar"), "{}", log);
    assert!(log.contains("test.py"), "{}", log);
    let first = log.lines().next().expect("line");
    let entry: serde_json::Value = serde_json::from_str(first).expect("json");
    let threads = entry
        .get("threads")
        .and_then(|v| v.as_array())
        .expect("threads");
    let mut has_c = false;
    let mut has_py = false;
    for t in threads {
        if t.get("stacktrace").is_some() {
            has_c = true;
        }
        if t.get("python_stacktrace").is_some() {
            has_py = true;
        }
    }
    assert!(has_c, "no c stacktrace: {}", first);
    assert!(has_py, "no python stacktrace: {}", first);
}
