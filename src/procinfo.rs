use std::fs;
use std::os::unix::fs::MetadataExt;
use nix::libc;

#[derive(Default)]
pub struct ProcState {
    pub prev_proc_time: u64,
    pub prev_total_time: u64,
}

pub fn pid_uid(pid: u32) -> Option<u32> {
    fs::metadata(format!("/proc/{}", pid)).ok().map(|m| m.uid())
}

pub fn read_pids() -> Vec<u32> {
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

pub fn proc_cpu_jiffies(pid: u32) -> Option<u64> {
    let (u, s) = read_proc_stat(pid)?;
    Some(u + s)
}

fn read_status_value(pid: u32, key: &str) -> Option<u64> {
    let status = fs::read_to_string(format!("/proc/{}/status", pid)).ok()?;
    for line in status.lines() {
        if line.starts_with(key) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(val) = parts.get(1) {
                return val.parse::<u64>().ok();
            }
        }
    }
    None
}

pub fn process_name(pid: u32) -> Option<String> {
    fs::read_to_string(format!("/proc/{}/comm", pid))
        .ok()
        .map(|s| s.trim().to_string())
}

pub fn vsz_kb(pid: u32) -> Option<u64> {
    read_status_value(pid, "VmSize:")
}

pub fn swap_kb(pid: u32) -> Option<u64> {
    read_status_value(pid, "VmSwap:")
}

pub fn proc_cpu_time_sec(pid: u32) -> Option<f64> {
    let (u, s) = read_proc_stat(pid)?;
    let clk = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    if clk > 0.0 {
        Some((u + s) as f64 / clk)
    } else {
        None
    }
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

pub fn get_proc_usage(pid: u32, state: &mut ProcState) -> Option<(f32, u64)> {
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

pub fn should_suppress(cpu: f32, rss_kb: u64) -> bool {
    cpu == 0.0 && rss_kb < 100 * 1024
}
