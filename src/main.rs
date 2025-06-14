use std::{env, fs};
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
        println!("Stack trace for pid {}:", pid);
        for (i, addr) in stack.iter().enumerate() {
            println!("{:>2}: {:#x}", i, addr);
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

fn main() {
    if let Some(pid) = parse_pid() {
        if let Err(e) = attach_and_trace(pid) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    let pids = read_pids();
    println!("Found {} PIDs: {:?}", pids.len(), pids);
}
