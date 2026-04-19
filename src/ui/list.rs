use std::collections::{HashMap, HashSet};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::Constraint,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Cell, Row, Table, TableState},
    Frame,
};

use crate::actions::ActionMessage;
use crate::apis::{
    github::{CheckStatus, MergeableState},
    jira::Issue,
};
use crate::app::{App, DisplayRow, InlineNewState};
use crate::theme::Theme;

use super::{issue_type_icon, max_col_width, status_color, CellMap, COLUMNS, SPINNER_FRAMES};

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;

    use crate::fixtures::{render_to_string, selected_issue_app};
    use crate::ui::render;

    #[test]
    fn snapshots_list_view() {
        let mut app = selected_issue_app();
        let rendered = render_to_string(120, 16, |frame| {
            render(&mut app, frame);
        });

        assert_snapshot!("list_view", rendered);
    }
}

#[derive(Debug, Clone, Default)]
pub struct ListViewState {
    pub area_height: u16,
    pub scroll_offset: usize,
    pub loading_children: HashSet<String>,
}

impl ListViewState {
    pub fn handle_action_message(&mut self, msg: &ActionMessage) {
        match msg {
            ActionMessage::Issues(Ok(_)) => {
                self.loading_children.clear();
            }
            ActionMessage::ChildrenLoaded(parent_key, _) => {
                self.loading_children.remove(parent_key);
            }
            _ => {}
        }
    }

    pub fn start_loading_children(&mut self, parent_key: &str) {
        self.loading_children.insert(parent_key.to_string());
    }

}

pub fn render_list(app: &mut App, frame: &mut Frame, area: ratatui::layout::Rect) {
    // Store visible list height for half-page scrolling
    app.list_view.area_height = area.height.saturating_sub(1);

    // Build row data as (CellMap, Style) so we can measure before converting to Row.
    let row_data: Vec<(CellMap, Style)> = app
        .display_rows
        .iter()
        .enumerate()
        .map(|(row_idx, display_row)| match display_row {
            DisplayRow::StoryHeader {
                key,
                summary,
                depth,
            } => {
                let collapsed = app.collapsed_stories.contains(key);
                let has_pending_import = app.pending_import_keys.contains(key);
                story_header_row(key, summary, row_idx, collapsed, *depth, has_pending_import)
            }
            DisplayRow::Issue {
                index,
                depth,
                child_of,
            } => {
                let issue = match child_of {
                    Some(parent_key) => &app.story_children[parent_key][*index],
                    None => &app.issues[*index],
                };
                issue_row(app, issue, row_idx, *depth)
            }
            DisplayRow::InlineNew { depth } => {
                inline_new_row(app.inline_new.as_ref(), row_idx, *depth)
            }
            DisplayRow::Loading { depth } => {
                loading_row(app.animation.spinner_tick, row_idx, *depth)
            }
            DisplayRow::Empty { depth } => empty_row(row_idx, *depth),
        })
        .collect();

    let constraints = [
        Constraint::Length(max_col_width(&row_data, "Key").min(16)),
        Constraint::Min(10), // Summary (flex fill)
        Constraint::Length(max_col_width(&row_data, "PR").min(8)),
        Constraint::Length(max_col_width(&row_data, "CI").min(14)),
        Constraint::Length(max_col_width(&row_data, "Status").min(14)),
        Constraint::Length(max_col_width(&row_data, "Assignee").min(20)),
        Constraint::Length(max_col_width(&row_data, "Repo").min(24)),
    ];

    // Convert CellMaps to ordered Rows.
    let mut state = TableState::default()
        .with_offset(app.list_view.scroll_offset)
        .with_selected(Some(app.selected_index));

    let rows: Vec<Row> = row_data
        .into_iter()
        .map(|(mut cells, style)| {
            let ordered: Vec<Cell> = COLUMNS
                .iter()
                .map(|col| Cell::from(cells.remove(col).unwrap_or_default()))
                .collect();
            Row::new(ordered).style(style)
        })
        .collect();

    let table = Table::new(rows, constraints)
        .header(
            Row::new(COLUMNS.iter().copied())
                .style(
                    Style::default()
                        .fg(Theme::Muted)
                        .add_modifier(Modifier::BOLD),
                )
                .bottom_margin(0),
        )
        .column_spacing(2)
        .row_highlight_style(
            Style::default()
                .fg(Theme::Text)
                .bg(Theme::SurfaceAlt)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().style(Style::default().bg(Theme::Panel)));
    frame.render_stateful_widget(table, area, &mut state);
    app.list_view.scroll_offset = state.offset();

}

pub async fn handle_list(app: &mut App, key_event: KeyEvent) {
    let previous_was_g = matches!(
        app.previous_key,
        Some(KeyEvent { code: KeyCode::Char('g'), .. })
    );

    if key_event.modifiers.contains(KeyModifiers::CONTROL) {
        match key_event.code {
            KeyCode::Char('d') | KeyCode::Char('D') => {
                move_selection_by(app, app.list_view.area_height as isize / 2);
            }
            KeyCode::Char('u') | KeyCode::Char('U') => {
                move_selection_by(app, -(app.list_view.area_height as isize / 2));
            }
            _ => {}
        }
        return;
    }

    match key_event.code {
        KeyCode::Char(c) => {
            if previous_was_g && c == 'g' {
                app.selected_index = 0;
                adjust_scroll_offset(app);
                return;
            }

            match c {
                'b' => app.spawn_branch_diff(),
                'j' => move_selection_down(app),
                'k' => move_selection_up(app),
                'G' => move_selection_to_end(app),
                'g' => {}
                'p' => app.spawn_pick_up(),
                'o' => match app.open_selected_pr_in_browser().await {
                    Ok(_) => {}
                    Err(err) => app.status_bar.message = format!("{err}"),
                },
                't' => match app.open_selected_issue_in_browser().await {
                    Ok(_) => {}
                    Err(err) => app.status_bar.message = format!("Failed to open issue: {err}"),
                },
                'n' => {
                    app.start_inline_new();
                }
                'a' => app.open_label_picker(),
                'r' => {
                    app.loading = true;
                    app.spawn_refresh();
                }
                's' => app.spawn_toggle_story_type(),
                'f' => app.spawn_finish(),
                '/' => app.start_search(),
                'V' => app.spawn_approve_merge(),
                'c' => app.open_ci_log_popup(),
                'e' => app.spawn_openspec_propose(),
                'i' => app.open_import_tasks_popup(),
                'h' => {
                    app.collapse_story();
                }
                'l' => {
                    app.expand_story();
                }
                ' ' => {
                    app.toggle_story_collapse();
                }
                _ => {}
            }
        }
        KeyCode::Esc => {
            if !app.search_filter.is_empty() {
                app.cancel_search();
            }
        }
        KeyCode::Down => move_selection_down(app),
        KeyCode::Up => move_selection_up(app),
        _ => {}
    }
}

pub async fn handle_inline_new(app: &mut App, key_event: KeyEvent) {
    if key_event.modifiers.contains(KeyModifiers::CONTROL) {
        match key_event.code {
            KeyCode::Char('s') | KeyCode::Char('S') => {
                app.status_bar.message = "Creating issue...".to_string();
                app.spawn_submit_inline_new();
                return;
            }
            _ => {}
        }
    }

    match key_event.code {
        KeyCode::Esc => app.cancel_inline_new(),
        KeyCode::Enter => {
            app.status_bar.message = "Creating issue...".to_string();
            app.spawn_submit_inline_new();
        }
        KeyCode::Backspace => {
            if let Some(state) = app.inline_new.as_mut() {
                state.summary.pop();
            }
        }
        KeyCode::Char(c) => {
            if !key_event.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
                if let Some(state) = app.inline_new.as_mut() {
                    state.summary.push(c);
                }
            }
        }
        _ => {}
    }
}

pub fn handle_search(app: &mut App, key_event: KeyEvent) {
    match key_event.code {
        KeyCode::Esc => app.cancel_search(),
        KeyCode::Enter => app.confirm_search(),
        KeyCode::Backspace => app.search_backspace(),
        KeyCode::Char(c) => {
            if !key_event.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
                app.search_type_char(c);
            }
        }
        _ => {}
    }
}

pub const SCROLL_OFF: usize = 3;

pub fn adjust_scroll_offset(app: &mut App) {
    let height = app.list_view.area_height as usize;
    if height == 0 || app.display_rows.is_empty() {
        app.prefetch_selected_pr_detail();
        return;
    }

    let margin = SCROLL_OFF.min(height / 2);
    let selected = app.selected_index;
    let offset = app.list_view.scroll_offset;

    if selected < offset + margin {
        app.list_view.scroll_offset = selected.saturating_sub(margin);
    }

    if selected + margin >= offset + height {
        app.list_view.scroll_offset = (selected + margin + 1).saturating_sub(height);
    }

    let max_offset = app.display_rows.len().saturating_sub(height);
    app.list_view.scroll_offset = app.list_view.scroll_offset.min(max_offset);
    app.prefetch_selected_pr_detail();
}

pub fn move_selection_down(app: &mut App) {
    if app.display_rows.is_empty() {
        app.selected_index = 0;
        return;
    }
    let last = app.display_rows.len() - 1;
    if app.selected_index < last {
        app.selected_index += 1;
    }
    adjust_scroll_offset(app);
}

pub fn move_selection_up(app: &mut App) {
    if app.selected_index == 0 {
        return;
    }
    app.selected_index -= 1;
    adjust_scroll_offset(app);
}

pub fn move_selection_to_end(app: &mut App) {
    if app.display_rows.is_empty() {
        return;
    }
    app.selected_index = app.display_rows.len() - 1;
    adjust_scroll_offset(app);
}

pub fn move_selection_by(app: &mut App, delta: isize) {
    if app.display_rows.is_empty() {
        return;
    }
    let last = app.display_rows.len() - 1;
    let new_index = (app.selected_index as isize + delta).clamp(0, last as isize) as usize;
    app.selected_index = new_index;
    adjust_scroll_offset(app);
}

pub fn scroll_viewport(app: &mut App, delta: isize) {
    if app.display_rows.is_empty() {
        app.prefetch_selected_pr_detail();
        return;
    }
    let height = app.list_view.area_height as usize;
    let max_offset = app.display_rows.len().saturating_sub(height);
    let new_offset =
        (app.list_view.scroll_offset as isize + delta).clamp(0, max_offset as isize) as usize;
    app.list_view.scroll_offset = new_offset;

    let last = app.display_rows.len() - 1;
    if app.selected_index < new_offset {
        app.selected_index = new_offset;
    } else if app.selected_index >= new_offset + height {
        app.selected_index = (new_offset + height - 1).min(last);
    }
    app.prefetch_selected_pr_detail();
}

fn story_header_row(
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

    let key_line = if has_pending_import {
        Line::from(vec![
            Span::styled(format!("{}{} {}", indent, icon, key), header_style),
            Span::styled(" *", Style::default().fg(Theme::Warning)),
        ])
    } else {
        Line::styled(format!("{}{} {}", indent, icon, key), header_style)
    };

    let cells = HashMap::from([
        ("Key", key_line),
        (
            "Summary",
            Line::styled(format!("§ {}", first_line), header_style),
        ),
    ]);
    (cells, row_style)
}

fn inline_new_row(
    state: Option<&InlineNewState>,
    _idx: usize,
    depth: u8,
) -> (CellMap<'static>, Style) {
    let row_style = Style::default().fg(Theme::Text);

    let summary_text = state.map(|s| s.summary.as_str()).unwrap_or("");
    let prefix = if depth > 0 {
        format!("{}↳ ", "  ".repeat(depth as usize))
    } else {
        String::new()
    };

    let cells = HashMap::from([
        (
            "Key",
            Line::styled(
                format!("{prefix}NEW"),
                Style::default()
                    .fg(Theme::Warning)
                    .add_modifier(Modifier::BOLD),
            ),
        ),
        (
            "Summary",
            Line::from(vec![
                Span::styled("◦ ", Style::default().fg(Theme::Muted)),
                Span::styled(summary_text.to_string(), Style::default().fg(Theme::Text)),
                Span::styled(
                    "▏".to_string(),
                    Style::default()
                        .fg(Theme::Accent)
                        .add_modifier(Modifier::SLOW_BLINK),
                ),
            ]),
        ),
    ]);
    (cells, row_style)
}

fn loading_row(spinner_tick: usize, _idx: usize, depth: u8) -> (CellMap<'static>, Style) {
    let row_style = Style::default().fg(Theme::Muted);
    let indent = "  ".repeat(depth as usize);
    let spinner = SPINNER_FRAMES[spinner_tick % SPINNER_FRAMES.len()];
    let cells = HashMap::from([(
        "Summary",
        Line::styled(
            format!("{indent}{spinner} Loading..."),
            Style::default().fg(Theme::Muted),
        ),
    )]);
    (cells, row_style)
}

fn empty_row(_idx: usize, depth: u8) -> (CellMap<'static>, Style) {
    let row_style = Style::default().fg(Theme::Muted);
    let indent = "  ".repeat(depth as usize);
    let cells = HashMap::from([(
        "Summary",
        Line::styled(
            format!("{indent}No issues"),
            Style::default().fg(Theme::Muted),
        ),
    )]);
    (cells, row_style)
}

fn issue_row(app: &App, issue: &Issue, _idx: usize, depth: u8) -> (CellMap<'static>, Style) {
    let issue_type = issue.issue_type().map(|ty| ty.name).unwrap_or_default();
    let status_name = issue.status().map(|s| s.name).unwrap_or_default();
    let status_style = status_color(&status_name);
    let assignee = issue.assignee().map(|u| u.display_name).unwrap_or_default();
    let is_active = app.active_branches.contains_key(&issue.key);
    let repos = app
        .repo_matches(issue)
        .into_iter()
        .map(|entry| entry.label.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let summary = issue
        .summary()
        .unwrap_or_default()
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    let row_style = Style::default().fg(Theme::Text);

    let key_prefix = if depth > 0 {
        format!("{}↳ ", "  ".repeat(depth as usize))
    } else {
        String::new()
    };

    let has_pending_import = app.pending_import_keys.contains(&issue.key);
    let key_line = if has_pending_import {
        Line::from(vec![
            Span::styled(
                format!("{}{}", key_prefix, issue.key),
                Style::default()
                    .fg(Theme::Accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" *", Style::default().fg(Theme::Warning)),
        ])
    } else {
        Line::styled(
            format!("{}{}", key_prefix, issue.key),
            Style::default()
                .fg(Theme::Accent)
                .add_modifier(Modifier::BOLD),
        )
    };

    let mut cells = HashMap::from([
        ("Key", key_line),
        (
            "Summary",
            Line::styled(
                format!("{} {}", issue_type_icon(&issue_type), summary),
                Style::default().fg(Theme::Text),
            ),
        ),
        (
            "Status",
            Line::from(vec![
                Span::styled("●", status_style),
                Span::raw(" "),
                Span::styled(status_name, status_style),
            ]),
        ),
        (
            "Assignee",
            Line::styled(assignee, Style::default().fg(Theme::Muted)),
        ),
        (
            "Repo",
            Line::from(if is_active {
                vec![
                    Span::styled("⎇ ", Style::default().fg(Theme::Accent)),
                    Span::styled(repos, Style::default().fg(Theme::Accent)),
                ]
            } else {
                vec![Span::styled(repos, Style::default().fg(Theme::AccentSoft))]
            }),
        ),
    ]);

    if let Some(pr) = app.github_prs.get(&issue.key) {
        let pr_color = if pr.is_draft {
            Theme::Muted
        } else {
            Theme::Info
        };
        let mut pr_spans = vec![Span::styled(
            format!("#{}", pr.number),
            Style::default().fg(pr_color),
        )];
        if pr.mergeable == Some(MergeableState::Conflicting) {
            pr_spans.push(Span::styled(" !", Style::default().fg(Theme::Error)));
        }
        cells.insert("PR", Line::from(pr_spans));

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
            let spinner = SPINNER_FRAMES[app.animation.spinner_tick % SPINNER_FRAMES.len()];
            ci_spans.push(Span::styled(
                format!(" {spinner}"),
                Style::default().fg(Theme::Warning),
            ));
            if let Some(eta) = app.pr_eta(pr) {
                ci_spans.push(Span::styled(
                    format!(" {eta}"),
                    Style::default().fg(Theme::Muted),
                ));
            }
        }
        cells.insert("CI", Line::from(ci_spans));
    }

    (cells, row_style)
}
