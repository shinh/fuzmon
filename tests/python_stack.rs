use std::process::{Command, Stdio};
use std::{thread, time::Duration};
use std::fs;
use tempfile::tempdir;

#[test]
fn python_stack_trace_contains_functions() {
    let dir = tempdir().expect("tempdir");
    let script = dir.path().join("test.py");
    fs::write(&script, r#"
import time

def foo():
    bar()

def bar():
    while True:
        time.sleep(1)

if __name__ == '__main__':
    foo()
"#).expect("write script");

    let mut child = Command::new("python3")
        .arg(&script)
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn python");

    thread::sleep(Duration::from_millis(500));
    let pid = child.id();

    let logdir = tempdir().expect("logdir");
    let mut mon = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args(["run", "-p", &pid.to_string(), "-o", logdir.path().to_str().unwrap()])
        .stdout(Stdio::null())
        .spawn()
        .expect("run fuzmon");

    thread::sleep(Duration::from_millis(800));
    let _ = mon.kill();
    let _ = mon.wait();

    let _ = child.kill();
    let _ = child.wait();

    let log_path = logdir.path().join(format!("{}.log", pid));
    let log = fs::read_to_string(log_path).expect("read log");
    assert!(log.contains("foo"), "{}", log);
    assert!(log.contains("bar"), "{}", log);
    assert!(log.contains("test.py"), "{}", log);
}
