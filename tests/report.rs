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

    let out = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args(["report", log_path.to_str().unwrap()])
        .output()
        .expect("run report");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Total runtime: 10"), "{}", stdout);
    assert!(stdout.contains("Total CPU time"), "{}", stdout);
    assert!(stdout.contains("2000"), "{}", stdout);
    assert!(stdout.contains("REPORT_VAR"), "{}", stdout);
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

    let out = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .args([
            "report",
            dir.path().to_str().unwrap(),
            "-c",
            cfg.path().to_str().unwrap(),
        ])
        .output()
        .expect("run report dir");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let pos1 = stdout.find("1111").expect("1111");
    let pos2 = stdout.find("2222").expect("2222");
    assert!(pos1 < pos2, "order: {}", stdout);
}
