use std::borrow::Cow;
use std::fs;
use addr2line::Loader;
use nix::sys::{ptrace, wait::waitpid};
use nix::unistd::Pid;
use py_spy::{Config as PySpyConfig, PythonSpy};
use crate::procinfo::process_name;


pub struct ExeInfo {
    pub start: u64,
    pub end: u64,
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

pub fn load_loader(pid: i32) -> Option<(Loader, ExeInfo)> {
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
    let mut info_str = String::new();
    let mut found_frames = false;
    if let Ok(mut frames) = loader.find_frames(probe) {
        let mut first = true;
        while let Ok(Some(frame)) = frames.next() {
            found_frames = true;
            if !first {
                info_str.push_str(" (inlined by) ");
            }
            first = false;
            if let Some(func) = frame.function {
                if !info_str.is_empty() {
                    info_str.push(' ');
                }
                let name = func.demangle().unwrap_or_else(|_| Cow::from("??"));
                info_str.push_str(&name);
            }
            if let Some(loc) = frame.location {
                if let (Some(file), Some(line)) = (loc.file, loc.line) {
                    info_str.push_str(&format!(" at {}:{}", file, line));
                }
            }
        }
    }
    if !found_frames {
        if let Some(sym) = loader.find_symbol(probe) {
            info_str.push_str(sym);
        }
    }
    if info_str.is_empty() { None } else { Some(info_str) }
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

pub fn attach_and_trace(pid: i32) -> nix::Result<()> {
    if let Some(name) = process_name(pid as u32) {
        if name.starts_with("python") {
            if let Ok(trace) = capture_python_stack_trace(pid) {
                println!("Stack trace for pid {}:", pid);
                for line in trace {
                    println!("{}", line);
                }
                return Ok(());
            }
        }
    }
    let trace = capture_stack_trace(pid)?;
    println!("Stack trace for pid {}:", pid);
    for line in trace {
        println!("{}", line);
    }
    Ok(())
}

pub fn capture_stack_trace(pid: i32) -> nix::Result<Vec<String>> {
    let target = Pid::from_raw(pid);
    ptrace::attach(target)?;
    waitpid(target, None)?;

    let res = (|| {
        let stack = get_stack_trace(target, 32)?;
        let loader = load_loader(pid);
        let mut lines = Vec::new();
        for (i, addr) in stack.iter().enumerate() {
            let line = if let Some((ref l, ref exe)) = loader {
                if let Some(info) = describe_addr(l, exe, *addr) {
                    format!("{:>2}: {:#x} {}", i, addr, info)
                } else {
                    format!("{:>2}: {:#x}", i, addr)
                }
            } else {
                format!("{:>2}: {:#x}", i, addr)
            };
            lines.push(line);
        }
        Ok(lines)
    })();

    let _ = ptrace::detach(target, None);
    res
}

pub fn capture_python_stack_trace(pid: i32) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let config = PySpyConfig::default();
    let mut spy = PythonSpy::new(pid as py_spy::Pid, &config)?;
    let traces = spy.get_stack_traces()?;
    let mut lines = Vec::new();
    for t in traces {
        lines.push(format!("thread {}", t.thread_id));
        for f in t.frames {
            lines.push(format!("    {} {}:{}", f.name, f.filename, f.line));
        }
    }
    Ok(lines)
}
