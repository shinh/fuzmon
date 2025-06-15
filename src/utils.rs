use chrono::Utc;

/// Returns current date as YYYYMMDD string in UTC.
pub fn current_date_string() -> String {
    Utc::now().format("%Y%m%d").to_string()
}
