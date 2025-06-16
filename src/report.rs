use chrono::{DateTime, Utc};
use html_escape::encode_text;
use log::warn;
use plotters::prelude::*;
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::config::{ReportArgs, finalize_report_config, load_config};
use crate::log::{Frame, LogEntry, read_log_entries};

#[derive(Clone)]
struct Stats {
    pid: u32,
    cmd: String,
    env: Option<String>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    runtime: i64,
    cpu: f64,
    avg_cpu: f64,
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
    let avg_cpu = if runtime > 0 {
        cpu * 100.0 / runtime as f64
    } else {
        0.0
    };
    Some(Stats {
        pid,
        cmd,
        env,
        start,
        end,
        runtime,
        cpu,
        avg_cpu,
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

#[derive(Clone, Copy)]
enum GraphField {
    Cpu,
    Rss,
}

fn write_svg(entries: &[LogEntry], out: &Path, field: GraphField) -> io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut sorted: Vec<&LogEntry> = entries.iter().collect();
    sorted.sort_by_key(|e| e.timestamp.clone());
    let start = chrono::DateTime::parse_from_rfc3339(&sorted[0].timestamp)
        .map(|t| t.with_timezone(&Utc))
        .unwrap();
    let end = chrono::DateTime::parse_from_rfc3339(&sorted.last().unwrap().timestamp)
        .map(|t| t.with_timezone(&Utc))
        .unwrap();

    let mut max_val = 0.0f64;
    let mut series = Vec::new();
    for e in &sorted {
        let t = chrono::DateTime::parse_from_rfc3339(&e.timestamp)
            .map(|tt| tt.with_timezone(&Utc))
            .unwrap();
        let v = match field {
            GraphField::Cpu => e.cpu_time_percent,
            GraphField::Rss => e.memory.rss_kb as f64,
        };
        max_val = max_val.max(v);
        series.push((t, v));
    }
    if max_val <= 0.0 {
        max_val = 1.0;
    }

    let root = SVGBackend::new(out, (600, 300)).into_drawing_area();
    root.fill(&WHITE)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    let (y_desc, caption, scale) = match field {
        GraphField::Cpu => ("CPU %", "CPU usage (%)", 1.0),
        GraphField::Rss => {
            if max_val >= 1024.0 * 1024.0 {
                ("RSS GB", "Resident set size (GB)", 1024.0 * 1024.0)
            } else {
                ("RSS MB", "Resident set size (MB)", 1024.0)
            }
        }
    };
    let y_max = (max_val / scale).max(1.0);
    let mut chart = ChartBuilder::on(&root)
        .caption(caption, ("sans-serif", 20))
        .margin(5)
        .x_label_area_size(40)
        .y_label_area_size(40)
        .build_cartesian_2d(start..end, 0f64..y_max)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    chart
        .configure_mesh()
        .x_desc("time (UTC)")
        .y_desc(y_desc)
        .x_labels(5)
        .y_labels(5)
        .x_label_formatter(&|dt| dt.format("%H:%M:%S").to_string())
        .draw()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    chart
        .draw_series(LineSeries::new(
            series.into_iter().map(|(x, v)| (x, v / scale)),
            &BLUE,
        ))
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    root.present()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
}

fn collect_series(
    entries: &[LogEntry],
    field: GraphField,
) -> (Vec<(DateTime<Utc>, f64)>, DateTime<Utc>, DateTime<Utc>) {
    if entries.is_empty() {
        let now = Utc::now();
        return (Vec::new(), now, now);
    }
    let mut sorted: Vec<&LogEntry> = entries.iter().collect();
    sorted.sort_by_key(|e| e.timestamp.clone());
    let start = chrono::DateTime::parse_from_rfc3339(&sorted[0].timestamp)
        .map(|t| t.with_timezone(&Utc))
        .unwrap();
    let end = chrono::DateTime::parse_from_rfc3339(&sorted.last().unwrap().timestamp)
        .map(|t| t.with_timezone(&Utc))
        .unwrap();
    let mut series = Vec::new();
    for e in &sorted {
        let t = chrono::DateTime::parse_from_rfc3339(&e.timestamp)
            .map(|tt| tt.with_timezone(&Utc))
            .unwrap();
        let v = match field {
            GraphField::Cpu => e.cpu_time_percent,
            GraphField::Rss => e.memory.rss_kb as f64,
        };
        series.push((t, v));
    }
    (series, start, end)
}

fn write_multi_svg(stats: &[Stats], out: &Path, field: GraphField) {
    let mut data = Vec::new();
    let mut start_all: Option<DateTime<Utc>> = None;
    let mut end_all: Option<DateTime<Utc>> = None;
    let mut max_val = 0.0f64;
    for s in stats {
        if let Ok(entries) = read_log_entries(Path::new(&s.path)) {
            let (series, start, end) = collect_series(&entries, field);
            if series.is_empty() {
                continue;
            }
            start_all = Some(start_all.map_or(start, |cur| cur.min(start)));
            end_all = Some(end_all.map_or(end, |cur| cur.max(end)));
            for &(_, v) in &series {
                max_val = max_val.max(v);
            }
            let token = s.cmd.split_whitespace().next().unwrap_or("");
            let base = Path::new(token)
                .file_name()
                .map(|b| b.to_string_lossy().into_owned())
                .unwrap_or_else(|| token.to_string());
            let label = format!("{} {}", s.pid, base);
            data.push((label, series));
        }
    }
    if data.is_empty() {
        return;
    }
    if max_val <= 0.0 {
        max_val = 1.0;
    }
    let start = match start_all {
        Some(s) => s,
        None => return,
    };
    let end = end_all.unwrap_or(start + chrono::Duration::seconds(1));
    let root = SVGBackend::new(out, (600, 300)).into_drawing_area();
    if root.fill(&WHITE).is_err() {
        return;
    }
    let (y_desc, caption, scale) = match field {
        GraphField::Cpu => ("CPU %", "Top CPU usage", 1.0),
        GraphField::Rss => {
            if max_val >= 1024.0 * 1024.0 {
                ("RSS GB", "Top RSS (GB)", 1024.0 * 1024.0)
            } else {
                ("RSS MB", "Top RSS (MB)", 1024.0)
            }
        }
    };
    let y_max = (max_val / scale).max(1.0);
    let mut chart = match ChartBuilder::on(&root)
        .caption(caption, ("sans-serif", 20))
        .margin(5)
        .x_label_area_size(40)
        .y_label_area_size(40)
        .build_cartesian_2d(start..end, 0f64..y_max)
    {
        Ok(c) => c,
        Err(_) => return,
    };
    if chart
        .configure_mesh()
        .x_desc("time (UTC)")
        .y_desc(y_desc)
        .x_labels(5)
        .y_labels(5)
        .x_label_formatter(&|dt| dt.format("%H:%M:%S").to_string())
        .draw()
        .is_err()
    {
        return;
    }
    for (i, (label, series)) in data.into_iter().enumerate() {
        let color = Palette99::pick(i).mix(0.9);
        if chart
            .draw_series(LineSeries::new(
                series.into_iter().map(|(x, v)| (x, v / scale)),
                &color,
            ))
            .map(|l| {
                l.label(label).legend(move |(x, y)| {
                    PathElement::new(vec![(x, y), (x + 20, y)], color.clone())
                })
            })
            .is_err()
        {
            return;
        }
    }
    let _ = chart.configure_series_labels().border_style(&BLACK).draw();
    let _ = root.present();
}

fn write_chrome_trace(entries: &[LogEntry], out: &Path) -> io::Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut sorted: Vec<&LogEntry> = entries.iter().collect();
    sorted.sort_by_key(|e| e.timestamp.clone());
    let mut events = Vec::new();
    use std::collections::HashMap;
    let mut active: HashMap<(u32, usize), (String, serde_json::Value, i64, u32)> = HashMap::new();

    fn handle_frames(
        tid: u32,
        frames: &[&Frame],
        pid: u32,
        ts: i64,
        active: &mut HashMap<(u32, usize), (String, serde_json::Value, i64, u32)>,
        events: &mut Vec<serde_json::Value>,
    ) {
        if frames.is_empty() {
            return;
        }

        // handle existing events beyond current depth
        let mut depth = frames.len();
        loop {
            let key = (tid, depth);
            if let Some((name, args, start, pid_saved)) = active.remove(&key) {
                let dur = ts - start;
                events.push(json!({
                    "name": name,
                    "ph": "X",
                    "pid": pid_saved,
                    "tid": tid,
                    "ts": start,
                    "dur": if dur <= 0 { 1 } else { dur },
                    "args": args,
                }));
                depth += 1;
            } else {
                break;
            }
        }

        for (idx, frame) in frames.iter().enumerate() {
            let name = if let Some(f) = &frame.func {
                f.clone()
            } else if let Some(a) = frame.addr {
                format!("{:#x}", a)
            } else {
                "?".to_string()
            };
            let args = json!({
                "addr": frame.addr,
                "file": frame.file,
                "line": frame.line,
            });
            let key = (tid, idx);
            match active.get_mut(&key) {
                Some((cur, cur_args, _start, _pid)) if cur == &name => {
                    *cur_args = args;
                }
                Some((cur, cur_args, start, pid_saved)) => {
                    let dur = ts - *start;
                    events.push(json!({
                        "name": cur,
                        "ph": "X",
                        "pid": *pid_saved,
                        "tid": tid,
                        "ts": *start,
                        "dur": if dur <= 0 { 1 } else { dur },
                        "args": cur_args.clone(),
                    }));
                    *cur = name;
                    *cur_args = args;
                    *start = ts;
                    *pid_saved = pid;
                }
                None => {
                    active.insert(key, (name, args, ts, pid));
                }
            }
        }
    }

    for (i, e) in sorted.iter().enumerate() {
        if e.threads.is_empty() {
            continue;
        }
        let dt = chrono::DateTime::parse_from_rfc3339(&e.timestamp)
            .map(|t| t.with_timezone(&Utc))
            .map_err(|er| io::Error::new(io::ErrorKind::InvalidData, er))?;
        let ts = dt.timestamp_micros();

        for t in &e.threads {
            if let Some(st) = &t.stacktrace {
                let frames: Vec<&Frame> = st.iter().collect();
                handle_frames(t.tid << 1, &frames, e.pid, ts, &mut active, &mut events);
            }
            if let Some(py) = &t.python_stacktrace {
                let frames: Vec<&Frame> = py.iter().collect();
                handle_frames(
                    (t.tid << 1) | 1,
                    &frames,
                    e.pid,
                    ts,
                    &mut active,
                    &mut events,
                );
            }
        }

        if i == sorted.len() - 1 {
            let final_ts = ts;
            for ((tid, _idx), (name, args, start, pid)) in active.drain() {
                let dur = final_ts - start;
                events.push(json!({
                    "name": name,
                    "ph": "X",
                    "pid": pid,
                    "tid": tid,
                    "ts": start,
                    "dur": if dur <= 0 { 1 } else { dur },
                    "args": args,
                }));
            }
        }
    }
    if events.is_empty() {
        return Ok(());
    }
    let obj = json!({ "traceEvents": events });
    fs::write(out, serde_json::to_vec(&obj)?)
}

fn write_graphs(entries: &[LogEntry], out_dir: &Path, pid: u32) {
    let cpu_path = out_dir.join(format!("{}_cpu.svg", pid));
    if let Err(e) = write_svg(entries, &cpu_path, GraphField::Cpu) {
        warn!("failed to write {}: {}", cpu_path.display(), e);
    }
    let rss_path = out_dir.join(format!("{}_rss.svg", pid));
    if let Err(e) = write_svg(entries, &rss_path, GraphField::Rss) {
        warn!("failed to write {}: {}", rss_path.display(), e);
    }
}

fn write_trace(entries: &[LogEntry], out_dir: &Path, pid: u32) -> bool {
    let path = out_dir.join(format!("{}_trace.json", pid));
    if let Err(e) = write_chrome_trace(entries, &path) {
        warn!("failed to write {}: {}", path.display(), e);
        return false;
    }
    path.exists()
}

fn truncate(s: &str, len: usize) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i >= len {
            out.push_str("...");
            break;
        }
        out.push(c);
    }
    out
}

fn render_single(s: &Stats, has_trace: bool) -> String {
    let mut out = String::new();
    out.push_str("<html><body>\n");
    out.push_str(&format!("<h1>Report for PID {}</h1>\n", s.pid));
    out.push_str(&format!("<p>Command: {}</p>\n", encode_text(&s.cmd)));
    out.push_str("<ul>\n");
    out.push_str(&format!("<li>Total runtime: {} sec</li>\n", s.runtime));
    out.push_str(&format!("<li>Total CPU time: {:.1} sec</li>\n", s.cpu));
    out.push_str(&format!("<li>Average CPU usage: {:.1}%</li>\n", s.avg_cpu));
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
    out.push_str(&format!(
        "<p>CPU usage<br><img src=\"{}_cpu.svg\" alt=\"CPU usage graph\" /></p>\n",
        s.pid
    ));
    out.push_str(&format!(
        "<p>RSS<br><img src=\"{}_rss.svg\" alt=\"RSS graph\" /></p>\n",
        s.pid
    ));
    if has_trace {
        out.push_str(&format!(
            "<p><a href=\"{}_trace.json\">Trace JSON</a></p>\n",
            s.pid
        ));
    }
    out.push_str("</body></html>\n");
    out
}

fn render_index(stats: &[Stats], link: bool) -> String {
    let mut out = String::new();
    out.push_str("<html><head><style>table,th,td{border:1px solid black;border-collapse:collapse;}pre{margin:0;}</style></head><body>\n");
    out.push_str("<p>CPU usage<br><img src=\"top_cpu.svg\" alt=\"Top CPU usage graph\" /></p>\n");
    out.push_str("<p>Peak RSS<br><img src=\"top_rss.svg\" alt=\"Top RSS graph\" /></p>\n");
    if let (Some(start), Some(end)) = (
        stats.iter().map(|s| s.start).min(),
        stats.iter().map(|s| s.end).max(),
    ) {
        out.push_str(&format!("<p>Start: {}</p>\n", start));
        out.push_str(&format!("<p>End: {}</p>\n", end));
    }
    out.push_str("<table>\n");
    out.push_str(
        "<tr><th>PID</th><th>Command</th><th>Total runtime</th><th>Total CPU time</th><th>Avg CPU (%)</th><th>Peak RSS</th></tr>\n",
    );
    for s in stats {
        let pid_cell = if link {
            format!("<a href=\"{}.html\">{}</a>", s.pid, s.pid)
        } else {
            s.pid.to_string()
        };
        let summary = truncate(&s.cmd, 30);
        let cmd_cell = format!(
            "<details><summary>{}</summary><pre>{}</pre></details>",
            encode_text(&summary),
            encode_text(&s.cmd)
        );
        out.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{:.1}</td><td>{:.1}</td><td>{}</td></tr>\n",
            pid_cell, cmd_cell, s.runtime, s.cpu, s.avg_cpu, s.peak_rss
        ));
    }
    out.push_str("</table></body></html>\n");
    out
}

fn report_file(path: &Path, out_dir: &Path) {
    match read_log_entries(path) {
        Ok(entries) => {
            if let Some(s) = calc_stats(path, &entries) {
                write_graphs(&entries, out_dir, s.pid);
                let has_trace = write_trace(&entries, out_dir, s.pid);
                let html = render_single(&s, has_trace);
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
    by_cpu.sort_by(|a, b| {
        let a_cpu = if a.avg_cpu <= 0.1 { 0.0 } else { a.avg_cpu };
        let b_cpu = if b.avg_cpu <= 0.1 { 0.0 } else { b.avg_cpu };
        b_cpu
            .partial_cmp(&a_cpu)
            .unwrap()
            .then_with(|| b.peak_rss.cmp(&a.peak_rss))
    });
    let mut by_rss = stats.clone();
    by_rss.sort_by_key(|s| std::cmp::Reverse(s.peak_rss));

    let cpu_top: Vec<_> = by_cpu.iter().take(top_cpu).cloned().collect();
    let rss_top: Vec<_> = by_rss.iter().take(top_rss).cloned().collect();

    let mut map: HashMap<String, Stats> = HashMap::new();
    for s in cpu_top.clone() {
        map.entry(s.path.clone()).or_insert(s);
    }
    for s in rss_top.clone() {
        map.entry(s.path.clone()).or_insert(s);
    }
    let mut selected: Vec<_> = map.into_values().collect();
    selected.sort_by(|a, b| {
        let a_cpu = if a.avg_cpu <= 0.1 { 0.0 } else { a.avg_cpu };
        let b_cpu = if b.avg_cpu <= 0.1 { 0.0 } else { b.avg_cpu };
        b_cpu
            .partial_cmp(&a_cpu)
            .unwrap()
            .then_with(|| b.peak_rss.cmp(&a.peak_rss))
    });

    write_multi_svg(&cpu_top, &out_dir.join("top_cpu.svg"), GraphField::Cpu);
    write_multi_svg(&rss_top, &out_dir.join("top_rss.svg"), GraphField::Rss);

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
                    write_graphs(&entries, out_dir, s.pid);
                    let has_trace = write_trace(&entries, out_dir, s.pid);
                    let html = render_single(&stats, has_trace);
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
