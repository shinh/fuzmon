use chrono::Utc;
use log::{info, warn};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::sleep;
use std::time::Duration;

use crate::config::{Config, RunArgs, load_config, merge_config, uid_from_name};
use crate::log::{FdLogEvent, LogEntry, MemoryInfo, ThreadInfo, write_log};
use crate::procinfo::{
    ProcState, cmdline, detect_fd_events, environ, get_proc_usage, pid_uid, proc_exists,
    process_name, read_pids, rss_kb, should_suppress, swap_kb, vsz_kb,
};
use crate::stacktrace::{capture_c_stack_traces, capture_python_stack_traces};

pub fn run(args: RunArgs) {
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

    let mut child = None;
    let mut target_pid = args.pid.map(|p| p as u32);
    if target_pid.is_none() && !args.command.is_empty() {
        let mut cmd = std::process::Command::new(&args.command[0]);
        if args.command.len() > 1 {
            cmd.args(&args.command[1..]);
        }
        match cmd.spawn() {
            Ok(c) => {
                target_pid = Some(c.id());
                child = Some(c);
                info!("spawned {} as pid {}", args.command[0], target_pid.unwrap());
            }
            Err(e) => {
                let msg = format!("failed to spawn {}: {}", args.command[0], e);
                println!("{}", msg);
                warn!("{}", msg);
                return;
            }
        }
    }

    if let Some(pid) = target_pid {
        if fs::metadata(format!("/proc/{}", pid)).is_err() {
            let msg = format!("pid {} not found", pid);
            println!("{}", msg);
            warn!("{}", msg);
            return;
        }
    }

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
        if let Some(pid) = target_pid {
            if !proc_exists(pid) {
                let name = process_name(pid).unwrap_or_else(|| "?".to_string());
                let msg = format!("Process {pid} ({name}) disappeared, exiting");
                println!("{}", msg);
                info!("{}", msg);
                break;
            }
        }
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
        if let Some(ref mut c) = child {
            if c.try_wait().ok().flatten().is_some() {
                break;
            }
        } else if let Some(pid) = target_pid {
            if fs::metadata(format!("/proc/{}", pid)).is_err() {
                break;
            }
        }
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
    if term.load(Ordering::SeqCst) {
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
    }
    if let Some(mut c) = child {
        let _ = c.wait();
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
    prune_states(states, &pids, output_dir, use_msgpack, compress);
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
        if fs::metadata(format!("/proc/{}", pid)).is_ok() {
            vec![pid]
        } else {
            Vec::new()
        }
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

fn prune_states(
    states: &mut HashMap<u32, ProcState>,
    pids: &[u32],
    output_dir: Option<&str>,
    use_msgpack: bool,
    compress: bool,
) {
    let existing: Vec<u32> = states.keys().copied().collect();
    let pid_set: HashSet<u32> = pids.iter().copied().collect();
    for old in &existing {
        if !pid_set.contains(old) {
            if let Some(mut state) = states.remove(old) {
                if let Some(dir) = output_dir {
                    let events: Vec<FdLogEvent> = state
                        .fds
                        .drain()
                        .map(|(fd, path)| FdLogEvent {
                            fd,
                            event: "close".into(),
                            path,
                        })
                        .collect();
                    if !events.is_empty() {
                        let entry = LogEntry {
                            timestamp: Utc::now()
                                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                            pid: *old,
                            process_name: process_name(*old).unwrap_or_else(|| "?".into()),
                            cpu_time_percent: 0.0,
                            memory: MemoryInfo {
                                rss_kb: 0,
                                vsz_kb: 0,
                                swap_kb: 0,
                            },
                            cmdline: None,
                            env: None,
                            fd_events: Some(events),
                            threads: Vec::new(),
                        };
                        write_log(dir, &entry, use_msgpack, compress);
                    }
                }
            }
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
    if should_skip_pid(
        pid,
        target_pid,
        ignore_patterns,
        record_cpu_percent_threshold,
        cpu,
    ) {
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
        let entry = build_log_entry(
            pid,
            state,
            cpu,
            rss,
            fd_log_events,
            stacktrace_cpu_percent_threshold,
        );
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
    state: &mut ProcState,
    cpu_percent: f32,
    rss: u64,
    fd_events: Vec<FdLogEvent>,
    stacktrace_cpu_percent_threshold: f64,
) -> LogEntry {
    let mut entry = LogEntry {
        timestamp: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        pid,
        process_name: process_name(pid).unwrap_or_else(|| "?".into()),
        cpu_time_percent: cpu_percent as f64,
        memory: MemoryInfo {
            rss_kb: rss,
            vsz_kb: vsz_kb(pid).unwrap_or(0),
            swap_kb: swap_kb(pid).unwrap_or(0),
        },
        cmdline: None,
        env: None,
        fd_events: if fd_events.is_empty() {
            None
        } else {
            Some(fd_events)
        },
        threads: Vec::new(),
    };
    if !state.metadata_written {
        entry.cmdline = cmdline(pid);
        entry.env = environ(pid);
        state.metadata_written = true;
    }
    if cpu_percent >= stacktrace_cpu_percent_threshold as f32 {
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
