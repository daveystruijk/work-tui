use chrono::Utc;

/// Compute duration in seconds between two ISO 8601 timestamps.
pub fn parse_duration_secs(start: &str, end: &str) -> Option<u64> {
    let s = start.parse::<chrono::DateTime<chrono::Utc>>().ok()?;
    let e = end.parse::<chrono::DateTime<chrono::Utc>>().ok()?;
    Some(e.signed_duration_since(s).num_seconds().unsigned_abs())
}

/// Seconds elapsed since an ISO 8601 timestamp.
pub fn elapsed_since_iso(ts: &str) -> Option<u64> {
    let started = ts.parse::<chrono::DateTime<chrono::Utc>>().ok()?;
    Some(
        Utc::now()
            .signed_duration_since(started)
            .num_seconds()
            .unsigned_abs(),
    )
}

/// Format seconds as a human-readable duration (e.g. "2m", "1m30s").
pub fn format_duration(secs: u64) -> String {
    if secs < 60 {
        return format!("{secs}s");
    }
    let m = secs / 60;
    let s = secs % 60;
    if s == 0 {
        format!("{m}m")
    } else {
        format!("{m}m{s:02}s")
    }
}
