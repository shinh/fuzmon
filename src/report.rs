use chrono::Utc;
use log::warn;
use std::path::Path;

use crate::log::read_log_entries;
use html_escape::encode_text;

pub fn report(path: &str) {
    let file_path = Path::new(path);
    match read_log_entries(file_path) {
        Ok(entries) => {
            if entries.is_empty() {
                println!("<p>No entries</p>");
                return;
            }
            let mut sorted = entries;
            sorted.sort_by_key(|e| e.timestamp.clone());
            let first = &sorted[0];
            let pid = first.pid;
            let cmd = first.cmdline.clone().unwrap_or_else(|| "(unknown)".into());
            let env = first.env.clone();
            let start = chrono::DateTime::parse_from_rfc3339(&first.timestamp)
                .map(|t| t.with_timezone(&Utc))
                .unwrap();
            let end = chrono::DateTime::parse_from_rfc3339(&sorted.last().unwrap().timestamp)
                .map(|t| t.with_timezone(&Utc))
                .unwrap();
            let total_time = (end - start).num_seconds();
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
            println!("<html><body>");
            println!("<h1>Report for PID {}</h1>", pid);
            println!("<p>Command: {}</p>", encode_text(&cmd));
            println!("<ul>");
            println!("<li>Total runtime: {} sec</li>", total_time);
            println!("<li>Total CPU time: {:.1} sec</li>", cpu);
            println!("<li>Peak RSS: {} KB</li>", peak_rss);
            println!("</ul>");
            if let Some(e) = env {
                if !e.is_empty() {
                    println!(
                        "<details><summary>Environment</summary><pre>{}</pre></details>",
                        encode_text(&e)
                    );
                }
            } else {
                println!("<p>Environment: unknown</p>");
            }
            println!("</body></html>");
        }
        Err(e) => warn!("failed to read {}: {}", file_path.display(), e),
    }
}
