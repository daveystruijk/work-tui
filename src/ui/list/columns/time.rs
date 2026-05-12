use ratatui::{style::Style, text::Line};

use crate::theme::Theme;
use crate::ticket::Ticket;

/// Render the "◷" (time) column for a ticket row.
///
/// Shows the most recent PR activity as a relative timestamp.
pub fn render(ticket: &Ticket) -> Line<'static> {
    let Some(pr) = ticket.pr.as_ref() else {
        return Line::default();
    };
    let Some(timestamp) = pr.most_recent_activity() else {
        return Line::default();
    };
    let Some(label) = crate::utils::time::format_relative_time(timestamp) else {
        return Line::default();
    };
    let color = crate::utils::time::elapsed_since_iso(timestamp)
        .map(Theme::recency_color)
        .unwrap_or(Theme::Muted);
    Line::styled(label, Style::default().fg(color))
}
