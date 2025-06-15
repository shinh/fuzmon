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

    let output = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args(["-p", &pid.to_string()])
        .output()
        .expect("run fuzmon");

    let _ = child.kill();
    let _ = child.wait();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("foo"), "{}", stdout);
    assert!(stdout.contains("bar"), "{}", stdout);
    assert!(stdout.contains("test.py"), "{}", stdout);
}
