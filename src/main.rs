use std::{env, fs, borrow::Cow, thread::sleep, time::Duration};
use addr2line::Loader;
use nix::sys::{ptrace, wait::waitpid};
use nix::unistd::Pid;

fn parse_pid() -> Option<i32> {
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "-p" || arg == "--pid" {
            if let Some(pid_str) = args.next() {
                match pid_str.parse::<i32>() {
                    Ok(p) => return Some(p),
                    Err(_) => {
                        eprintln!("Invalid PID: {}", pid_str);
                        std::process::exit(1);
                    }
                }
            } else {
                eprintln!("-p requires a PID argument");
                std::process::exit(1);
            }
        }
    }
    None
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

fn get_proc_usage(pid: u32) -> Option<(f32, u64)> {
    let (u1, s1) = read_proc_stat(pid)?;
    let t1 = read_total_cpu_time()?;
    sleep(Duration::from_millis(100));
    let (u2, s2) = read_proc_stat(pid)?;
    let t2 = read_total_cpu_time()?;
    let delta_proc = (u2 + s2).saturating_sub(u1 + s1);
    let delta_total = t2.saturating_sub(t1);
    if delta_total == 0 {
        return None;
    }
    let cpu = 100.0 * delta_proc as f32 / delta_total as f32;
    let rss = read_rss_kb(pid).unwrap_or(0);
    Some((cpu, rss))
}

fn main() {
    if let Some(pid) = parse_pid() {
        if let Err(e) = attach_and_trace(pid) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    let pids = read_pids();
    println!("Found {} PIDs", pids.len());
    for pid in pids {
        if let Some((cpu, rss)) = get_proc_usage(pid) {
            println!("PID {:>5}: {:>5.1}% CPU, {:>8} KB RSS", pid, cpu, rss);
        }
    }
}
