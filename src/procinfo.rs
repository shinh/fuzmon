use log::warn;
use std::collections::HashMap;

fn compute_cpu_percent(delta_proc: u64, delta_total: u64, num_cpus: usize) -> f32 {
    if delta_total == 0 {
        return 0.0;
    }
    100.0 * delta_proc as f32 / delta_total as f32 * num_cpus as f32
}
use std::fs;
use std::os::unix::fs::MetadataExt;

#[derive(Default)]
pub struct ProcState {
    pub prev_proc_time: u64,
    pub prev_total_time: u64,
    pub fds: HashMap<i32, String>,
    pub pending_fd_events: Vec<FdEvent>,
    pub metadata_written: bool,
}

pub fn pid_uid(pid: u32) -> Option<u32> {
    match fs::metadata(format!("/proc/{}", pid)) {
        Ok(m) => Some(m.uid()),
        Err(e) => {
            warn!("metadata for {} failed: {}", pid, e);
            None
        }
    }
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
    } else {
        warn!("read_dir /proc failed");
    }
    pids
}

fn read_proc_stat(pid: u32) -> Option<(u64, u64)> {
    let data = match fs::read_to_string(format!("/proc/{}/stat", pid)) {
        Ok(d) => d,
        Err(e) => {
            warn!("read stat {} failed: {}", pid, e);
            return None;
        }
    };
    let parts: Vec<&str> = data.split_whitespace().collect();
    let utime = parts.get(13)?.parse::<u64>().ok()?; // field 14
    let stime = parts.get(14)?.parse::<u64>().ok()?; // field 15
    Some((utime, stime))
}

pub fn read_fd_map(pid: u32) -> HashMap<i32, String> {
    let mut map = HashMap::new();
    if let Ok(entries) = fs::read_dir(format!("/proc/{}/fd", pid)) {
        for entry in entries.flatten() {
            if let Ok(name) = entry.file_name().into_string() {
                if let Ok(fd) = name.parse::<i32>() {
                    match fs::read_link(entry.path()) {
                        Ok(target) => {
                            if let Some(path) = target.to_str() {
                                map.insert(fd, path.to_string());
                            }
                        }
                        Err(e) => warn!("read_link for {} fd {} failed: {}", pid, fd, e),
                    }
                }
            }
        }
    } else {
        warn!("read_dir fd for {} failed", pid);
    }
    map
}

#[derive(Debug)]
pub struct FdEvent {
    pub fd: i32,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
}

pub fn detect_fd_events(pid: u32, state: &mut ProcState) -> Vec<FdEvent> {
    let current = read_fd_map(pid);
    let mut events = Vec::new();
    for (fd, old_path) in &state.fds {
        match current.get(fd) {
            None => events.push(FdEvent {
                fd: *fd,
                old_path: Some(old_path.clone()),
                new_path: None,
            }),
            Some(new_path) if new_path != old_path => events.push(FdEvent {
                fd: *fd,
                old_path: Some(old_path.clone()),
                new_path: Some(new_path.clone()),
            }),
            _ => {}
        }
    }
    for (fd, new_path) in &current {
        if !state.fds.contains_key(fd) {
            events.push(FdEvent {
                fd: *fd,
                old_path: None,
                new_path: Some(new_path.clone()),
            });
        }
    }
    state.fds = current;
    events
}

fn read_status_value(pid: u32, key: &str) -> Option<u64> {
    let status = match fs::read_to_string(format!("/proc/{}/status", pid)) {
        Ok(s) => s,
        Err(e) => {
            warn!("read status {} failed: {}", pid, e);
            return None;
        }
    };
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

pub fn cmdline(pid: u32) -> Option<String> {
    fs::read(format!("/proc/{}/cmdline", pid)).ok().map(|data| {
        data.split(|&b| b == 0)
            .filter_map(|s| std::str::from_utf8(s).ok())
            .filter(|s| !s.is_empty())
            .collect::<Vec<&str>>()
            .join(" ")
    })
}

pub fn environ(pid: u32) -> Option<String> {
    fs::read(format!("/proc/{}/environ", pid)).ok().map(|data| {
        data.split(|&b| b == 0)
            .filter_map(|s| std::str::from_utf8(s).ok())
            .filter(|s| !s.is_empty())
            .collect::<Vec<&str>>()
            .join("\n")
    })
}

fn read_total_cpu_time() -> Option<u64> {
    let data = match fs::read_to_string("/proc/stat") {
        Ok(d) => d,
        Err(e) => {
            warn!("read /proc/stat failed: {}", e);
            return None;
        }
    };
    let line = data.lines().next()?;
    let mut total = 0u64;
    for v in line.split_whitespace().skip(1) {
        total += v.parse::<u64>().ok()?;
    }
    Some(total)
}

pub fn rss_kb(pid: u32) -> Option<u64> {
    let status = match fs::read_to_string(format!("/proc/{}/status", pid)) {
        Ok(s) => s,
        Err(e) => {
            warn!("read rss {} failed: {}", pid, e);
            return None;
        }
    };
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
    let cpu = compute_cpu_percent(delta_proc, delta_total, num_cpus::get());
    let rss = rss_kb(pid).unwrap_or(0);
    Some((cpu, rss))
}

pub fn should_suppress(cpu: f32, rss_kb: u64) -> bool {
    cpu == 0.0 && rss_kb < 100 * 1024
}

#[cfg(test)]
mod tests {
    use super::compute_cpu_percent;

    #[test]
    fn busy_two_threads_reports_200_percent() {
        let percent = compute_cpu_percent(2, 2, 2);
        assert!((percent - 200.0).abs() < f32::EPSILON);
    }
}
