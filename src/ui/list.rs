use std::collections::HashMap;

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState},
    Frame,
};

use crate::app::{App, DisplayRow, InlineNewState};
use crate::github::CheckStatus;
use crate::jira::Issue;
use crate::theme::Theme;

use super::{
    centered_rect, help_bar, issue_type_icon, max_col_width, status_color, CellMap, COLUMNS,
    SPINNER_FRAMES,
};

pub fn render_list(app: &mut App, frame: &mut Frame) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(44)])
        .split(frame.area());

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(columns[0]);

    // Store visible list height for half-page scrolling
    app.list_area_height = main_chunks[0].height.saturating_sub(1);

    super::sidebar::render_sidebar(app, frame, columns[1]);

    // Build row data as (CellMap, Style) so we can measure before converting to Row.
    let row_data: Vec<(CellMap, Style)> = app
        .display_rows
        .iter()
        .enumerate()
        .map(|(row_idx, display_row)| match display_row {
            DisplayRow::StoryHeader { key, summary } => {
                let collapsed = app.collapsed_stories.contains(key);
                story_header_row(key, summary, row_idx, collapsed)
            }
            DisplayRow::Issue { index, depth } => {
                issue_row(app, &app.issues[*index], row_idx, *depth)
            }
            DisplayRow::InlineNew { depth } => {
                inline_new_row(app.inline_new.as_ref(), row_idx, *depth)
            }
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
        .with_offset(app.list_scroll_offset)
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
    frame.render_stateful_widget(table, main_chunks[0], &mut state);
    app.list_scroll_offset = state.offset();

    super::command_bar::render_command_bar(app, frame, main_chunks[1]);

    if app.label_picker_active() {
        render_label_picker_modal(app, frame);
    }
}

fn story_header_row(
    key: &str,
    summary: &str,
    _idx: usize,
    collapsed: bool,
) -> (CellMap<'static>, Style) {
    let row_style = Style::default().fg(Theme::Muted);

    let first_line = summary.lines().next().unwrap_or_default().to_string();
    let icon = if collapsed { "▶" } else { "▼" };
    let header_style = Style::default()
        .fg(Theme::AccentSoft)
        .add_modifier(Modifier::BOLD);

    let cells = HashMap::from([
        (
            "Key",
            Line::styled(format!("{} {}", icon, key), header_style),
        ),
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
    let prefix = if depth > 0 { "  ↳ " } else { "" };

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

    let key_prefix = if depth > 0 { "  ↳ " } else { "" };

    let mut cells = HashMap::from([
        (
            "Key",
            Line::styled(
                format!("{}{}", key_prefix, issue.key),
                Style::default()
                    .fg(Theme::Accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ),
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
        cells.insert(
            "PR",
            Line::styled(format!("#{}", pr.number), Style::default().fg(pr_color)),
        );

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
            let spinner = SPINNER_FRAMES[app.spinner_tick % SPINNER_FRAMES.len()];
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

fn render_label_picker_modal(app: &App, frame: &mut Frame) {
    let Some(picker) = &app.label_picker else {
        return;
    };
    let area = centered_rect(60, 70, frame.area());
    frame.render_widget(Clear, area);

    let popup = Block::bordered()
        .title(Span::styled(
            " Add repo label ",
            Style::default()
                .fg(Theme::Accent)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(Theme::Surface))
        .border_style(Style::default().fg(Theme::Accent));
    let inner = popup.inner(area);
    frame.render_widget(popup, area);

    let filtered = app.filtered_repo_entries();

    let items: Vec<ListItem> = if filtered.is_empty() {
        let msg = if app.repo_entries.is_empty() {
            "No repositories available"
        } else {
            "No matches"
        };
        vec![ListItem::new(Line::from(vec![Span::styled(
            msg,
            Style::default().fg(Theme::Muted),
        )]))]
    } else {
        filtered
            .iter()
            .map(|entry| {
                let path = entry.path.display().to_string();
                ListItem::new(vec![
                    Line::from(vec![Span::styled(
                        entry.label.clone(),
                        Style::default()
                            .fg(Theme::Text)
                            .add_modifier(Modifier::BOLD),
                    )]),
                    Line::from(vec![Span::styled(path, Style::default().fg(Theme::Muted))]),
                ])
            })
            .collect()
    };

    let mut state = ListState::default();
    if !filtered.is_empty() {
        state.select(Some(picker.selected));
    }

    let list = List::new(items).highlight_style(
        Style::default()
            .fg(Theme::Panel)
            .bg(Theme::AccentSoft)
            .add_modifier(Modifier::BOLD),
    );

    let filter_display = format!(
        " {} ",
        if picker.filter.is_empty() {
            "Type to filter...".to_string()
        } else {
            picker.filter.clone()
        }
    );
    let filter_style = if picker.filter.is_empty() {
        Style::default().fg(Theme::Muted)
    } else {
        Style::default().fg(Theme::Text)
    };

    let modal_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("/ ", Style::default().fg(Theme::Accent)),
            Span::styled(filter_display, filter_style),
        ]))
        .block(
            Block::bordered()
                .style(Style::default().bg(Theme::Surface))
                .border_style(Style::default().fg(Theme::Accent)),
        ),
        modal_layout[0],
    );

    frame.render_stateful_widget(list, modal_layout[1], &mut state);

    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "↑/↓:Move  •  Enter:Add  •  Esc:Cancel",
            Style::default().fg(Theme::Muted),
        )]))
        .alignment(Alignment::Center)
        .style(Style::default().bg(Theme::Surface)),
        modal_layout[2],
    );
}

pub fn render_new(app: &App, frame: &mut Frame) {
    let Some(form) = &app.new_form else {
        frame.render_widget(Paragraph::new("No form"), frame.area());
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(frame.area());

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("✦ ", Style::default().fg(Theme::AccentSoft)),
            Span::styled(
                "New issue",
                Style::default()
                    .fg(Theme::Text)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!(" {} ", form.project_key),
                Style::default()
                    .fg(Theme::Panel)
                    .bg(Theme::Accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .block(
            Block::bordered()
                .title(Span::styled(
                    " Compose ",
                    Style::default()
                        .fg(Theme::Accent)
                        .add_modifier(Modifier::BOLD),
                ))
                .style(Style::default().bg(Theme::Panel))
                .border_style(Style::default().fg(Theme::AccentSoft)),
        )
        .style(Style::default().bg(Theme::Panel)),
        chunks[0],
    );

    let form_block = Block::bordered()
        .title(Span::styled(
            " New issue details ",
            Style::default()
                .fg(Theme::Text)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(Theme::Panel))
        .border_style(Style::default().fg(Theme::Accent));
    let form_area = form_block.inner(chunks[1]);
    frame.render_widget(form_block, chunks[1]);

    let fields = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(3),
        ])
        .split(form_area);

    render_form_field(
        frame,
        fields[0],
        "Type",
        &format!(
            "‹ {} ›   {}/{}",
            form.issue_types
                .get(form.issue_type_idx)
                .map(|ty| ty.name.as_str())
                .unwrap_or(""),
            form.issue_type_idx + 1,
            form.issue_types.len(),
        ),
        form.active_field == 0,
    );
    render_form_field(
        frame,
        fields[1],
        "Summary",
        &form.summary,
        form.active_field == 1,
    );
    render_form_field(
        frame,
        fields[2],
        "Description",
        &form.description,
        form.active_field == 2,
    );

    frame.render_widget(
        help_bar("Esc:Cancel  Tab:Next field  Ctrl+S:Submit"),
        chunks[2],
    );
}

fn render_form_field(
    frame: &mut Frame,
    area: ratatui::layout::Rect,
    label: &str,
    value: &str,
    active: bool,
) {
    let block_style = if active {
        Style::default().fg(Theme::Accent).bg(Theme::Surface)
    } else {
        Style::default().fg(Theme::Muted).bg(Theme::Panel)
    };

    let label_style = if active {
        Style::default()
            .fg(Theme::AccentSoft)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Theme::Muted)
            .add_modifier(Modifier::BOLD)
    };

    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(vec![
                Span::styled(if active { "▌ " } else { "  " }, block_style),
                Span::styled(label, label_style),
            ]),
            Line::from(Span::styled(
                value.to_string(),
                Style::default().fg(Theme::Text),
            )),
        ]))
        .block(
            Block::bordered()
                .style(Style::default().bg(block_style.bg.unwrap_or(Theme::Panel)))
                .border_style(block_style),
        )
        .style(Style::default().bg(block_style.bg.unwrap_or(Theme::Panel))),
        area,
    );
}
