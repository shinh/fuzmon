use fuzmon::test_utils::run_fuzmon;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use tempfile::tempdir;

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
    print("ready", flush=True)
    sys.stdin.readline()

if __name__ == '__main__':
    foo()
"#,
    )
    .expect("write script");

    let mut child = Command::new("python3")
        .arg(&script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn python");
    let mut child_in = child.stdin.take().expect("child stdin");
    let mut child_out = BufReader::new(child.stdout.take().expect("child stdout"));

    let mut line = String::new();
    child_out.read_line(&mut line).expect("read line");
    assert_eq!(line.trim(), "ready");

    let pid = child.id();

    let logdir = tempdir().expect("logdir");
    let log = run_fuzmon(env!("CARGO_BIN_EXE_fuzmon"), pid, &logdir);

    child_in.write_all(b"\n").unwrap();
    drop(child_in);
    let _ = child.wait();
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
