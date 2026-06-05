use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Log a hook decision to `cc-yes.log` in the plugin directory.
pub fn log_decision(
    tool_name: &str,
    command: &str,
    decision: &str,
    reason: &str,
) {
    let plugin_root = std::env::var("CLAUDE_PLUGIN_ROOT")
        .unwrap_or_else(|_| ".".to_string());
    let log_path = PathBuf::from(&plugin_root).join("cc-yes.log");

    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let timestamp = now_iso();
    let line = format!(
        "[{}] {} | {} | {} | {}\n",
        timestamp, tool_name, decision, command, reason
    );

    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = file.write_all(line.as_bytes());
    }
}

fn now_iso() -> String {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => {
            let secs = d.as_secs();
            // Simple UTC timestamp (works until 2100)
            let day_secs = secs % 86400;
            let days = secs / 86400;
            // Days since 1970-01-01 to year/month/day
            let (y, m, d) = days_to_ymd(days);
            let h = day_secs / 3600;
            let min = (day_secs % 3600) / 60;
            let s = day_secs % 60;
            format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, h, min, s)
        }
        Err(_) => "unknown".to_string(),
    }
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut y = 1970u64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        y += 1;
    }
    let months_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 1;
    for &md in &months_days {
        if days < md {
            break;
        }
        days -= md;
        m += 1;
    }
    (y, m, days + 1)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}
