use std::process::Command;

pub fn fuzmon_cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_fuzmon"))
}
