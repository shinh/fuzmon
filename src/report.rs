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

fn render_single(s: &Stats) -> String {
    let mut out = String::new();
    out.push_str("<html><body>\n");
    out.push_str(&format!("<h1>Report for PID {}</h1>\n", s.pid));
    out.push_str(&format!("<p>Command: {}</p>\n", encode_text(&s.cmd)));
    out.push_str("<ul>\n");
    out.push_str(&format!("<li>Total runtime: {} sec</li>\n", s.runtime));
    out.push_str(&format!("<li>Total CPU time: {:.1} sec</li>\n", s.cpu));
    out.push_str(&format!("<li>Peak RSS: {} KB</li>\n", s.peak_rss));
    out.push_str("</ul>\n");
    if let Some(e) = &s.env {
        if !e.is_empty() {
            out.push_str(&format!(
                "<details><summary>Environment</summary><pre>{}</pre></details>\n",
                encode_text(e)
            ));
        }
    } else {
        out.push_str("<p>Environment: unknown</p>\n");
    }
    out.push_str("</body></html>\n");
    out
}

fn render_index(stats: &[Stats], link: bool) -> String {
    let mut out = String::new();
    out.push_str("<html><body>\n");
    out.push_str("<table>\n");
    out.push_str(
        "<tr><th>PID</th><th>Command</th><th>Total runtime</th><th>Total CPU time</th><th>Peak RSS</th></tr>\n",
    );
    for s in stats {
        let pid_cell = if link {
            format!("<a href=\"{}.html\">{}</a>", s.pid, s.pid)
        } else {
            s.pid.to_string()
        };
        out.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{:.1}</td><td>{}</td></tr>\n",
            pid_cell,
            encode_text(&s.cmd),
            s.runtime,
            s.cpu,
            s.peak_rss
        ));
    }
    out.push_str("</table></body></html>\n");
    out
}

fn report_file(path: &Path, out_dir: &Path) {
    match read_log_entries(path) {
        Ok(entries) => {
            if let Some(s) = calc_stats(path, &entries) {
                let html = render_single(&s);
                let index = out_dir.join("index.html");
                if let Err(e) = fs::write(&index, html) {
                    warn!("failed to write {}: {}", index.display(), e);
                }
            } else {
                let index = out_dir.join("index.html");
                if let Err(e) = fs::write(&index, "<p>No entries</p>") {
                    warn!("failed to write {}: {}", index.display(), e);
                }
            }
        }
        Err(e) => warn!("failed to read {}: {}", path.display(), e),
    }
}

fn report_dir(path: &Path, out_dir: &Path, top_cpu: usize, top_rss: usize) {
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
        let index = out_dir.join("index.html");
        if let Err(e) = fs::write(&index, "<p>No entries</p>") {
            warn!("failed to write {}: {}", index.display(), e);
        }
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

    // write index.html
    let index_html = render_index(&selected, true);
    let index_path = out_dir.join("index.html");
    if let Err(e) = fs::write(&index_path, index_html) {
        warn!("failed to write {}: {}", index_path.display(), e);
    }

    // write per pid files
    for s in &selected {
        match read_log_entries(Path::new(&s.path)) {
            Ok(entries) => {
                if let Some(stats) = calc_stats(Path::new(&s.path), &entries) {
                    let html = render_single(&stats);
                    let out = out_dir.join(format!("{}.html", s.pid));
                    if let Err(e) = fs::write(&out, html) {
                        warn!("failed to write {}: {}", out.display(), e);
                    }
                }
            }
            Err(e) => warn!("failed to read {}: {}", s.path, e),
        }
    }
}

pub fn report(args: &ReportArgs) {
    let cfg = if let Some(ref path) = args.config {
        finalize_report_config(load_config(path).report)
    } else {
        finalize_report_config(Default::default())
    };
    let input = Path::new(&args.path);
    let out_dir = if let Some(ref o) = args.output {
        PathBuf::from(o)
    } else {
        let name = input
            .file_stem()
            .or_else(|| input.file_name())
            .unwrap_or_default();
        PathBuf::from(name)
    };
    if let Err(e) = fs::create_dir_all(&out_dir) {
        warn!("failed to create {}: {}", out_dir.display(), e);
    }
    if input.is_dir() {
        report_dir(
            input,
            &out_dir,
            cfg.top_cpu.unwrap_or(10),
            cfg.top_rss.unwrap_or(10),
        );
    } else {
        report_file(input, &out_dir);
    }
    println!("{}", out_dir.display());
}
