use chrono::Utc;
use log::{info, warn};
use regex::Regex;
use rmp_serde::decode::{Error as MsgpackError, from_read as read_msgpack};
use rmp_serde::encode::write_named;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::sleep,
    time::Duration,
};

mod config;
mod procinfo;
mod stacktrace;

use crate::config::{
    Cli, Commands, Config, RunArgs, load_config, merge_config, parse_cli, uid_from_name,
};
use crate::procinfo::{
    ProcState, detect_fd_events, get_proc_usage, pid_uid, proc_cpu_time_sec, process_name,
    read_pids, rss_kb, should_suppress, swap_kb, vsz_kb,
};
use crate::stacktrace::{capture_c_stack_traces, capture_python_stack_traces};
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
    threads: Vec<ThreadInfo>,
}

#[derive(Serialize, Deserialize, Debug)]
struct ThreadInfo {
    tid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    stacktrace: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    python_stacktrace: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Debug)]
struct FdLogEvent {
    fd: i32,
    event: String,
    path: String,
}

fn main() {
    env_logger::init();
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
    let config = match args.config.as_deref() {
        Some(path) => load_config(path),
        None => Config::default(),
    };
    let config = merge_config(config, &args);

    let ignore_patterns: Vec<Regex> = config
        .filter
        .ignore_process_name
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| Regex::new(&p).ok())
        .collect();

    let mut format = config.output.format.as_deref().unwrap_or("jsonl.zst");
    format = match format {
        "json" => "jsonl",
        "json.zst" => "jsonl.zst",
        "msgpack" => "msgpacks",
        "msgpack.zst" => "msgpacks.zst",
        other => other,
    };
    let use_msgpack = matches!(format, "msgpacks" | "msgpacks.zst");
    let compress = config
        .output
        .compress
        .unwrap_or_else(|| format.ends_with(".zst"));
    let verbose = args.verbose;

    let output_dir = config.output.path.as_deref();
    if let Some(dir) = output_dir {
        if let Err(e) = fs::create_dir_all(dir) {
            warn!("failed to create {}: {}", dir, e);
        }
    }

    let target_pid = args.pid.map(|p| p as u32);
    let target_uid = config.filter.target_user.as_deref().and_then(uid_from_name);

    let interval = config.monitor.interval_sec.unwrap_or(0);
    let sleep_dur = if interval == 0 {
        Duration::from_millis(200)
    } else {
        Duration::from_secs(interval)
    };

    let record_cpu_percent_threshold = config
        .monitor
        .record_cpu_time_percent_threshold
        .unwrap_or(0.0);
    let stacktrace_cpu_percent_threshold = config
        .monitor
        .stacktrace_cpu_time_percent_threshold
        .unwrap_or(1.0);

    let term = Arc::new(AtomicBool::new(false));
    {
        let t = term.clone();
        ctrlc::set_handler(move || {
            t.store(true, Ordering::SeqCst);
            info!("SIGINT received, shutting down");
        })
        .expect("set SIGINT handler");
    }

    let mut states: HashMap<u32, ProcState> = HashMap::new();
    loop {
        monitor_iteration(
            &mut states,
            target_pid,
            target_uid,
            &ignore_patterns,
            record_cpu_percent_threshold,
            stacktrace_cpu_percent_threshold,
            output_dir,
            use_msgpack,
            compress,
            verbose,
        );
        if term.load(Ordering::SeqCst) {
            break;
        }
        let mut elapsed = Duration::from_millis(0);
        while elapsed < sleep_dur {
            if term.load(Ordering::SeqCst) {
                return;
            }
            let step = std::cmp::min(Duration::from_millis(100), sleep_dur - elapsed);
            sleep(step);
            elapsed += step;
        }
        if term.load(Ordering::SeqCst) {
            break;
        }
    }
}

fn monitor_iteration(
    states: &mut HashMap<u32, ProcState>,
    target_pid: Option<u32>,
    target_uid: Option<u32>,
    ignore_patterns: &[Regex],
    record_cpu_percent_threshold: f64,
    stacktrace_cpu_percent_threshold: f64,
    output_dir: Option<&str>,
    use_msgpack: bool,
    compress: bool,
    verbose: bool,
) {
    let pids = collect_pids(target_pid, target_uid);
    if verbose {
        println!("Found {} PIDs", pids.len());
    }
    prune_states(states, &pids);
    for pid in &pids {
        process_pid(
            *pid,
            states,
            target_pid,
            ignore_patterns,
            record_cpu_percent_threshold,
            stacktrace_cpu_percent_threshold,
            output_dir,
            use_msgpack,
            compress,
            verbose,
        );
    }
}

fn collect_pids(target_pid: Option<u32>, target_uid: Option<u32>) -> Vec<u32> {
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
    pids
}

fn prune_states(states: &mut HashMap<u32, ProcState>, pids: &[u32]) {
    let existing: Vec<u32> = states.keys().copied().collect();
    let pid_set: HashSet<u32> = pids.iter().copied().collect();
    for old in &existing {
        if !pid_set.contains(old) {
            states.remove(old);
            info!("process {} disappeared", old);
        }
    }
}

fn process_pid(
    pid: u32,
    states: &mut HashMap<u32, ProcState>,
    target_pid: Option<u32>,
    ignore_patterns: &[Regex],
    record_cpu_percent_threshold: f64,
    stacktrace_cpu_percent_threshold: f64,
    output_dir: Option<&str>,
    use_msgpack: bool,
    compress: bool,
    verbose: bool,
) {
    let is_new = !states.contains_key(&pid);
    let state = states.entry(pid).or_default();
    let usage = get_proc_usage(pid, state);
    let cpu = usage.map(|u| u.0).unwrap_or(0.0);
    if should_skip_pid(pid, target_pid, ignore_patterns, record_cpu_percent_threshold, cpu) {
        return;
    }
    if is_new {
        info!("new process {}", pid);
    }
    let raw_events = detect_fd_events(pid, state);
    state.pending_fd_events.extend(raw_events);
    let rss = usage
        .map(|u| u.1)
        .unwrap_or_else(|| rss_kb(pid).unwrap_or(0));
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
        let entry = build_log_entry(pid, cpu, rss, fd_log_events, stacktrace_cpu_percent_threshold);
        if verbose {
            if let Ok(line) = serde_json::to_string(&entry) {
                println!("{}", line);
            }
        }
        write_log(dir, &entry, use_msgpack, compress);
    }
}

fn should_skip_pid(
    pid: u32,
    target_pid: Option<u32>,
    ignore_patterns: &[Regex],
    record_cpu_percent_threshold: f64,
    cpu_percent: f32,
) -> bool {
    if target_pid.is_none() {
        if let Some(name) = process_name(pid) {
            if ignore_patterns.iter().any(|re| re.is_match(&name)) {
                return true;
            }
        }
        if cpu_percent < record_cpu_percent_threshold as f32 {
            return true;
        }
    }
    false
}

fn build_log_entry(
    pid: u32,
    cpu: f32,
    rss: u64,
    fd_events: Vec<FdLogEvent>,
    stacktrace_cpu_percent_threshold: f64,
) -> LogEntry {
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
        threads: Vec::new(),
    };
    if cpu < stacktrace_cpu_percent_threshold as f32 {
        let name = &entry.process_name;
        let mut c_traces = capture_c_stack_traces(pid as i32);
        let mut py_traces = if name.starts_with("python") {
            match capture_python_stack_traces(pid as i32) {
                Ok(t) => t,
                Err(e) => {
                    warn!("python trace failed: {}", e);
                    HashMap::new()
                }
            }
        } else {
            HashMap::new()
        };
        for (tid, c) in c_traces.drain(..) {
            let py = py_traces.remove(&(tid as u32));
            entry.threads.push(ThreadInfo {
                tid: tid as u32,
                stacktrace: c,
                python_stacktrace: py,
            });
        }
        for (tid, py) in py_traces.into_iter() {
            entry.threads.push(ThreadInfo {
                tid,
                stacktrace: None,
                python_stacktrace: Some(py),
            });
        }
    }
    entry
}

fn write_log(dir: &str, entry: &LogEntry, use_msgpack: bool, compress: bool) {
    let ext = if use_msgpack { "msgpacks" } else { "jsonl" };
    let base = format!("{}/{}.{}", dir.trim_end_matches('/'), entry.pid, ext);
    let path = if compress {
        format!("{}.zst", base)
    } else {
        base
    };
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => {
            if compress {
                match zstd::Encoder::new(file, 0) {
                    Ok(mut enc) => {
                        if use_msgpack {
                            if let Err(e) = write_named(&mut enc, entry) {
                                warn!("write msgpack failed: {}", e);
                            }
                        } else {
                            if serde_json::to_writer(&mut enc, entry).is_err() {
                                warn!("write json failed");
                            }
                            if enc.write_all(b"\n").is_err() {
                                warn!("write newline failed");
                            }
                        }
                        if let Err(e) = enc.finish() {
                            warn!("finish zstd failed: {}", e);
                        }
                    }
                    Err(e) => warn!("zstd init failed: {}", e),
                }
            } else {
                let mut file = file;
                if use_msgpack {
                    if let Err(e) = write_named(&mut file, entry) {
                        warn!("write msgpack failed: {}", e);
                    }
                } else {
                    if serde_json::to_writer(&mut file, entry).is_err() {
                        warn!("write json failed");
                    }
                    if file.write_all(b"\n").is_err() {
                        warn!("write newline failed");
                    }
                }
            }
        }
        Err(e) => warn!("open {} failed: {}", path, e),
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

    if ext == "msgpacks" {
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
