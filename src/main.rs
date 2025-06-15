use chrono::Utc;
use regex::Regex;
use rmp_serde::decode::{from_read as read_msgpack, Error as MsgpackError};
use rmp_serde::encode::write_named;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::{collections::HashMap, thread::sleep, time::Duration};

mod config;
mod procinfo;
mod stacktrace;

use crate::config::{load_config, merge_config, parse_cli, uid_from_name, Cli, Commands, RunArgs};
use crate::procinfo::{
    detect_fd_events, get_proc_usage, pid_uid, proc_cpu_jiffies, proc_cpu_time_sec, process_name,
    read_pids, should_suppress, swap_kb, vsz_kb, ProcState,
};
use crate::stacktrace::{capture_python_stack_traces, capture_stack_traces};
use clap::CommandFactory;

#[derive(Serialize, Deserialize, Debug)]
struct MemoryInfo {
    rss_kb: u64,
    vsz_kb: u64,
    swap_kb: u64,
}

#[derive(Serialize, Deserialize, Debug)]
struct LogEntry {
    timestamp: String,
    pid: u32,
    process_name: String,
    cpu_time_sec: f64,
    memory: MemoryInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    fd_events: Option<Vec<FdLogEvent>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    stacktrace: Vec<Option<Vec<String>>>,
}

#[derive(Serialize, Deserialize, Debug)]
struct FdLogEvent {
    fd: i32,
    event: String,
    path: String,
}

fn main() {
    let cli = parse_cli();
    if let Some(cmd) = cli.command {
        match cmd {
            Commands::Run(args) => run(args),
            Commands::Dump(args) => dump(&args.path),
        }
    } else {
        Cli::command().print_help().unwrap();
        println!();
    }
}

fn run(args: RunArgs) {
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
    let target_uid = config.filter.target_user.as_deref().and_then(uid_from_name);

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
        let raw_events = detect_fd_events(*pid, state);
        state.pending_fd_events.extend(raw_events);
        if let Some((cpu, rss)) = get_proc_usage(*pid, state) {
            let fd_log_events: Vec<FdLogEvent> = state
                .pending_fd_events
                .drain(..)
                .flat_map(|ev| {
                    let mut events = Vec::new();
                    if let Some(old_path) = ev.old_path {
                        events.push(FdLogEvent {
                            fd: ev.fd,
                            event: "close".into(),
                            path: old_path,
                        });
                    }
                    if let Some(new_path) = ev.new_path {
                        events.push(FdLogEvent {
                            fd: ev.fd,
                            event: "open".into(),
                            path: new_path,
                        });
                    }
                    events
                })
                .collect();
            if verbose && !should_suppress(cpu, rss) {
                println!("PID {:>5}: {:>5.1}% CPU, {:>8} KB RSS", pid, cpu, rss);
            }

            if let Some(dir) = output_dir {
                let entry = build_log_entry(*pid, cpu, rss, fd_log_events);
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

fn build_log_entry(pid: u32, cpu: f32, rss: u64, fd_events: Vec<FdLogEvent>) -> LogEntry {
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
        fd_events: if fd_events.is_empty() {
            None
        } else {
            Some(fd_events)
        },
        stacktrace: Vec::new(),
    };
    if cpu < 1.0 {
        let name = &entry.process_name;
        if name.starts_with("python") {
            match capture_python_stack_traces(pid as i32) {
                Ok(t) => entry.stacktrace = t,
                Err(_) => {
                    entry.stacktrace = capture_stack_traces(pid as i32);
                }
            }
        } else {
            entry.stacktrace = capture_stack_traces(pid as i32);
        }
    }
    entry
}

fn write_log(dir: &str, entry: &LogEntry, use_msgpack: bool, compress: bool) {
    let ext = if use_msgpack { "msgs" } else { "jsonl" };
    let base = format!("{}/{}.{}", dir.trim_end_matches('/'), entry.pid, ext);
    let path = if compress {
        format!("{}.zst", base)
    } else {
        base
    };
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

fn read_log_entries(path: &Path) -> io::Result<Vec<LogEntry>> {
    let file = fs::File::open(path)?;
    let is_zst = path.extension().and_then(|e| e.to_str()) == Some("zst");
    let reader: Box<dyn std::io::Read> = if is_zst {
        Box::new(zstd::Decoder::new(file)?)
    } else {
        Box::new(file)
    };

    let ext = {
        let mut base = path.to_path_buf();
        if is_zst {
            base.set_extension("");
        }
        base.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string()
    };

    if ext == "msgs" {
        let mut r = reader;
        let mut entries = Vec::new();
        loop {
            match read_msgpack(&mut r) {
                Ok(e) => entries.push(e),
                Err(MsgpackError::InvalidMarkerRead(ref ioe))
                | Err(MsgpackError::InvalidDataRead(ref ioe))
                    if ioe.kind() == io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e)),
            }
        }
        Ok(entries)
    } else {
        let buf = BufReader::new(reader);
        let mut entries = Vec::new();
        for line in buf.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<LogEntry>(&line) {
                Ok(e) => entries.push(e),
                Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e)),
            }
        }
        Ok(entries)
    }
}

fn dump(path: &str) {
    let p = Path::new(path);
    if p.is_dir() {
        if let Ok(entries) = fs::read_dir(p) {
            for entry in entries.flatten() {
                let file_path = entry.path();
                if file_path.is_file() {
                    dump_file(&file_path);
                }
            }
        }
    } else {
        dump_file(p);
    }
}

fn dump_file(path: &Path) {
    println!("{}", path.display());
    match read_log_entries(path) {
        Ok(entries) => {
            for e in entries {
                println!("{:?}", e);
            }
        }
        Err(e) => eprintln!("failed to read {}: {}", path.display(), e),
    }
}
