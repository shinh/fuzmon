use fuzmon::test_utils::run_fuzmon;
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

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

    let pid = child.id();

    let logdir = tempdir().expect("logdir");
    let log = run_fuzmon(env!("CARGO_BIN_EXE_fuzmon"), pid, &logdir);

    child_in.write_all(b"\n").unwrap();
    drop(child_in);
    let _ = child.wait();
    let line = log.lines().next().expect("line");
    let entry: Value = serde_json::from_str(line).expect("json");
    let threads = entry
        .get("threads")
        .and_then(|v| v.as_array())
        .expect("array");
    assert!(threads.len() >= 2, "len {}", threads.len());
}
