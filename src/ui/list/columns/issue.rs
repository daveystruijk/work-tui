use std::collections::HashSet;

use nucleo_matcher::{
    pattern::{Atom, AtomKind, CaseMatching, Normalization},
    Config, Matcher,
};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::theme::Theme;
use crate::ticket::Ticket;
use crate::ui::issue_type_icon;

use super::{highlight_spans, search_match_indices};

/// Collapse state for expandable issues (stories/epics).
#[derive(Debug, Clone, Copy)]
pub enum CollapseState {
    /// Not an expandable issue — show the regular type icon.
    None,
    /// Expandable and currently collapsed.
    Collapsed,
    /// Expandable and currently expanded.
    Expanded,
}

/// Render the "Issue" column for a ticket row.
///
/// Contains: indent + icon + key (with search highlight) + pending-import marker + summary (with search highlight).
/// For expandable issues, shows ▶/▼ instead of the type icon.
pub fn render(
    ticket: &Ticket,
    pending_import_keys: &HashSet<String>,
    search_filter: &str,
    depth: u8,
    collapse_state: CollapseState,
) -> Line<'static> {
    let issue = &ticket.issue;
    let issue_type = issue.issue_type().map(|ty| ty.name).unwrap_or_default();
    let summary = issue
        .summary()
        .unwrap_or_default()
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();

    let indent = "  ".repeat(depth as usize);

    let icon = match collapse_state {
        CollapseState::None => issue_type_icon(&issue_type),
        CollapseState::Collapsed => "▶",
        CollapseState::Expanded => "▼",
    };

    let is_searching = !search_filter.is_empty();
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
    let highlight_style = Style::default()
        .fg(Theme::SearchMatch)
        .add_modifier(Modifier::BOLD);

    let has_pending_import = pending_import_keys.contains(&issue.key);

    // Key field — highlight search matches
    let key_base_style = Style::default()
        .fg(Theme::Accent)
        .add_modifier(Modifier::BOLD);
    let key_highlight_style = Style::default()
        .fg(Theme::SearchMatch)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    let key_indices = if is_searching {
        search_match_indices(&issue.key, &search_atoms, &mut matcher)
    } else {
        Vec::new()
    };
    let key_spans = highlight_spans(
        &issue.key,
        &key_indices,
        key_base_style,
        key_highlight_style,
    );

    let mut spans = vec![Span::styled(
        format!("{indent}{icon} "),
        Style::default().fg(Theme::Muted),
    )];
    spans.extend(key_spans);
    if has_pending_import {
        spans.push(Span::styled(" *", Style::default().fg(Theme::Warning)));
    }

    // Summary field — highlight search matches
    let summary_base_style = Style::default().fg(Theme::Text);
    let summary_indices = if is_searching {
        search_match_indices(&summary, &search_atoms, &mut matcher)
    } else {
        Vec::new()
    };
    let summary_highlighted = highlight_spans(
        &summary,
        &summary_indices,
        summary_base_style,
        highlight_style,
    );
    spans.push(Span::styled(" ", summary_base_style));
    spans.extend(summary_highlighted);

    Line::from(spans)
}
