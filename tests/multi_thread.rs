use serde_json::Value;
use std::fs;
use std::process::{Command, Stdio};
use std::io::Write;
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

static pthread_mutex_t mutex = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t cond = PTHREAD_COND_INITIALIZER;
static int done = 0;

void* worker(void* arg) {
    (void)arg;
    pthread_mutex_lock(&mutex);
    while (!done) {
        pthread_cond_wait(&cond, &mutex);
    }
    pthread_mutex_unlock(&mutex);
    return NULL;
}

int main() {
    pthread_t t;
    pthread_create(&t, NULL, worker, NULL);

    char buf;
    read(0, &buf, 1);

    pthread_mutex_lock(&mutex);
    done = 1;
    pthread_cond_signal(&cond);
    pthread_mutex_unlock(&mutex);

    pthread_join(t, NULL);
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
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn");
    let mut child_in = child.stdin.take().expect("child stdin");

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
    let line = log.lines().next().expect("line");
    let entry: Value = serde_json::from_str(line).expect("json");
    let stack = entry
        .get("stacktrace")
        .and_then(|v| v.as_array())
        .expect("array");
    assert!(stack.len() >= 2, "len {}", stack.len());
}
