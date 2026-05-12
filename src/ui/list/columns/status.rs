use nucleo_matcher::{
    pattern::{Atom, AtomKind, CaseMatching, Normalization},
    Config, Matcher,
};
use ratatui::{
    style::{Modifier, Style},
    text::Line,
};

use crate::theme::Theme;
use crate::ticket::Ticket;
use crate::ui::status_color;

use super::{highlight_spans, search_match_indices};

/// Render the "Status" column for a ticket row.
pub fn render(ticket: &Ticket, search_filter: &str) -> Line<'static> {
    let status_name = ticket.issue.status().map(|s| s.name).unwrap_or_default();
    let status_style = status_color(&status_name);
    let is_searching = !search_filter.is_empty();

    let highlight_style = Style::default()
        .fg(Theme::SearchMatch)
        .add_modifier(Modifier::BOLD);

    let status_indices = if is_searching {
        let mut matcher = Matcher::new(Config::DEFAULT);
        let search_atoms: Vec<Atom> = search_filter
            .split_whitespace()
            .map(|word| {
                Atom::new(
                    word,
                    CaseMatching::Ignore,
                    Normalization::Smart,
                    AtomKind::Substring,
                    false,
                )
            })
            .collect();
        search_match_indices(&status_name, &search_atoms, &mut matcher)
    } else {
        Vec::new()
    };

    Line::from(highlight_spans(
        &status_name,
        &status_indices,
        status_style,
        highlight_style,
    ))
}
