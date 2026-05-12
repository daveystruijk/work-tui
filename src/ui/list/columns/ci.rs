use std::collections::HashMap;

use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::apis::github::{CheckStatus, PrInfo};
use crate::theme::Theme;
use crate::ticket::Ticket;
use crate::ui::SPINNER_FRAMES;

/// Render the "CI" column for a ticket row.
pub fn render(
    ticket: &Ticket,
    spinner_tick: usize,
    check_durations: &HashMap<String, u64>,
) -> Line<'static> {
    let Some(pr) = ticket.pr.as_ref() else {
        return Line::default();
    };

    let mut ci_spans = Vec::new();
    for run in &pr.check_runs {
        let (icon, color) = match run.status {
            CheckStatus::Pass => ("✓", Theme::Success),
            CheckStatus::Fail => ("✗", Theme::Error),
            CheckStatus::Pending => ("●", Theme::Warning),
        };
        ci_spans.push(Span::styled(icon, Style::default().fg(color)));
    }
    if pr.checks == CheckStatus::Pending {
        let spinner = SPINNER_FRAMES[spinner_tick % SPINNER_FRAMES.len()];
        ci_spans.push(Span::styled(
            format!(" {spinner}"),
            Style::default().fg(Theme::Warning),
        ));
        if let Some(eta) = pr_eta(check_durations, pr) {
            ci_spans.push(Span::styled(
                format!(" {eta}"),
                Style::default().fg(Theme::Muted),
            ));
        }
    }

    Line::from(ci_spans)
}

/// Compute the ETA string for a PR's pending checks.
fn pr_eta(check_durations: &HashMap<String, u64>, pr: &PrInfo) -> Option<String> {
    let pending_runs: Vec<_> = pr
        .check_runs
        .iter()
        .filter(|r| r.status == CheckStatus::Pending)
        .collect();
    if pending_runs.is_empty() {
        return None;
    }
    let mut max_remaining: Option<u64> = None;
    for run in &pending_runs {
        let cache_key = format!("{}/{}", pr.repo_slug, run.name);
        let Some(&historical) = check_durations.get(&cache_key) else {
            continue;
        };
        let elapsed = run
            .started_at
            .as_deref()
            .and_then(crate::utils::time::elapsed_since_iso)
            .unwrap_or(0);
        let remaining = historical.saturating_sub(elapsed);
        max_remaining = Some(max_remaining.map_or(remaining, |cur: u64| cur.max(remaining)));
    }
    max_remaining.map(|r| format!("~{}", crate::utils::time::format_duration(r)))
}
