use std::{fs, time::Duration, thread};

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
    loop {
        let pids = read_pids();
        println!("Found {} PIDs: {:?}", pids.len(), pids);
        thread::sleep(Duration::from_secs(5));
    }
}
