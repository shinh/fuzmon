use clap::{Parser, Subcommand};
use serde::Deserialize;
use std::fs;
use toml::Value;

#[derive(Parser)]
#[command(name = "fuzmon")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run the monitor
    Run(RunArgs),
    /// Dump logs
    Dump(DumpArgs),
}

#[derive(Parser, Clone)]
pub struct DumpArgs {
    /// Path to log file or directory
    pub path: String,
}

#[derive(Parser, Default, Clone)]
pub struct RunArgs {
    /// PID to trace
    #[arg(short, long)]
    pub pid: Option<i32>,
    /// Path to configuration file
    #[arg(short = 'c', long)]
    pub config: Option<String>,
    /// User name filter
    #[arg(long)]
    pub target_user: Option<String>,
    /// Output directory for logs
    #[arg(short = 'o', long)]
    pub output: Option<String>,
    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Default, Deserialize)]
pub struct FilterConfig {
    #[serde(default)]
    pub target_user: Option<String>,
    #[serde(default)]
    pub ignore_process_name: Option<Vec<String>>,
}

#[derive(Default, Deserialize)]
pub struct OutputConfig {
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub compress: Option<bool>,
}

#[derive(Default, Deserialize)]
pub struct MonitorConfig {
    #[serde(default)]
    pub interval_sec: Option<u64>,
    #[serde(default)]
    pub cpu_time_jiffies_threshold: Option<u64>,
}

#[derive(Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub filter: FilterConfig,
    #[serde(default)]
    pub output: OutputConfig,
    #[serde(default)]
    pub monitor: MonitorConfig,
}

pub fn load_config(path: &str) -> Result<Config, String> {
    let data = fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {}", path, e))?;
    let value: Value = data
        .parse()
        .map_err(|e| format!("failed to parse {}: {}", path, e))?;
    value
        .try_into()
        .map_err(|e| format!("failed to parse {}: {}", path, e))
}

pub fn uid_from_name(name: &str) -> Option<u32> {
    let passwd = fs::read_to_string("/etc/passwd").ok()?;
    for line in passwd.lines() {
        let mut parts = line.split(':');
        if let (Some(user), Some(_), Some(uid_str)) = (parts.next(), parts.next(), parts.next()) {
            if user == name {
                if let Ok(uid) = uid_str.parse::<u32>() {
                    return Some(uid);
                }
            }
        }
    }
    None
}

pub fn merge_config(mut cfg: Config, args: &RunArgs) -> Config {
    if let Some(ref u) = args.target_user {
        cfg.filter.target_user = Some(u.clone());
    }
    if let Some(ref p) = args.output {
        cfg.output.path = Some(p.clone());
    }
    if cfg.output.path.is_none() {
        cfg.output.path = Some("/tmp/fuzmon".into());
    }
    if cfg.output.compress.is_none() {
        cfg.output.compress = Some(true);
    }
    if cfg.monitor.cpu_time_jiffies_threshold.is_none() {
        cfg.monitor.cpu_time_jiffies_threshold = Some(1);
    }
    cfg
}

pub fn parse_cli() -> Cli {
    Cli::parse()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::NamedTempFile;

    #[test]
    fn load_example_config() {
        let cfg = load_config("ai_docs/example_config.toml").expect("load config");
        assert_eq!(cfg.output.format.as_deref(), Some("json"));
        assert_eq!(cfg.output.path.as_deref(), Some("/var/log/fuzmon/"));
        assert_eq!(cfg.output.compress, Some(true));
        assert_eq!(cfg.monitor.interval_sec, Some(60));
        assert_eq!(cfg.monitor.cpu_time_jiffies_threshold, Some(1));
        assert_eq!(cfg.filter.target_user.as_deref(), Some("myname"));
    }

    #[test]
    fn cli_overrides_config() {
        let tmp = NamedTempFile::new().expect("tmp");
        fs::write(
            tmp.path(),
            "target_user = \"hoge\"\n[output]\npath = \"/tmp/a\"",
        )
        .unwrap();
        let cfg = load_config(tmp.path().to_str().unwrap()).expect("load config");
        let args = RunArgs {
            target_user: Some("foo".into()),
            output: Some("/tmp/b".into()),
            ..Default::default()
        };
        let merged = merge_config(cfg, &args);
        assert_eq!(merged.filter.target_user.as_deref(), Some("foo"));
        assert_eq!(merged.output.path.as_deref(), Some("/tmp/b"));
    }

    #[test]
    fn default_output_path() {
        let cfg = Config::default();
        let args = RunArgs::default();
        let merged = merge_config(cfg, &args);
        assert_eq!(merged.output.path.as_deref(), Some("/tmp/fuzmon"));
        assert_eq!(merged.output.compress, Some(true));
        assert_eq!(merged.monitor.cpu_time_jiffies_threshold, Some(1));
    }

    #[test]
    fn invalid_config_returns_error() {
        let tmp = NamedTempFile::new().expect("tmp");
        fs::write(tmp.path(), "[filter]\nignore_process_name = false").unwrap();
        let err = load_config(tmp.path().to_str().unwrap()).err().unwrap();
        assert!(err.contains("ignore_process_name"));
        assert!(err.contains("invalid type"));
    }
}
