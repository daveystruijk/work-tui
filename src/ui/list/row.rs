use std::collections::{HashMap, HashSet};

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::apis::github::PrInfo;
use crate::apis::jira::Issue;
use crate::app::InlineNewView;
use crate::theme::Theme;
use crate::ticket::Ticket;
use crate::ui::{CellMap, SPINNER_FRAMES};

use super::columns;
use super::UiAnimationView;

/// Read-only shared state passed to ListView for rendering.
pub struct ListRenderContext<'a> {
    pub issues: &'a [Issue],
    pub story_children: &'a HashMap<String, Vec<Issue>>,
    pub ticket_store: &'a crate::ticket::TicketStore,
    pub github_prs: &'a HashMap<String, PrInfo>,
    pub active_branches: &'a HashMap<String, String>,
    pub check_durations: &'a HashMap<String, u64>,
    pub animation: &'a UiAnimationView,
    pub inline_new: Option<&'a InlineNewView>,
    pub search_filter: &'a str,
}

/// Look up an issue by key across all issue sources.
pub fn find_issue_by_key<'a>(
    issues: &'a [Issue],
    story_children: &'a HashMap<String, Vec<Issue>>,
    key: &str,
) -> Option<&'a Issue> {
    issues.iter().find(|issue| issue.key == key).or_else(|| {
        story_children
            .values()
            .flat_map(|children| children.iter())
            .find(|issue| issue.key == key)
    })
}

/// Render a ticket (issue) row by assembling all column renderers.
pub fn issue_row(
    ctx: &ListRenderContext,
    pending_import_keys: &HashSet<String>,
    ticket: &Ticket,
    _idx: usize,
    depth: u8,
) -> (CellMap<'static>, Style) {
    let row_style = Style::default().fg(Theme::Text);

    let cells = HashMap::from([
        (
            "Issue",
            columns::issue::render(ticket, pending_import_keys, ctx.search_filter, depth),
        ),
        ("Status", columns::status::render(ticket, ctx.search_filter)),
        ("Dev", columns::dev::render(ticket, ctx.search_filter)),
        ("◷", columns::time::render(ticket)),
        ("PR", columns::pr::render(ticket)),
        (
            "CI",
            columns::ci::render(ticket, ctx.animation.spinner_tick, ctx.check_durations),
        ),
        ("Repo", columns::repo::render(ticket)),
    ]);

    (cells, row_style)
}

/// Render a story/epic group header row.
pub fn story_header_row(
    key: &str,
    summary: &str,
    _idx: usize,
    collapsed: bool,
    depth: u8,
    has_pending_import: bool,
) -> (CellMap<'static>, Style) {
    let row_style = Style::default().fg(Theme::Muted);

    let first_line = summary.lines().next().unwrap_or_default().to_string();
    let icon = if collapsed { "▶" } else { "▼" };
    let indent = "  ".repeat(depth as usize);
    let header_style = Style::default()
        .fg(Theme::AccentSoft)
        .add_modifier(Modifier::BOLD);

    let mut summary_spans = vec![Span::styled(
        format!("{}{} {}", indent, icon, key),
        header_style,
    )];
    if has_pending_import {
        summary_spans.push(Span::styled(" *", Style::default().fg(Theme::Warning)));
    }
    summary_spans.push(Span::styled(format!(" {first_line}"), header_style));
    let summary_line = Line::from(summary_spans);

    let cells = HashMap::from([("Issue", summary_line)]);
    (cells, row_style)
}

/// Render a section header row (BOARD / BACKLOG).
pub fn section_header_row(
    section: &crate::app::ListSection,
    count: usize,
    _width: u16,
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
pub fn inline_new_row(
    state: Option<&InlineNewView>,
    _idx: usize,
    depth: u8,
) -> (CellMap<'static>, Style) {
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
pub fn loading_row(spinner_tick: usize, _idx: usize, depth: u8) -> (CellMap<'static>, Style) {
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
pub fn empty_row(_idx: usize, depth: u8) -> (CellMap<'static>, Style) {
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
