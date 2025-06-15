use addr2line::Loader;
use log::{info, warn};
use nix::sys::{ptrace, wait::waitpid};
use nix::unistd::Pid;
use object::{Object, ObjectKind};
use py_spy::{Config as PySpyConfig, PythonSpy};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::rc::Rc;
use std::time::SystemTime;

struct CachedLoader {
    loader: Option<Rc<Loader>>,
    mtime: Option<SystemTime>,
}

thread_local! {
    static LOADER_CACHE: RefCell<HashMap<String, CachedLoader>> = RefCell::new(HashMap::new());
}

fn get_loader(path: &str) -> Option<Rc<Loader>> {
    if path.starts_with("[") {
        return None;
    }
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return None,
    };
    if !meta.file_type().is_file() {
        return None;
    }
    let mtime = meta.modified().ok();
    LOADER_CACHE.with(|cache| {
        let mut map = cache.borrow_mut();
        if let Some(entry) = map.get(path) {
            if entry.mtime == mtime {
                return entry.loader.clone();
            }
            map.remove(path);
        }
        match Loader::new(path) {
            Ok(loader) => {
                info!("load debug symbols from {}", path);
                let rc = Rc::new(loader);
                map.insert(
                    path.to_string(),
                    CachedLoader {
                        loader: Some(rc.clone()),
                        mtime,
                    },
                );
                Some(rc)
            }
            Err(e) => {
                warn!("Loader::new {} failed: {}", path, e);
                map.insert(
                    path.to_string(),
                    CachedLoader { loader: None, mtime },
                );
                None
            }
        }
    })
}

pub struct ExeInfo {
    pub start: u64,
    pub end: u64,
    pub offset: u64,
}

pub struct Module {
    pub loader: Rc<Loader>,
    pub info: ExeInfo,
    pub is_pic: bool,
}

pub fn load_loaders(pid: i32) -> Vec<Module> {
    let maps = match fs::read_to_string(format!("/proc/{}/maps", pid)) {
        Ok(m) => m,
        Err(e) => {
            warn!("read maps {} failed: {}", pid, e);
            return Vec::new();
        }
    };
    let mut infos: HashMap<String, ExeInfo> = HashMap::new();
    for line in maps.lines() {
        let mut parts = line.split_whitespace();
        let range = match parts.next() {
            Some(v) => v,
            None => continue,
        };
        let _perms = match parts.next() {
            Some(v) => v,
            None => continue,
        };
        let offset = match parts.next() {
            Some(v) => v,
            None => continue,
        };
        let _dev = parts.next();
        let _inode = parts.next();
        let path = match parts.next() {
            Some(v) => v,
            None => continue,
        };
        if let Some((start, end)) = range.split_once('-') {
            if let (Ok(start_addr), Ok(end_addr), Ok(off)) = (
                u64::from_str_radix(start, 16),
                u64::from_str_radix(end, 16),
                u64::from_str_radix(offset, 16),
            ) {
                let entry = infos.entry(path.to_string()).or_insert(ExeInfo {
                    start: start_addr,
                    end: end_addr,
                    offset: off,
                });
                if start_addr < entry.start {
                    entry.start = start_addr;
                    entry.offset = off;
                }
                if end_addr > entry.end {
                    entry.end = end_addr;
                }
            }
        }
    }
    let mut modules = Vec::new();
    for (path, info) in infos {
        if let Some(loader) = get_loader(&path) {
            match fs::read(&path) {
                Ok(data) => match object::File::parse(&*data) {
                    Ok(obj) => {
                        let is_pic = matches!(obj.kind(), ObjectKind::Dynamic);
                        modules.push(Module {
                            loader,
                            info,
                            is_pic,
                        });
                    }
                    Err(e) => warn!("parse {} failed: {}", path, e),
                },
                Err(e) => warn!("read {} failed: {}", path, e),
            }
        }
    }
    modules
}

fn describe_addr(loader: &Rc<Loader>, info: &ExeInfo, addr: u64, is_pic: bool) -> Option<String> {
    if addr < info.start || addr >= info.end {
        return None;
    }
    let mut probe = addr;
    if is_pic {
        probe = addr.wrapping_sub(info.start).wrapping_add(info.offset);
    }
    probe = probe.wrapping_sub(loader.relative_address_base());
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
    if info_str.is_empty() {
        None
    } else {
        Some(info_str)
    }
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

pub fn capture_stack_trace(pid: i32) -> nix::Result<Vec<String>> {
    let target = Pid::from_raw(pid);
    ptrace::attach(target)?;
    waitpid(target, None)?;

    let res = (|| {
        let stack = get_stack_trace(target, 32)?;
        let modules = load_loaders(pid);
        let mut lines = Vec::new();
        for (i, addr) in stack.iter().enumerate() {
            let mut line = format!("{:>2}: {:#x}", i, addr);
            for m in &modules {
                if let Some(info) = describe_addr(&m.loader, &m.info, *addr, m.is_pic) {
                    line = format!("{:>2}: {:#x} {}", i, addr, info);
                    break;
                }
            }
            lines.push(line);
        }
        Ok(lines)
    })();

    if let Err(e) = ptrace::detach(target, None) {
        warn!("detach failed: {}", e);
    }
    res
}

pub fn capture_c_stack_traces(pid: i32) -> Vec<(i32, Option<Vec<String>>)> {
    let mut tids: Vec<i32> = match fs::read_dir(format!("/proc/{}/task", pid)) {
        Ok(d) => d
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter_map(|s| s.parse::<i32>().ok())
            .collect(),
        Err(_) => Vec::new(),
    };
    tids.sort_unstable();
    let mut traces = Vec::new();
    for tid in tids {
        match capture_stack_trace(tid) {
            Ok(t) => traces.push((tid, Some(t))),
            Err(_) => traces.push((tid, None)),
        }
    }
    traces
}

pub fn capture_python_stack_traces(
    pid: i32,
) -> Result<HashMap<u32, Vec<String>>, Box<dyn std::error::Error>> {
    let config = PySpyConfig::default();
    let mut spy = PythonSpy::new(pid as py_spy::Pid, &config)?;
    let traces = spy.get_stack_traces()?;
    let mut result = HashMap::new();
    for t in traces {
        if let Some(tid) = t.os_thread_id {
            let mut lines = Vec::new();
            for f in t.frames {
                lines.push(format!("{} {}:{}", f.name, f.filename, f.line));
            }
            result.insert(tid as u32, lines);
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    fn clear_cache() {
        LOADER_CACHE.with(|c| c.borrow_mut().clear());
    }

    #[test]
    fn loader_none_for_nonexistent() {
        clear_cache();
        assert!(get_loader("/no/such/file").is_none());
    }

    #[test]
    fn loader_none_for_non_regular() {
        clear_cache();
        assert!(get_loader("/dev/null").is_none());
    }

    #[test]
    fn loader_retry_after_update() {
        clear_cache();
        let dir = tempdir().unwrap();
        let exe = dir.path().join("tprog");
        std::fs::write(&exe, b"bad").unwrap();
        assert!(get_loader(exe.to_str().unwrap()).is_none());
        assert!(get_loader(exe.to_str().unwrap()).is_none());

        std::thread::sleep(std::time::Duration::from_millis(1100));
        let src = dir.path().join("t.c");
        std::fs::write(&src, "int main(){return 0;}").unwrap();
        let status = Command::new("gcc")
            .args([src.to_str().unwrap(), "-o", exe.to_str().unwrap()])
            .status()
            .expect("compile");
        assert!(status.success());
        assert!(get_loader(exe.to_str().unwrap()).is_some());
    }
}
