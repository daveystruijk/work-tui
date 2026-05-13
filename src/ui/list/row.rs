use std::collections::{HashMap, HashSet};

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::app::InlineNewView;
use crate::theme::Theme;
use crate::ticket::{Ticket, TicketStore};
use crate::ui::{CellMap, SPINNER_FRAMES};

use super::columns;
use super::columns::issue::CollapseState;
use super::UiAnimationView;

/// Read-only shared state passed to ListView for rendering.
pub struct ListRenderContext<'a> {
    pub ticket_store: &'a TicketStore,
    pub check_durations: &'a HashMap<String, u64>,
    pub animation: &'a UiAnimationView,
    pub inline_new: Option<&'a InlineNewView>,
    pub search_filter: &'a str,
}

/// Render a ticket (issue) row by assembling all column renderers.
pub fn issue_row(
    ctx: &ListRenderContext,
    pending_import_keys: &HashSet<String>,
    ticket: &Ticket,
    depth: u8,
    collapse_state: CollapseState,
) -> (CellMap<'static>, Style) {
    let row_style = Style::default().fg(Theme::Text);
    let is_group_header = !matches!(collapse_state, CollapseState::None);

    let issue_cell = columns::issue::render(
        ticket,
        pending_import_keys,
        ctx.search_filter,
        depth,
        collapse_state,
    );
    let cells = if is_group_header {
        HashMap::from([("Issue", issue_cell)])
    } else {
        let status_cell = columns::status::render(ticket, ctx.search_filter);
        let dev_cell = columns::dev::render(ticket, ctx.search_filter);
        let time_cell = columns::time::render(ticket);
        let pr_cell = columns::pr::render(ticket);
        let ci_cell = columns::ci::render(ticket, ctx.animation.spinner_tick, ctx.check_durations);
        let repo_cell = columns::repo::render(ticket);
        HashMap::from([
            ("Issue", issue_cell),
            ("Status", status_cell),
            ("Dev", dev_cell),
            ("◷", time_cell),
            ("PR", pr_cell),
            ("CI", ci_cell),
            ("Repo", repo_cell),
        ])
    };

    (cells, row_style)
}

/// Render a section header row (BOARD / BACKLOG).
pub fn section_header_row(
    section: &crate::app::ListSection,
    count: usize,
) -> (CellMap<'static>, Style) {
    let row_style = Style::default().fg(Theme::Muted).bg(Theme::SidebarBg);
    let label = match section {
        crate::app::ListSection::Board => "BOARD",
        crate::app::ListSection::Backlog => "BACKLOG",
    };
    let label_color = match label {
        "BOARD" | "BACKLOG" => Theme::Muted,
        _ => Theme::AccentSoft,
    };
    let header_style = Style::default()
        .fg(label_color)
        .bg(Theme::SidebarBg)
        .add_modifier(Modifier::BOLD);
    let issue_word = if count == 1 { "issue" } else { "issues" };
    let cells = HashMap::from([(
        "Issue",
        Line::from(vec![Span::styled(
            format!("{label} ({count} {issue_word})"),
            header_style,
        )]),
    )]);
    (cells, row_style)
}

/// Render an inline-new-issue placeholder row.
pub fn inline_new_row(state: Option<&InlineNewView>, depth: u8) -> (CellMap<'static>, Style) {
    let row_style = Style::default().fg(Theme::Text);

    let summary_text = state.map(|s| s.summary.as_str()).unwrap_or("");
    let prefix = if depth > 0 {
        "  ".repeat(depth as usize)
    } else {
        String::new()
    };

    let cells = HashMap::from([(
        "Issue",
        Line::from(vec![
            Span::styled(format!("{prefix}◦ "), Style::default().fg(Theme::Muted)),
            Span::styled(
                "NEW",
                Style::default()
                    .fg(Theme::Warning)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default()),
            Span::styled(summary_text.to_string(), Style::default().fg(Theme::Text)),
            Span::styled(
                "▏".to_string(),
                Style::default()
                    .fg(Theme::Accent)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ]),
    )]);
    (cells, row_style)
}

/// Render a loading spinner row.
pub fn loading_row(spinner_tick: usize, depth: u8) -> (CellMap<'static>, Style) {
    let row_style = Style::default().fg(Theme::Muted);
    let indent = "  ".repeat(depth as usize);
    let spinner = SPINNER_FRAMES[spinner_tick % SPINNER_FRAMES.len()];
    let cells = HashMap::from([(
        "Issue",
        Line::styled(
            format!("{indent}{spinner} Loading..."),
            Style::default().fg(Theme::Muted),
        ),
    )]);
    (cells, row_style)
}

/// Render an empty-children placeholder row.
pub fn empty_row(depth: u8) -> (CellMap<'static>, Style) {
    let row_style = Style::default().fg(Theme::Muted);
    let indent = "  ".repeat(depth as usize);
    let cells = HashMap::from([(
        "Issue",
        Line::styled(
            format!("{indent}No issues"),
            Style::default().fg(Theme::Muted),
        ),
    )]);
    (cells, row_style)
}
