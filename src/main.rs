use std::{thread::sleep, time::Duration, collections::HashMap};

mod config;
mod procinfo;
mod stacktrace;

use crate::config::{parse_args, load_config, merge_config, uid_from_name};
use crate::procinfo::{read_pids, pid_uid, get_proc_usage, ProcState, should_suppress};
use crate::stacktrace::attach_and_trace;

fn main() {
    let args = parse_args();

    let config = args
        .config
        .as_deref()
        .and_then(load_config)
        .unwrap_or_default();
    let config = merge_config(config, &args);

    if let Some(pid) = args.pid {
        if let Err(e) = attach_and_trace(pid) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    let target_uid = config
        .filter
        .target_user
        .as_deref()
        .and_then(uid_from_name);

    let interval = config.monitor.interval_sec.unwrap_or(0);
    let sleep_dur = if interval == 0 {
        Duration::from_millis(200)
    } else {
        Duration::from_secs(interval)
    };

    let mut states: HashMap<u32, ProcState> = HashMap::new();
    loop {
        let mut pids = read_pids();
        if let Some(uid) = target_uid {
            pids.retain(|p| pid_uid(*p) == Some(uid));
        }
        println!("Found {} PIDs", pids.len());
        for pid in &pids {
            let state = states.entry(*pid).or_default();
            if let Some((cpu, rss)) = get_proc_usage(*pid, state) {
                if !should_suppress(cpu, rss) {
                    println!("PID {:>5}: {:>5.1}% CPU, {:>8} KB RSS", pid, cpu, rss);
                }
            }
        }
        states.retain(|pid, _| pids.contains(pid));
        println!();
        sleep(sleep_dur);
    }
}
