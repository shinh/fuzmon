use std::process::Command;

#[test]
fn dump_outputs_todo() {
    let out = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .arg("dump")
        .output()
        .expect("run fuzmon dump");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("TODO!"));
}

#[test]
fn help_subcommand_shows_usage() {
    let out = Command::new(env!("CARGO_BIN_EXE_fuzmon"))
        .arg("help")
        .output()
        .expect("run fuzmon help");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("fuzmon"));
    assert!(stdout.contains("run"));
}
