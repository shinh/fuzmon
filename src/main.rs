use std::{thread::sleep, time::Duration, collections::HashMap};
use std::fs::{self, OpenOptions};
use std::io::Write;
use serde::Serialize;
use regex::Regex;
use chrono::{Utc, SecondsFormat};
use rmp_serde::encode::write_named;

mod config;
mod procinfo;
mod stacktrace;

use crate::config::{parse_args, load_config, merge_config, uid_from_name};
use crate::procinfo::{
    read_pids, pid_uid, get_proc_usage, ProcState, should_suppress, process_name,
    proc_cpu_time_sec, proc_cpu_jiffies, vsz_kb, swap_kb,
};
use crate::stacktrace::{attach_and_trace, capture_stack_trace, capture_python_stack_trace};

#[derive(Serialize)]
struct MemoryInfo {
    rss_kb: u64,
    vsz_kb: u64,
    swap_kb: u64,
}

#[derive(Serialize)]
struct LogEntry {
    timestamp: String,
    pid: u32,
    process_name: String,
    cpu_time_sec: f64,
    memory: MemoryInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    stacktrace: Option<Vec<String>>,
}

fn main() {
    let args = parse_args();

    let config = args
        .config
        .as_deref()
        .and_then(load_config)
        .unwrap_or_default();
    let config = merge_config(config, &args);

    let ignore_patterns: Vec<Regex> = config
        .filter
        .ignore_process_name
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| Regex::new(&p).ok())
        .collect();

    let use_msgpack = matches!(config.output.format.as_deref(), Some("msgpack"));

    let output_dir = config.output.path.as_deref();
    if let Some(dir) = output_dir {
        let _ = fs::create_dir_all(dir);
    }

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

    let cpu_jiffies_threshold = config.monitor.cpu_time_jiffies_threshold.unwrap_or(1);

    let mut states: HashMap<u32, ProcState> = HashMap::new();
    loop {
        let mut pids = read_pids();
        if let Some(uid) = target_uid {
            pids.retain(|p| pid_uid(*p) == Some(uid));
        }
        println!("Found {} PIDs", pids.len());
        for pid in &pids {
            if let Some(name) = process_name(*pid) {
                if ignore_patterns.iter().any(|re| re.is_match(&name)) {
                    continue;
                }
            }
            if proc_cpu_jiffies(*pid).unwrap_or(0) < cpu_jiffies_threshold {
                continue;
            }
            let state = states.entry(*pid).or_default();
            if let Some((cpu, rss)) = get_proc_usage(*pid, state) {
                if !should_suppress(cpu, rss) {
                    println!("PID {:>5}: {:>5.1}% CPU, {:>8} KB RSS", pid, cpu, rss);
                }

                if let Some(dir) = output_dir {
                    let ts = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
                    let mut entry = LogEntry {
                        timestamp: ts,
                        pid: *pid,
                        process_name: process_name(*pid).unwrap_or_else(|| "?".into()),
                        cpu_time_sec: proc_cpu_time_sec(*pid).unwrap_or(0.0),
                        memory: MemoryInfo {
                            rss_kb: rss,
                            vsz_kb: vsz_kb(*pid).unwrap_or(0),
                            swap_kb: swap_kb(*pid).unwrap_or(0),
                        },
                        stacktrace: None,
                    };
                    if cpu < 1.0 {
                        let name = &entry.process_name;
                        if name.starts_with("python") {
                            match capture_python_stack_trace(*pid as i32) {
                                Ok(t) => entry.stacktrace = Some(t),
                                Err(_) => {
                                    if let Ok(t) = capture_stack_trace(*pid as i32) {
                                        entry.stacktrace = Some(t);
                                    }
                                }
                            }
                        } else if let Ok(trace) = capture_stack_trace(*pid as i32) {
                            entry.stacktrace = Some(trace);
                        }
                    }
                    let path = format!("{}/{}.log", dir.trim_end_matches('/'), pid);
                    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
                        if use_msgpack {
                            let _ = write_named(&mut file, &entry);
                        } else {
                            let _ = serde_json::to_writer(&mut file, &entry);
                            let _ = file.write_all(b"\n");
                        }
                    }
                }
            }
        }
        states.retain(|pid, _| pids.contains(pid));
        println!();
        sleep(sleep_dur);
    }
}
