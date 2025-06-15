use std::fs;
use std::path::Path;

use crate::log::read_log_entries;

pub fn dump(path: &str) {
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
