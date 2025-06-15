use log::warn;
use rmp_serde::decode::{Error as MsgpackError, from_read as read_msgpack};
use rmp_serde::encode::write_named;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

use fuzmon::utils::current_date_string;

#[derive(Serialize, Deserialize, Debug)]
pub struct MemoryInfo {
    pub rss_kb: u64,
    pub vsz_kb: u64,
    pub swap_kb: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Frame {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addr: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub func: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<i32>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ThreadInfo {
    pub tid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stacktrace: Option<Vec<Frame>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub python_stacktrace: Option<Vec<Frame>>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct FdLogEvent {
    pub fd: i32,
    pub event: String,
    pub path: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LogEntry {
    pub timestamp: String,
    pub pid: u32,
    pub process_name: String,
    pub cpu_time_percent: f64,
    pub memory: MemoryInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cmdline: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fd_events: Option<Vec<FdLogEvent>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub threads: Vec<ThreadInfo>,
}

pub fn write_log(dir: &str, entry: &LogEntry, use_msgpack: bool, compress: bool) {
    let date = current_date_string();
    let dir = format!("{}/{}", dir.trim_end_matches('/'), date);
    if let Err(e) = fs::create_dir_all(&dir) {
        warn!("failed to create {}: {}", dir, e);
    }
    let ext = if use_msgpack { "msgpacks" } else { "jsonl" };
    let base = format!("{}/{}.{}", dir, entry.pid, ext);
    let path = if compress {
        format!("{}.zst", base)
    } else {
        base
    };
    match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(file) => {
            if compress {
                match zstd::Encoder::new(file, 0) {
                    Ok(mut enc) => {
                        if use_msgpack {
                            if let Err(e) = write_named(&mut enc, entry) {
                                warn!("write msgpack failed: {}", e);
                            }
                        } else {
                            if serde_json::to_writer(&mut enc, entry).is_err() {
                                warn!("write json failed");
                            }
                            if enc.write_all(b"\n").is_err() {
                                warn!("write newline failed");
                            }
                        }
                        if let Err(e) = enc.finish() {
                            warn!("finish zstd failed: {}", e);
                        }
                    }
                    Err(e) => warn!("zstd init failed: {}", e),
                }
            } else {
                let mut file = file;
                if use_msgpack {
                    if let Err(e) = write_named(&mut file, entry) {
                        warn!("write msgpack failed: {}", e);
                    }
                } else {
                    if serde_json::to_writer(&mut file, entry).is_err() {
                        warn!("write json failed");
                    }
                    if file.write_all(b"\n").is_err() {
                        warn!("write newline failed");
                    }
                }
            }
        }
        Err(e) => warn!("open {} failed: {}", path, e),
    }
}

pub fn read_log_entries(path: &Path) -> io::Result<Vec<LogEntry>> {
    let file = fs::File::open(path)?;
    let is_zst = path.extension().and_then(|e| e.to_str()) == Some("zst");
    let reader: Box<dyn std::io::Read> = if is_zst {
        Box::new(zstd::Decoder::new(file)?)
    } else {
        Box::new(file)
    };

    let ext = {
        let mut base = path.to_path_buf();
        if is_zst {
            base.set_extension("");
        }
        base.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string()
    };

    if ext == "msgpacks" {
        let mut r = reader;
        let mut entries = Vec::new();
        loop {
            match read_msgpack(&mut r) {
                Ok(e) => entries.push(e),
                Err(MsgpackError::InvalidMarkerRead(ref ioe))
                | Err(MsgpackError::InvalidDataRead(ref ioe))
                    if ioe.kind() == io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e)),
            }
        }
        Ok(entries)
    } else {
        let buf = BufReader::new(reader);
        let mut entries = Vec::new();
        for line in buf.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<LogEntry>(&line) {
                Ok(e) => entries.push(e),
                Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidData, e)),
            }
        }
        Ok(entries)
    }
}
