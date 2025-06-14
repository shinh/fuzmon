use std::{env, fs, borrow::Cow, thread::sleep, time::Duration, collections::HashMap};
use serde::Deserialize;
use addr2line::Loader;
use nix::sys::{ptrace, wait::waitpid};
use nix::unistd::Pid;
use std::os::unix::fs::MetadataExt;

#[derive(Default, Clone)]
struct CmdArgs {
    pid: Option<i32>,
    config: Option<String>,
    target_user: Option<String>,
}

#[derive(Default, Deserialize)]
struct FilterConfig {
    #[serde(default)]
    target_user: Option<String>,
    #[serde(default)]
    ignore_process_name: Option<Vec<String>>,
}

#[derive(Default, Deserialize)]
struct OutputConfig {
    #[serde(default)]
    format: Option<String>,
}

#[derive(Default, Deserialize)]
struct MonitorConfig {
    #[serde(default)]
    interval_sec: Option<u64>,
}

#[derive(Default, Deserialize)]
struct Config {
    #[serde(default)]
    filter: FilterConfig,
    #[serde(default)]
    output: OutputConfig,
    #[serde(default)]
    monitor: MonitorConfig,
}

fn load_config(path: &str) -> Option<Config> {
    let data = fs::read_to_string(path).ok()?;
    toml::from_str(&data).ok()
}

fn uid_from_name(name: &str) -> Option<u32> {
    let passwd = fs::read_to_string("/etc/passwd").ok()?;
    for line in passwd.lines() {
        let mut parts = line.split(':');
        if let (Some(user), Some(_), Some(uid_str)) = (parts.next(), parts.next(), parts.next()) {
            if user == name {
                if let Ok(uid) = uid_str.parse::<u32>() {
                    return Some(uid);
                }
            }
        }
    }
    None
}

fn pid_uid(pid: u32) -> Option<u32> {
    fs::metadata(format!("/proc/{}", pid)).ok().map(|m| m.uid())
}

fn merge_config(mut cfg: Config, args: &CmdArgs) -> Config {
    if let Some(ref u) = args.target_user {
        cfg.filter.target_user = Some(u.clone());
    }
    cfg
}

fn parse_args() -> CmdArgs {
    let mut args = env::args().skip(1);
    let mut out = CmdArgs::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-p" | "--pid" => {
                if let Some(pid_str) = args.next() {
                    out.pid = Some(pid_str.parse::<i32>().unwrap_or_else(|_| {
                        eprintln!("Invalid PID: {}", pid_str);
                        std::process::exit(1);
                    }));
                } else {
                    eprintln!("-p requires a PID argument");
                    std::process::exit(1);
                }
            }
            "-c" | "--config" => {
                if let Some(path) = args.next() {
                    out.config = Some(path);
                } else {
                    eprintln!("-c requires a file path");
                    std::process::exit(1);
                }
            }
            "--target_user" => {
                if let Some(val) = args.next() {
                    out.target_user = Some(val);
                } else {
                    eprintln!("--target_user requires a value");
                    std::process::exit(1);
                }
            }
            _ => {
                eprintln!("Unknown argument: {}", arg);
                std::process::exit(1);
            }
        }
    }
    out
}

struct ExeInfo {
    start: u64,
    end: u64,
}

fn find_exe_info(pid: i32) -> Option<ExeInfo> {
    let exe = fs::read_link(format!("/proc/{}/exe", pid)).ok()?;
    let maps = fs::read_to_string(format!("/proc/{}/maps", pid)).ok()?;
    for line in maps.lines() {
        if line.contains(exe.to_str()?) {
            let mut parts = line.split_whitespace();
            if let (Some(range), Some(perms), Some(offset)) = (parts.next(), parts.next(), parts.next()) {
                if perms.starts_with('r') && perms.contains('x') {
                    if let Some((start, end)) = range.split_once('-') {
                        if let (Ok(start_addr), Ok(end_addr), Ok(_off)) = (
                            u64::from_str_radix(start, 16),
                            u64::from_str_radix(end, 16),
                            u64::from_str_radix(offset, 16),
                        ) {
                            return Some(ExeInfo { start: start_addr, end: end_addr });
                        }
                    }
                }
            }
        }
    }
    None
}

fn load_loader(pid: i32) -> Option<(Loader, ExeInfo)> {
    let exe_path = fs::read_link(format!("/proc/{}/exe", pid)).ok()?;
    let loader = Loader::new(&exe_path).ok()?;
    let info = find_exe_info(pid)?;
    Some((loader, info))
}

fn describe_addr(loader: &Loader, info: &ExeInfo, addr: u64) -> Option<String> {
    if addr < info.start || addr >= info.end {
        return None;
    }
    let probe = addr.wrapping_sub(loader.relative_address_base());
    let mut info = String::new();
    let mut found_frames = false;
    if let Ok(mut frames) = loader.find_frames(probe) {
        let mut first = true;
        while let Ok(Some(frame)) = frames.next() {
            found_frames = true;
            if !first {
                info.push_str(" (inlined by) ");
            }
            first = false;
            if let Some(func) = frame.function {
                if !info.is_empty() {
                    info.push(' ');
                }
                let name = func.demangle().unwrap_or_else(|_| Cow::from("??"));
                info.push_str(&name);
            }
            if let Some(loc) = frame.location {
                if let (Some(file), Some(line)) = (loc.file, loc.line) {
                    info.push_str(&format!(" at {}:{}", file, line));
                }
            }
        }
    }
    if !found_frames {
        if let Some(sym) = loader.find_symbol(probe) {
            info.push_str(sym);
        }
    }
    if info.is_empty() { None } else { Some(info) }
}

fn get_stack_trace(pid: Pid, max_frames: usize) -> nix::Result<Vec<u64>> {
    let regs = ptrace::getregs(pid)?;
    let mut rbp = regs.rbp as u64;
    let mut addrs = Vec::new();
    addrs.push(regs.rip as u64);

    for _ in 0..max_frames {
        if rbp == 0 {
            break;
        }
        let next_rip = ptrace::read(pid, (rbp + 8) as ptrace::AddressType)? as u64;
        addrs.push(next_rip);
        let next_rbp = ptrace::read(pid, rbp as ptrace::AddressType)? as u64;
        if next_rbp == 0 {
            break;
        }
        rbp = next_rbp;
    }

    Ok(addrs)
}

fn attach_and_trace(pid: i32) -> nix::Result<()> {
    let target = Pid::from_raw(pid);
    ptrace::attach(target)?;
    waitpid(target, None)?;

    let res = (|| {
        let stack = get_stack_trace(target, 32)?;
        let loader = load_loader(pid);
        println!("Stack trace for pid {}:", pid);
        for (i, addr) in stack.iter().enumerate() {
            if let Some((ref l, ref exe)) = loader {
                if let Some(info) = describe_addr(l, exe, *addr) {
                    println!("{:>2}: {:#x} {}", i, addr, info);
                } else {
                    println!("{:>2}: {:#x}", i, addr);
                }
            } else {
                println!("{:>2}: {:#x}", i, addr);
            }
        }
        Ok(())
    })();

    let _ = ptrace::detach(target, None);
    res
}

fn read_pids() -> Vec<u32> {
    let mut pids = Vec::new();
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            if let Ok(name) = entry.file_name().into_string() {
                if let Ok(pid) = name.parse::<u32>() {
                    pids.push(pid);
                }
            }
        }
    }
    pids
}

fn read_proc_stat(pid: u32) -> Option<(u64, u64)> {
    let data = fs::read_to_string(format!("/proc/{}/stat", pid)).ok()?;
    let parts: Vec<&str> = data.split_whitespace().collect();
    let utime = parts.get(13)?.parse::<u64>().ok()?; // field 14
    let stime = parts.get(14)?.parse::<u64>().ok()?; // field 15
    Some((utime, stime))
}

fn read_total_cpu_time() -> Option<u64> {
    let data = fs::read_to_string("/proc/stat").ok()?;
    let line = data.lines().next()?;
    let mut total = 0u64;
    for v in line.split_whitespace().skip(1) {
        total += v.parse::<u64>().ok()?;
    }
    Some(total)
}

fn read_rss_kb(pid: u32) -> Option<u64> {
    let status = fs::read_to_string(format!("/proc/{}/status", pid)).ok()?;
    for line in status.lines() {
        if line.starts_with("VmRSS:") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(val) = parts.get(1) {
                return val.parse::<u64>().ok();
            }
        }
    }
    None
}

#[derive(Default)]
struct ProcState {
    prev_proc_time: u64,
    prev_total_time: u64,
}

fn get_proc_usage(pid: u32, state: &mut ProcState) -> Option<(f32, u64)> {
    let (u, s) = read_proc_stat(pid)?;
    let total = read_total_cpu_time()?;
    let proc_total = u + s;
    if state.prev_total_time == 0 {
        state.prev_proc_time = proc_total;
        state.prev_total_time = total;
        return None;
    }
    let delta_proc = proc_total.saturating_sub(state.prev_proc_time);
    let delta_total = total.saturating_sub(state.prev_total_time);
    state.prev_proc_time = proc_total;
    state.prev_total_time = total;
    if delta_total == 0 {
        return None;
    }
    let cpu = 100.0 * delta_proc as f32 / delta_total as f32;
    let rss = read_rss_kb(pid).unwrap_or(0);
    Some((cpu, rss))
}

fn should_suppress(cpu: f32, rss_kb: u64) -> bool {
    cpu == 0.0 && rss_kb < 100 * 1024
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::fs;

    #[test]
    fn load_example_config() {
        let cfg = load_config("ai_docs/example_config.toml").expect("load config");
        assert_eq!(cfg.output.format.as_deref(), Some("json"));
        assert_eq!(cfg.monitor.interval_sec, Some(60));
        assert_eq!(cfg.filter.target_user.as_deref(), Some("myname"));
    }

    #[test]
    fn cli_overrides_config() {
        let tmp = NamedTempFile::new().expect("tmp");
        fs::write(tmp.path(), "target_user = \"hoge\"").unwrap();
        let cfg = load_config(tmp.path().to_str().unwrap()).expect("load config");
        let args = CmdArgs { target_user: Some("foo".into()), ..Default::default() };
        let merged = merge_config(cfg, &args);
        assert_eq!(merged.filter.target_user.as_deref(), Some("foo"));
    }
}
