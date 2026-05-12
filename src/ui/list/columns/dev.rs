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

use super::{highlight_spans, search_match_indices};

/// Render the "Dev" column for a ticket row.
pub fn render(ticket: &Ticket, search_filter: &str) -> Line<'static> {
    let assignee = ticket
        .issue
        .assignee()
        .as_ref()
        .map(|u| {
            u.display_name
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string()
        })
        .unwrap_or_default();
    let dev_base_style = Style::default().fg(Theme::Muted);
    let is_searching = !search_filter.is_empty();

    let highlight_style = Style::default()
        .fg(Theme::SearchMatch)
        .add_modifier(Modifier::BOLD);

    let dev_indices = if is_searching {
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
        search_match_indices(&assignee, &search_atoms, &mut matcher)
    } else {
        Vec::new()
    };

    Line::from(highlight_spans(
        &assignee,
        &dev_indices,
        dev_base_style,
        highlight_style,
    ))
}
