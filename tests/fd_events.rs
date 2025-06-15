use fuzmon::test_utils::wait_until_file_appears;
use fuzmon::utils::current_date_string;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use std::{thread, time::Duration};
use tempfile::tempdir;
use zstd::stream;

#[test]
fn detect_fd_open_close() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("testfile");
    let script = dir.path().join("script.py");
    fs::write(
        &script,
        r#"import sys
sys.stdin.readline()
f=open("testfile", 'w')
sys.stdin.readline()
f.close()
sys.stdin.readline()
"#,
    )
    .expect("write script");

    let mut child = Command::new("python3")
        .arg(&script)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn python");

    let pid = child.id();
    let mut child_in = child.stdin.take().expect("stdin");

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
        .expect("run fuzmon");

    let date = current_date_string();
    let base_dir = logdir.path().join(&date);
    let plain = base_dir.join(format!("{}.jsonl", pid));
    let zst = base_dir.join(format!("{}.jsonl.zst", pid));
    wait_until_file_appears(&logdir, pid);

    child_in.write_all(b"\n").unwrap();
    child_in.flush().unwrap();

    for _ in 0..50 {
        let path = if plain.exists() { &plain } else { &zst };
        if path.exists() {
            let content = if path.extension().and_then(|e| e.to_str()) == Some("zst") {
                let data = fs::read(path).unwrap();
                match stream::decode_all(&*data) {
                    Ok(d) => String::from_utf8_lossy(&d).into_owned(),
                    Err(_) => String::new(),
                }
            } else {
                fs::read_to_string(path).unwrap_or_default()
            };
            if content.contains("\"event\":\"open\"")
                && content.contains(file_path.to_str().unwrap())
            {
                break;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }

    child_in.write_all(b"\n").unwrap();
    child_in.flush().unwrap();

    for _ in 0..50 {
        let path = if plain.exists() { &plain } else { &zst };
        if path.exists() {
            let content = if path.extension().and_then(|e| e.to_str()) == Some("zst") {
                let data = fs::read(path).unwrap();
                match stream::decode_all(&*data) {
                    Ok(d) => String::from_utf8_lossy(&d).into_owned(),
                    Err(_) => String::new(),
                }
            } else {
                fs::read_to_string(path).unwrap_or_default()
            };
            if content.contains("\"event\":\"close\"") {
                break;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }

    drop(child_in);

    let _ = child.wait();
    fuzmon::test_utils::kill_with_sigint_and_wait(&mut mon);

    let path = if plain.exists() { &plain } else { &zst };
    let log_content = if path.extension().and_then(|e| e.to_str()) == Some("zst") {
        let data = fs::read(path).unwrap();
        match stream::decode_all(&*data) {
            Ok(d) => String::from_utf8_lossy(&d).into_owned(),
            Err(_) => String::new(),
        }
    } else {
        fs::read_to_string(path).unwrap_or_default()
    };
    let logfile = file_path.to_str().unwrap();
    assert!(
        log_content.contains("\"event\":\"open\""),
        "{}",
        log_content
    );
    assert!(log_content.contains("testfile"), "{}", log_content);
    assert!(
        log_content.contains("\"event\":\"close\""),
        "{}",
        log_content
    );
}
