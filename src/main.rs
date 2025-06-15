use std::{thread::sleep, time::Duration, collections::HashMap};
use std::fs::{self, OpenOptions};
use std::io::Write;
use serde::Serialize;
use regex::Regex;
use chrono::Utc;
use rmp_serde::encode::write_named;

mod config;
mod procinfo;
mod stacktrace;

use crate::config::{parse_args, load_config, merge_config, uid_from_name, CmdArgs};
use crate::procinfo::{
    read_pids, pid_uid, get_proc_usage, ProcState, should_suppress, process_name,
    proc_cpu_time_sec, proc_cpu_jiffies, vsz_kb, swap_kb, detect_fd_events,
};
use crate::stacktrace::{capture_stack_trace, capture_python_stack_trace};

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

#[derive(Serialize)]
struct FdLogEvent {
    timestamp: String,
    pid: u32,
    fd: i32,
    event: String,
    path: String,
}

fn main() {
    let args = parse_args();
    run(args);
}

fn run(args: CmdArgs) {
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
    let compress = config.output.compress.unwrap_or(false);
    let verbose = args.verbose;

    let output_dir = config.output.path.as_deref();
    if let Some(dir) = output_dir {
        let _ = fs::create_dir_all(dir);
    }

    let target_pid = args.pid.map(|p| p as u32);
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
        monitor_iteration(
            &mut states,
            target_pid,
            target_uid,
            &ignore_patterns,
            cpu_jiffies_threshold,
            output_dir,
            use_msgpack,
            compress,
            verbose,
        );
        sleep(sleep_dur);
    }
}

fn monitor_iteration(
    states: &mut HashMap<u32, ProcState>,
    target_pid: Option<u32>,
    target_uid: Option<u32>,
    ignore_patterns: &[Regex],
    cpu_jiffies_threshold: u64,
    output_dir: Option<&str>,
    use_msgpack: bool,
    compress: bool,
    verbose: bool,
) {
    let mut pids = if let Some(pid) = target_pid {
        vec![pid]
    } else {
        read_pids()
    };
    if target_pid.is_none() {
        if let Some(uid) = target_uid {
            pids.retain(|p| pid_uid(*p) == Some(uid));
        }
    }
    if verbose {
        println!("Found {} PIDs", pids.len());
    }
    for pid in &pids {
        if should_skip_pid(*pid, target_pid, ignore_patterns, cpu_jiffies_threshold) {
            continue;
        }
        let state = states.entry(*pid).or_default();
        let fd_events = detect_fd_events(*pid, state);
        if let Some(dir) = output_dir {
            for ev in &fd_events {
                if let Some(old_path) = &ev.old_path {
                    let entry = FdLogEvent {
                        timestamp: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                        pid: *pid,
                        fd: ev.fd,
                        event: "close".into(),
                        path: old_path.clone(),
                    };
                    if verbose {
                        if let Ok(line) = serde_json::to_string(&entry) {
                            println!("{}", line);
                        }
                    }
                    write_fd_event(dir, &entry, use_msgpack);
                }
                if let Some(new_path) = &ev.new_path {
                    let entry = FdLogEvent {
                        timestamp: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                        pid: *pid,
                        fd: ev.fd,
                        event: "open".into(),
                        path: new_path.clone(),
                    };
                    if verbose {
                        if let Ok(line) = serde_json::to_string(&entry) {
                            println!("{}", line);
                        }
                    }
                    write_fd_event(dir, &entry, use_msgpack);
                }
            }
        }
        if let Some((cpu, rss)) = get_proc_usage(*pid, state) {
            if verbose && !should_suppress(cpu, rss) {
                println!("PID {:>5}: {:>5.1}% CPU, {:>8} KB RSS", pid, cpu, rss);
            }

            if let Some(dir) = output_dir {
                let entry = build_log_entry(*pid, cpu, rss);
                if verbose {
                    if let Ok(line) = serde_json::to_string(&entry) {
                        println!("{}", line);
                    }
                }
                write_log(dir, &entry, use_msgpack, compress);
            }
        }
    }
    states.retain(|pid, _| pids.contains(pid));
}

fn should_skip_pid(
    pid: u32,
    target_pid: Option<u32>,
    ignore_patterns: &[Regex],
    cpu_jiffies_threshold: u64,
) -> bool {
    if target_pid.is_none() {
        if let Some(name) = process_name(pid) {
            if ignore_patterns.iter().any(|re| re.is_match(&name)) {
                return true;
            }
        }
        if proc_cpu_jiffies(pid).unwrap_or(0) < cpu_jiffies_threshold {
            return true;
        }
    }
    false
}

fn build_log_entry(pid: u32, cpu: f32, rss: u64) -> LogEntry {
    let mut entry = LogEntry {
        timestamp: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        pid,
        process_name: process_name(pid).unwrap_or_else(|| "?".into()),
        cpu_time_sec: proc_cpu_time_sec(pid).unwrap_or(0.0),
        memory: MemoryInfo {
            rss_kb: rss,
            vsz_kb: vsz_kb(pid).unwrap_or(0),
            swap_kb: swap_kb(pid).unwrap_or(0),
        },
        stacktrace: None,
    };
    if cpu < 1.0 {
        let name = &entry.process_name;
        if name.starts_with("python") {
            match capture_python_stack_trace(pid as i32) {
                Ok(t) => entry.stacktrace = Some(t),
                Err(_) => {
                    if let Ok(t) = capture_stack_trace(pid as i32) {
                        entry.stacktrace = Some(t);
                    }
                }
            }
        } else if let Ok(trace) = capture_stack_trace(pid as i32) {
            entry.stacktrace = Some(trace);
        }
    }
    entry
}

fn write_log(dir: &str, entry: &LogEntry, use_msgpack: bool, compress: bool) {
    let base = format!("{}/{}.log", dir.trim_end_matches('/'), entry.pid);
    let path = if compress { format!("{}.zst", base) } else { base };
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) {
        if compress {
            if let Ok(mut enc) = zstd::Encoder::new(file, 0) {
                if use_msgpack {
                    let _ = write_named(&mut enc, entry);
                } else {
                    let _ = serde_json::to_writer(&mut enc, entry);
                    let _ = enc.write_all(b"\n");
                }
                let _ = enc.finish();
            }
        } else {
            let mut file = file;
            if use_msgpack {
                let _ = write_named(&mut file, entry);
            } else {
                let _ = serde_json::to_writer(&mut file, entry);
                let _ = file.write_all(b"\n");
            }
        }
    }
}

fn write_fd_event(dir: &str, entry: &FdLogEvent, use_msgpack: bool) {
    let path = format!("{}/{}.log", dir.trim_end_matches('/'), entry.pid);
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        if use_msgpack {
            let _ = write_named(&mut file, entry);
        } else {
            let _ = serde_json::to_writer(&mut file, entry);
            let _ = file.write_all(b"\n");
        }
    }
}
