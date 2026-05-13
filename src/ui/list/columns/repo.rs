use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::theme::Theme;
use crate::ticket::Ticket;

/// Render the "Repo" column for a ticket row.
pub fn render(ticket: &Ticket) -> Line<'static> {
    let repos = ticket
        .repos
        .iter()
        .map(|repo| repo.label.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    let color = if ticket.active_branch.is_some() {
        Theme::Success
    } else {
        Theme::Muted
    };

    let mut spans = vec![Span::styled(repos, Style::default().fg(color))];

    if ticket.has_dirty_repo {
        spans.push(Span::styled("*", Style::default().fg(Theme::Warning)));
    }

    Line::from(spans)
}
