mod config;
mod dump;
mod log;
mod procinfo;
mod report;
mod run;
mod stacktrace;

use crate::config::{Cli, Commands, parse_cli};
use clap::CommandFactory;

fn main() {
    env_logger::init();
    let cli = parse_cli();
    if let Some(cmd) = cli.command {
        match cmd {
            Commands::Run(args) => run::run(args),
            Commands::Dump(args) => dump::dump(&args.path),
            Commands::Report(args) => report::report(&args),
        }
    } else {
        Cli::command().print_help().unwrap();
        println!();
    }
}
