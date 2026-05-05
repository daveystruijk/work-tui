use chrono::Utc;

/// Compute duration in seconds between two ISO 8601 timestamps.
pub fn parse_duration_secs(start: &str, end: &str) -> Option<u64> {
    let s = start.parse::<chrono::DateTime<chrono::Utc>>().ok()?;
    let e = end.parse::<chrono::DateTime<chrono::Utc>>().ok()?;
    Some(e.signed_duration_since(s).num_seconds().unsigned_abs())
}

/// Seconds elapsed since an ISO 8601 timestamp.
pub fn elapsed_since_iso(ts: &str) -> Option<u64> {
    let started = parse_timestamp(ts)?;
    Some(
        Utc::now()
            .signed_duration_since(started)
            .num_seconds()
            .unsigned_abs(),
    )
}

/// Parse an ISO 8601 timestamp, handling both RFC 3339 (`Z` suffix) and
/// Jira-style offsets (`+0000` without colon).
fn parse_timestamp(ts: &str) -> Option<chrono::DateTime<Utc>> {
    ts.parse::<chrono::DateTime<Utc>>().ok().or_else(|| {
        chrono::DateTime::parse_from_str(ts, "%Y-%m-%dT%H:%M:%S%.3f%z")
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    })
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

/// Format an ISO 8601 timestamp as a short relative time (e.g. "10s", "45m", "2h", "3d").
pub fn format_relative_time(timestamp: &str) -> Option<String> {
    let secs = elapsed_since_iso(timestamp)?;
    Some(format_elapsed_short(secs))
}

/// Format elapsed seconds as a compact relative label.
pub fn format_elapsed_short(secs: u64) -> String {
    if secs < 60 {
        return format!("{secs}s");
    }
    let minutes = secs / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    let days = hours / 24;
    format!("{days}d")
}
