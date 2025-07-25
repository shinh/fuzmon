use std::fs;
use std::process::{Command, Stdio};
use tempfile::{NamedTempFile, tempdir};

#[test]
fn html_report_has_stats() {
    let dir = tempdir().expect("dir");
    let mut child = Command::new("sleep")
        .arg("5")
        .env("REPORT_VAR", "123")
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn");
    let pid = child.id();
    let log_path = dir.path().join(format!("{pid}.jsonl"));
    fs::write(
        &log_path,
        format!(
            "{{\"timestamp\":\"2025-06-14T00:00:00Z\",\"pid\":{pid},\"process_name\":\"sleep\",\"cpu_time_percent\":50.0,\"memory\":{{\"rss_kb\":1000,\"vsz_kb\":0,\"swap_kb\":0}},\"cmdline\":\"sleep 5\",\"env\":\"REPORT_VAR=123\"}}\n{{\"timestamp\":\"2025-06-14T00:00:10Z\",\"pid\":{pid},\"process_name\":\"sleep\",\"cpu_time_percent\":0.0,\"memory\":{{\"rss_kb\":2000,\"vsz_kb\":0,\"swap_kb\":0}}}}\n"
        ),
    )
    .unwrap();

    unsafe {
        let _ = nix::libc::kill(child.id() as i32, nix::libc::SIGKILL);
    }
    let _ = child.wait();

    let outdir = tempdir().expect("outdir");
    let out = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args([
            "report",
            log_path.to_str().unwrap(),
            "-o",
            outdir.path().to_str().unwrap(),
        ])
        .output()
        .expect("run report");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(outdir.path().to_str().unwrap()),
        "{}",
        stdout
    );
    let html = fs::read_to_string(outdir.path().join("index.html")).unwrap();
    assert!(html.contains("Total runtime: 10"), "{}", html);
    assert!(html.contains("Total CPU time"), "{}", html);
    assert!(html.contains("Average CPU usage"), "{}", html);
    assert!(html.contains("2000"), "{}", html);
    assert!(html.contains("REPORT_VAR"), "{}", html);
    assert!(html.contains("<img"), "{}", html);
    assert!(outdir.path().join(format!("{pid}_cpu.svg")).exists());
    assert!(outdir.path().join(format!("{pid}_rss.svg")).exists());
}

#[test]
fn html_report_directory() {
    let dir = tempdir().expect("dir");
    let log1 = dir.path().join("1111.jsonl");
    let log2 = dir.path().join("2222.jsonl");
    fs::write(
        &log1,
        "{\"timestamp\":\"2025-06-14T00:00:00Z\",\"pid\":1111,\"process_name\":\"a\",\"cpu_time_percent\":100.0,\"memory\":{\"rss_kb\":1000,\"vsz_kb\":0,\"swap_kb\":0},\"cmdline\":\"a\"}\n{\"timestamp\":\"2025-06-14T00:00:10Z\",\"pid\":1111,\"process_name\":\"a\",\"cpu_time_percent\":0.0,\"memory\":{\"rss_kb\":1500,\"vsz_kb\":0,\"swap_kb\":0}}\n",
    )
    .unwrap();
    fs::write(
        &log2,
        "{\"timestamp\":\"2025-06-14T00:00:00Z\",\"pid\":2222,\"process_name\":\"b\",\"cpu_time_percent\":10.0,\"memory\":{\"rss_kb\":5000,\"vsz_kb\":0,\"swap_kb\":0},\"cmdline\":\"b\"}\n{\"timestamp\":\"2025-06-14T00:00:10Z\",\"pid\":2222,\"process_name\":\"b\",\"cpu_time_percent\":0.0,\"memory\":{\"rss_kb\":6000,\"vsz_kb\":0,\"swap_kb\":0}}\n",
    )
    .unwrap();

    let cfg = NamedTempFile::new().expect("cfg");
    fs::write(cfg.path(), "[report]\ntop_cpu=1\ntop_rss=1\n").unwrap();

    let outdir = tempdir().expect("outdir");
    let out = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args([
            "report",
            dir.path().to_str().unwrap(),
            "-c",
            cfg.path().to_str().unwrap(),
            "-o",
            outdir.path().to_str().unwrap(),
        ])
        .output()
        .expect("run report dir");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(outdir.path().to_str().unwrap()),
        "{}",
        stdout
    );
    let html = fs::read_to_string(outdir.path().join("index.html")).unwrap();
    assert!(html.contains("Start:"), "{}", html);
    assert!(html.contains("End:"), "{}", html);
    assert!(html.contains("<th>Start</th>"), "{}", html);
    assert!(html.contains("<th>End</th>"), "{}", html);
    let pos1 = html.find("1111").expect("1111");
    let pos2 = html.find("2222").expect("2222");
    assert!(pos1 < pos2, "order: {}", html);
    assert!(outdir.path().join("1111.html").exists());
    assert!(outdir.path().join("2222.html").exists());
    assert!(outdir.path().join("1111_cpu.svg").exists());
    assert!(outdir.path().join("1111_rss.svg").exists());
    assert!(outdir.path().join("2222_cpu.svg").exists());
    assert!(outdir.path().join("2222_rss.svg").exists());
    assert!(outdir.path().join("top_cpu.svg").exists());
    assert!(outdir.path().join("top_rss.svg").exists());
    assert!(html.contains("top_cpu.svg"), "{}", html);
    assert!(html.contains("top_rss.svg"), "{}", html);
}

#[test]
fn command_column_collapsed() {
    let dir = tempdir().expect("dir");
    let log = dir.path().join("1234.jsonl");
    fs::write(
        &log,
        "{\"timestamp\":\"2025-06-14T00:00:00Z\",\"pid\":1234,\"process_name\":\"a\",\"cpu_time_percent\":0.0,\"memory\":{\"rss_kb\":1000,\"vsz_kb\":0,\"swap_kb\":0},\"cmdline\":\"a very very long command line that should be collapsed\"}\n{\"timestamp\":\"2025-06-14T00:00:10Z\",\"pid\":1234,\"process_name\":\"a\",\"cpu_time_percent\":0.0,\"memory\":{\"rss_kb\":1000,\"vsz_kb\":0,\"swap_kb\":0}}\n",
    )
    .unwrap();

    let outdir = tempdir().expect("outdir");
    let out = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args([
            "report",
            dir.path().to_str().unwrap(),
            "-o",
            outdir.path().to_str().unwrap(),
        ])
        .output()
        .expect("run report dir");
    assert!(out.status.success());
    let html = fs::read_to_string(outdir.path().join("index.html")).unwrap();
    assert!(html.contains("<details>"), "{}", html);
    assert!(html.contains("border-collapse"), "{}", html);
}

#[test]
fn trace_json_created_with_stacktrace() {
    use fuzmon::test_utils::run_fuzmon;
    use fuzmon::utils::current_date_string;
    use std::io::{BufRead, BufReader, Write};

    let dir = tempdir().expect("dir");
    let script = dir.path().join("test.py");
    fs::write(
        &script,
        r#"import sys
def foo():
    print('ready', flush=True)
    sys.stdin.readline()
foo()
"#,
    )
    .unwrap();

    let mut child = Command::new("python3")
        .arg(&script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn python");
    let mut child_in = child.stdin.take().unwrap();
    let mut child_out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    child_out.read_line(&mut line).unwrap();
    assert_eq!(line.trim(), "ready");

    let pid = child.id();
    let logdir = tempdir().expect("logdir");
    run_fuzmon(env!("CARGO_BIN_EXE_fuzmon"), pid, &logdir);

    child_in.write_all(b"\n").unwrap();
    drop(child_in);
    let _ = child.wait();

    let date = current_date_string();
    let base = logdir.path().join(&date).join(format!("{pid}.jsonl"));
    let log_path = if base.exists() {
        base
    } else {
        base.with_extension("jsonl.zst")
    };

    let outdir = tempdir().expect("outdir");
    let out = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args([
            "report",
            log_path.to_str().unwrap(),
            "-o",
            outdir.path().to_str().unwrap(),
        ])
        .output()
        .expect("run report");
    assert!(out.status.success());
    let trace_path = outdir.path().join(format!("{pid}_trace.json"));
    assert!(trace_path.exists());
    let trace = fs::read_to_string(trace_path).unwrap();
    assert!(trace.contains("traceEvents"), "{}", trace);
    let html = fs::read_to_string(outdir.path().join("index.html")).unwrap();
    assert!(
        html.contains(&format!("<a href=\"{}_trace.json\"", pid)),
        "{}",
        html
    );
}

#[test]
fn no_trace_link_without_stacktrace() {
    let dir = tempdir().expect("dir");
    let pid = 4242;
    let log_path = dir.path().join(format!("{pid}.jsonl"));
    fs::write(
        &log_path,
        format!(
            "{{\"timestamp\":\"2025-06-14T00:00:00Z\",\"pid\":{pid},\"process_name\":\"sleep\",\"cpu_time_percent\":0.0,\"memory\":{{\"rss_kb\":1000,\"vsz_kb\":0,\"swap_kb\":0}}}}\n"
        ),
    )
    .unwrap();

    let outdir = tempdir().expect("outdir");
    let out = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args([
            "report",
            log_path.to_str().unwrap(),
            "-o",
            outdir.path().to_str().unwrap(),
        ])
        .output()
        .expect("run report");
    assert!(out.status.success());
    let trace_path = outdir.path().join(format!("{pid}_trace.json"));
    assert!(!trace_path.exists());
    let html = fs::read_to_string(outdir.path().join("index.html")).unwrap();
    assert!(!html.contains(&format!("{}_trace.json", pid)), "{}", html);
}

#[test]
fn trace_python_stack_on_separate_row() {
    use fuzmon::test_utils::run_fuzmon;
    use fuzmon::utils::current_date_string;
    use std::collections::HashSet;
    use std::io::{BufRead, BufReader, Write};

    let dir = tempdir().expect("dir");
    let script = dir.path().join("test_row.py");
    fs::write(
        &script,
        r#"import sys
def foo():
    print('ready', flush=True)
    sys.stdin.readline()
foo()
"#,
    )
    .unwrap();

    let mut child = Command::new("python3")
        .arg(&script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn python");
    let mut child_in = child.stdin.take().unwrap();
    let mut child_out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    child_out.read_line(&mut line).unwrap();
    assert_eq!(line.trim(), "ready");

    let pid = child.id();
    let logdir = tempdir().expect("logdir");
    run_fuzmon(env!("CARGO_BIN_EXE_fuzmon"), pid, &logdir);

    child_in.write_all(b"\n").unwrap();
    drop(child_in);
    let _ = child.wait();

    let date = current_date_string();
    let base = logdir.path().join(&date).join(format!("{pid}.jsonl"));
    let log_path = if base.exists() {
        base
    } else {
        base.with_extension("jsonl.zst")
    };

    let outdir = tempdir().expect("outdir");
    let out = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args([
            "report",
            log_path.to_str().unwrap(),
            "-o",
            outdir.path().to_str().unwrap(),
        ])
        .output()
        .expect("run report");
    assert!(out.status.success());

    let trace_path = outdir.path().join(format!("{pid}_trace.json"));
    let trace = fs::read_to_string(trace_path).unwrap();
    let obj: serde_json::Value = serde_json::from_str(&trace).unwrap();
    let events = obj
        .get("traceEvents")
        .and_then(|v| v.as_array())
        .expect("events");

    let mut tids: HashSet<u64> = HashSet::new();
    for e in events {
        if let Some(tid) = e.get("tid").and_then(|v| v.as_u64()) {
            tids.insert(tid);
        }
    }

    let mut has_pair = false;
    for tid in &tids {
        if tid % 2 == 0 && tids.contains(&(tid + 1)) {
            has_pair = true;
            break;
        }
    }
    assert!(has_pair, "no separate python row: {:?}", tids);
}
