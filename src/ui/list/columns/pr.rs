use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::apis::github::{MergeableState, ReviewDecision};
use crate::theme::Theme;
use crate::ticket::Ticket;

/// Render the "PR" column for a ticket row.
pub fn render(ticket: &Ticket) -> Line<'static> {
    let Some(pr) = ticket.pr.as_ref() else {
        return Line::default();
    };

    let pr_color = if pr.is_draft {
        Theme::Muted
    } else if pr.state.eq_ignore_ascii_case("merged") {
        Theme::Accent
    } else {
        match &pr.review_decision {
            Some(ReviewDecision::Approved) => Theme::Success,
            Some(ReviewDecision::ChangesRequested) => Theme::Error,
            _ => Theme::Info,
        }
    };

    let mut pr_spans = vec![Span::styled(
        format!("#{}", pr.number),
        Style::default().fg(pr_color),
    )];
    if pr.mergeable == Some(MergeableState::Conflicting) {
        pr_spans.push(Span::styled("!", Style::default().fg(Theme::Error)));
    }

    Line::from(pr_spans)
}
