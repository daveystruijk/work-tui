use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::theme::Theme;
use crate::ticket::Ticket;

/// Render the "Repo" column for a ticket row.
pub fn render(ticket: &Ticket) -> Line<'static> {
    let is_active = ticket.active_branch.is_some();
    let repos = ticket
        .repos
        .iter()
        .map(|repo| repo.label.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    if is_active {
        Line::from(vec![
            Span::styled("⎇ ", Style::default().fg(Theme::Accent)),
            Span::styled(repos, Style::default().fg(Theme::Accent)),
        ])
    } else {
        Line::from(vec![Span::styled(
            repos,
            Style::default().fg(Theme::AccentSoft),
        )])
    }
}
