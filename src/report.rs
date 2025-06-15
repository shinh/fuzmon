use chrono::Utc;
use html_escape::encode_text;
use log::warn;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{ReportArgs, finalize_report_config, load_config};
use crate::log::{LogEntry, read_log_entries};

#[derive(Clone)]
struct Stats {
    pid: u32,
    cmd: String,
    env: Option<String>,
    runtime: i64,
    cpu: f64,
    peak_rss: u64,
    path: String,
}

fn calc_stats(path: &Path, entries: &[LogEntry]) -> Option<Stats> {
    if entries.is_empty() {
        return None;
    }
    let mut sorted: Vec<&LogEntry> = entries.iter().collect();
    sorted.sort_by_key(|e| e.timestamp.clone());
    let first = sorted[0];
    let pid = first.pid;
    let cmd = first.cmdline.clone().unwrap_or_else(|| "(unknown)".into());
    let env = first.env.clone();
    let start = chrono::DateTime::parse_from_rfc3339(&first.timestamp)
        .map(|t| t.with_timezone(&Utc))
        .unwrap();
    let end = chrono::DateTime::parse_from_rfc3339(&sorted.last().unwrap().timestamp)
        .map(|t| t.with_timezone(&Utc))
        .unwrap();
    let runtime = (end - start).num_seconds();
    let mut cpu = 0.0f64;
    let mut peak_rss = 0u64;
    for win in sorted.windows(2) {
        if let [a, b] = win {
            let ta = chrono::DateTime::parse_from_rfc3339(&a.timestamp)
                .map(|t| t.with_timezone(&Utc))
                .unwrap();
            let tb = chrono::DateTime::parse_from_rfc3339(&b.timestamp)
                .map(|t| t.with_timezone(&Utc))
                .unwrap();
            let dt = (tb - ta).num_seconds() as f64;
            cpu += a.cpu_time_percent * dt / 100.0;
        }
    }
    for e in &sorted {
        peak_rss = peak_rss.max(e.memory.rss_kb);
    }
    Some(Stats {
        pid,
        cmd,
        env,
        runtime,
        cpu,
        peak_rss,
        path: path.display().to_string(),
    })
}

fn collect_files(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                collect_files(&p, files);
            } else if p.is_file() {
                files.push(p);
            }
        }
    }
}

fn report_file(path: &Path) {
    match read_log_entries(path) {
        Ok(entries) => {
            if let Some(s) = calc_stats(path, &entries) {
                print_single(&s);
            } else {
                println!("<p>No entries</p>");
            }
        }
        Err(e) => warn!("failed to read {}: {}", path.display(), e),
    }
}

fn print_single(s: &Stats) {
    println!("<html><body>");
    println!("<h1>Report for PID {}</h1>", s.pid);
    println!("<p>Command: {}</p>", encode_text(&s.cmd));
    println!("<ul>");
    println!("<li>Total runtime: {} sec</li>", s.runtime);
    println!("<li>Total CPU time: {:.1} sec</li>", s.cpu);
    println!("<li>Peak RSS: {} KB</li>", s.peak_rss);
    println!("</ul>");
    if let Some(e) = &s.env {
        if !e.is_empty() {
            println!(
                "<details><summary>Environment</summary><pre>{}</pre></details>",
                encode_text(e)
            );
        }
    } else {
        println!("<p>Environment: unknown</p>");
    }
    println!("</body></html>");
}

fn report_dir(path: &Path, top_cpu: usize, top_rss: usize) {
    let mut files = Vec::new();
    collect_files(path, &mut files);
    let mut stats = Vec::new();
    for f in files {
        match read_log_entries(&f) {
            Ok(entries) => {
                if let Some(s) = calc_stats(&f, &entries) {
                    stats.push(s);
                }
            }
            Err(e) => warn!("failed to read {}: {}", f.display(), e),
        }
    }
    if stats.is_empty() {
        println!("<p>No entries</p>");
        return;
    }

    let mut by_cpu = stats.clone();
    by_cpu.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap());
    let mut by_rss = stats.clone();
    by_rss.sort_by_key(|s| std::cmp::Reverse(s.peak_rss));

    let mut map: HashMap<String, Stats> = HashMap::new();
    for s in by_cpu.into_iter().take(top_cpu) {
        map.entry(s.path.clone()).or_insert(s);
    }
    for s in by_rss.into_iter().take(top_rss) {
        map.entry(s.path.clone()).or_insert(s);
    }
    let mut selected: Vec<_> = map.into_values().collect();
    selected.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap());

    println!("<html><body>");
    println!("<table>");
    println!(
        "<tr><th>PID</th><th>Command</th><th>Total runtime</th><th>Total CPU time</th><th>Peak RSS</th></tr>"
    );
    for s in &selected {
        println!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{:.1}</td><td>{}</td></tr>",
            s.pid,
            encode_text(&s.cmd),
            s.runtime,
            s.cpu,
            s.peak_rss
        );
    }
    println!("</table></body></html>");
}

pub fn report(args: &ReportArgs) {
    let cfg = if let Some(ref path) = args.config {
        finalize_report_config(load_config(path).report)
    } else {
        finalize_report_config(Default::default())
    };
    let path = Path::new(&args.path);
    if path.is_dir() {
        report_dir(path, cfg.top_cpu.unwrap_or(10), cfg.top_rss.unwrap_or(10));
    } else {
        report_file(path);
    }
}
