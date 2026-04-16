use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Cell, Clear, List, ListItem, ListState, Padding, Paragraph, Row, Table, TableState,
        Wrap,
    },
    Frame,
};

use crate::{
    app::{App, DisplayRow, InlineNewState, Screen},
    github::CheckStatus,
    jira::Issue,
};

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

const TEXT: Color = Color::White;
const MUTED: Color = Color::DarkGray;
const ACCENT: Color = Color::Blue;
const ACCENT_SOFT: Color = Color::Cyan;
const SURFACE: Color = Color::Reset;
const SURFACE_ALT: Color = Color::DarkGray;
const PANEL: Color = Color::Reset;
const SUCCESS: Color = Color::Green;
const WARNING: Color = Color::Yellow;
const ERROR: Color = Color::Red;
const INFO: Color = Color::LightBlue;

pub fn render(app: &mut App, frame: &mut Frame) {
    match app.screen {
        Screen::List => render_list(app, frame),
        Screen::New => render_new(app, frame),
    }
}

fn render_list(app: &mut App, frame: &mut Frame) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(44)])
        .split(frame.area());

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(columns[0]);

    // Store visible list height for half-page scrolling
    app.list_area_height = main_chunks[0].height.saturating_sub(4);

    render_sidebar(app, frame, columns[1]);

    let rows: Vec<Row> = app
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
    let mut state = TableState::default()
        .with_offset(app.list_scroll_offset)
        .with_selected(Some(app.selected_index));
    let table = Table::new(
        rows,
        [
            Constraint::Length(16),
            Constraint::Length(8),
            Constraint::Length(14),
            Constraint::Length(14),
            Constraint::Min(10),
            Constraint::Length(20),
            Constraint::Length(24),
        ],
    )
    .header(
        Row::new(["Key", "PR", "CI", "Status", "Summary", "Assignee", "Repo"])
            .style(Style::default().fg(MUTED).add_modifier(Modifier::BOLD))
            .bottom_margin(0),
    )
    .column_spacing(2)
    .row_highlight_style(
        Style::default()
            .fg(TEXT)
            .bg(SURFACE_ALT)
            .add_modifier(Modifier::BOLD),
    )
    .block(Block::default().style(Style::default().bg(PANEL)));
    frame.render_stateful_widget(table, main_chunks[0], &mut state);
    app.list_scroll_offset = state.offset();

    render_command_bar(app, frame, main_chunks[1]);

    if app.label_picker_active() {
        render_label_picker_modal(app, frame);
    }
}

fn render_sidebar(app: &App, frame: &mut Frame, area: Rect) {
    let issue = app.selected_issue();

    let mut lines: Vec<Line> = Vec::new();

    match issue {
        Some(issue) => {
            let issue_type = issue.issue_type().map(|ty| ty.name).unwrap_or_default();
            let icon = issue_type_icon(&issue_type);
            lines.push(Line::from(vec![Span::styled(
                format!("{icon} {}", issue.key),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            )]));
            lines.push(Line::from(Span::styled(
                issue_type,
                Style::default().fg(MUTED),
            )));

            lines.push(Line::from(""));

            let summary = issue.summary().unwrap_or_default();
            lines.push(Line::from(Span::styled(
                summary,
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            )));

            lines.push(Line::from(""));

            let status_name = issue.status().map(|s| s.name).unwrap_or_default();
            let status_style = status_color(&status_name);
            lines.push(Line::from(vec![
                Span::styled("Status    ", Style::default().fg(MUTED)),
                Span::styled("●", status_style),
                Span::raw(" "),
                Span::styled(status_name, status_style),
            ]));

            if let Some(priority) = issue.priority() {
                let priority_style = priority_color(&priority.name);
                lines.push(Line::from(vec![
                    Span::styled("Priority  ", Style::default().fg(MUTED)),
                    Span::styled(priority.name.clone(), priority_style),
                ]));
            }

            let assignee = issue
                .assignee()
                .map(|u| u.display_name)
                .unwrap_or_else(|| "Unassigned".to_string());
            lines.push(Line::from(vec![
                Span::styled("Assignee  ", Style::default().fg(MUTED)),
                Span::styled(assignee, Style::default().fg(TEXT)),
            ]));

            if let Some(reporter) = issue.reporter() {
                lines.push(Line::from(vec![
                    Span::styled("Reporter  ", Style::default().fg(MUTED)),
                    Span::styled(reporter.display_name, Style::default().fg(TEXT)),
                ]));
            }

            let labels = issue.labels();
            if !labels.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled("Labels    ", Style::default().fg(MUTED)),
                    Span::styled(labels.join(", "), Style::default().fg(ACCENT_SOFT)),
                ]));
            }

            if let Some(parent) = issue.parent() {
                let parent_summary = parent.summary().unwrap_or_default();
                lines.push(Line::from(vec![
                    Span::styled("Parent    ", Style::default().fg(MUTED)),
                    Span::styled(parent.key, Style::default().fg(ACCENT_SOFT)),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("          ", Style::default().fg(MUTED)),
                    Span::styled(parent_summary, Style::default().fg(MUTED)),
                ]));
            }

            let repos = app
                .repo_matches(issue)
                .into_iter()
                .map(|entry| entry.label.as_str())
                .collect::<Vec<_>>();
            let is_active = app.active_branches.contains_key(&issue.key);
            if !repos.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled("Repo      ", Style::default().fg(MUTED)),
                    if is_active {
                        Span::styled(
                            format!("⎇ {}", repos.join(", ")),
                            Style::default().fg(ACCENT),
                        )
                    } else {
                        Span::styled(repos.join(", "), Style::default().fg(ACCENT_SOFT))
                    },
                ]));
            }

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "─".repeat(area.width.saturating_sub(2) as usize),
                Style::default().fg(SURFACE_ALT),
            )));
            lines.push(Line::from(""));

            match app.github_prs.get(&issue.key) {
                Some(pr) => {
                    lines.push(Line::from(vec![
                        Span::styled("PR ", Style::default().fg(MUTED)),
                        Span::styled(
                            format!("#{}", pr.number),
                            Style::default().fg(INFO).add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" "),
                        Span::styled(
                            &pr.state,
                            Style::default().fg(if pr.state == "OPEN" || pr.state == "open" {
                                SUCCESS
                            } else if pr.state == "MERGED" || pr.state == "merged" {
                                ACCENT
                            } else {
                                MUTED
                            }),
                        ),
                    ]));

                    lines.push(Line::from(Span::styled(
                        &pr.title,
                        Style::default().fg(TEXT),
                    )));

                    lines.push(Line::from(""));

                    if !pr.check_runs.is_empty() {
                        lines.push(Line::from(Span::styled(
                            "CI Checks",
                            Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
                        )));
                        for run in &pr.check_runs {
                            let (icon, color) = match run.status {
                                CheckStatus::Pass => ("✓", SUCCESS),
                                CheckStatus::Fail => ("✗", ERROR),
                                CheckStatus::Pending => ("●", WARNING),
                            };
                            lines.push(Line::from(vec![
                                Span::styled(format!(" {icon} "), Style::default().fg(color)),
                                Span::styled(&run.name, Style::default().fg(TEXT)),
                            ]));
                        }

                        if pr.checks == CheckStatus::Pending {
                            if let Some(eta) = app.pr_eta(pr) {
                                lines.push(Line::from(Span::styled(
                                    format!("   ETA: {eta}"),
                                    Style::default().fg(MUTED),
                                )));
                            }
                        }
                    }

                    if !pr.body.is_empty() {
                        lines.push(Line::from(""));
                        lines.push(Line::from(Span::styled(
                            "─".repeat(area.width.saturating_sub(2) as usize),
                            Style::default().fg(SURFACE_ALT),
                        )));
                        lines.push(Line::from(""));
                        for body_line in pr.body.lines().take(10) {
                            lines.push(Line::from(Span::styled(
                                body_line,
                                Style::default().fg(MUTED),
                            )));
                        }
                        let line_count = pr.body.lines().count();
                        if line_count > 10 {
                            lines.push(Line::from(Span::styled(
                                format!("  … +{} more lines", line_count - 10),
                                Style::default().fg(SURFACE_ALT),
                            )));
                        }
                    }
                }
                None => {
                    lines.push(Line::from(Span::styled(
                        "No linked PR",
                        Style::default().fg(MUTED),
                    )));
                }
            }

            let description = issue.description();
            if let Some(desc) = description {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "─".repeat(area.width.saturating_sub(2) as usize),
                    Style::default().fg(SURFACE_ALT),
                )));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Description",
                    Style::default().fg(MUTED).add_modifier(Modifier::BOLD),
                )));
                for desc_line in desc.lines().take(15) {
                    lines.push(Line::from(Span::styled(
                        desc_line.to_string(),
                        Style::default().fg(TEXT),
                    )));
                }
                let desc_line_count = desc.lines().count();
                if desc_line_count > 15 {
                    lines.push(Line::from(Span::styled(
                        format!("  … +{} more lines", desc_line_count - 15),
                        Style::default().fg(SURFACE_ALT),
                    )));
                }
            }
        }
        None => {
            lines.push(Line::from(Span::styled(
                "No issue selected",
                Style::default().fg(MUTED),
            )));
        }
    }

    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(PANEL))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .padding(Padding::new(1, 1, 1, 0))
                    .style(Style::default().bg(PANEL)),
            ),
        area,
    );
}

fn story_header_row(key: &str, summary: &str, _idx: usize, collapsed: bool) -> Row<'static> {
    let row_style = Style::default().fg(MUTED);

    let first_line = summary.lines().next().unwrap_or_default().to_string();
    let icon = if collapsed { "▶" } else { "▼" };

    Row::new(vec![
        Cell::from(Span::styled(
            format!("{} {}", icon, key),
            Style::default()
                .fg(ACCENT_SOFT)
                .add_modifier(Modifier::BOLD),
        )),
        Cell::from(""), // PR
        Cell::from(""), // CI
        Cell::from(""), // Status
        Cell::from(Span::styled(
            format!("§ {}", first_line),
            Style::default()
                .fg(ACCENT_SOFT)
                .add_modifier(Modifier::BOLD),
        )),
        Cell::from(""), // Assignee
        Cell::from(""), // Repo
    ])
    .style(row_style)
}

fn inline_new_row(state: Option<&InlineNewState>, _idx: usize, depth: u8) -> Row<'static> {
    let row_style = Style::default().fg(TEXT);

    let summary_text = state.map(|s| s.summary.as_str()).unwrap_or("");
    let prefix = if depth > 0 { "  ↳ " } else { "" };

    Row::new(vec![
        Cell::from(Span::styled(
            format!("{prefix}NEW"),
            Style::default().fg(WARNING).add_modifier(Modifier::BOLD),
        )),
        Cell::from(""), // PR
        Cell::from(""), // CI
        Cell::from(""), // Status
        Cell::from(Line::from(vec![
            Span::styled("◦ ", Style::default().fg(MUTED)),
            Span::styled(summary_text.to_string(), Style::default().fg(TEXT)),
            Span::styled(
                "▏".to_string(),
                Style::default()
                    .fg(ACCENT)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ])),
        Cell::from(""), // Assignee
        Cell::from(""), // Repo
    ])
    .style(row_style)
}

fn issue_row(app: &App, issue: &Issue, _idx: usize, depth: u8) -> Row<'static> {
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
    let row_style = Style::default().fg(TEXT);
    let (pr_cell, ci_cell) = match app.github_prs.get(&issue.key) {
        Some(pr) => {
            let pr_cell = Cell::from(Span::styled(
                format!("#{}", pr.number),
                Style::default().fg(INFO),
            ));

            let mut ci_spans = Vec::new();
            for run in &pr.check_runs {
                let (icon, color) = match run.status {
                    CheckStatus::Pass => ("✓", SUCCESS),
                    CheckStatus::Fail => ("✗", ERROR),
                    CheckStatus::Pending => ("●", WARNING),
                };
                ci_spans.push(Span::styled(icon, Style::default().fg(color)));
            }
            if pr.checks == CheckStatus::Pending {
                let spinner = SPINNER_FRAMES[app.spinner_tick % SPINNER_FRAMES.len()];
                ci_spans.push(Span::styled(
                    format!(" {spinner}"),
                    Style::default().fg(WARNING),
                ));
                if let Some(eta) = app.pr_eta(pr) {
                    ci_spans.push(Span::styled(format!(" {eta}"), Style::default().fg(MUTED)));
                }
            }

            (pr_cell, Cell::from(Line::from(ci_spans)))
        }
        None => (Cell::from(""), Cell::from("")),
    };

    let key_prefix = if depth > 0 { "  ↳ " } else { "" };
    let key_cell = Cell::from(Span::styled(
        format!("{}{}", key_prefix, issue.key),
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
    ));

    Row::new(vec![
        key_cell,
        pr_cell,
        ci_cell,
        Cell::from(Line::from(vec![
            Span::styled("●", status_style),
            Span::raw(" "),
            Span::styled(status_name, status_style),
        ])),
        Cell::from(Span::styled(
            format!("{} {}", issue_type_icon(&issue_type), summary),
            Style::default().fg(TEXT),
        )),
        Cell::from(Span::styled(assignee, Style::default().fg(MUTED))),
        Cell::from(Line::from(if is_active {
            vec![
                Span::styled("⎇ ", Style::default().fg(ACCENT)),
                Span::styled(repos, Style::default().fg(ACCENT)),
            ]
        } else {
            vec![Span::styled(repos, Style::default().fg(ACCENT_SOFT))]
        })),
    ])
    .style(row_style)
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
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(SURFACE))
        .border_style(Style::default().fg(ACCENT));
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
            Style::default().fg(MUTED),
        )]))]
    } else {
        filtered
            .iter()
            .map(|entry| {
                let path = entry.path.display().to_string();
                ListItem::new(vec![
                    Line::from(vec![Span::styled(
                        entry.label.clone(),
                        Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
                    )]),
                    Line::from(vec![Span::styled(path, Style::default().fg(MUTED))]),
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
            .fg(PANEL)
            .bg(ACCENT_SOFT)
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
        Style::default().fg(MUTED)
    } else {
        Style::default().fg(TEXT)
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
            Span::styled("/ ", Style::default().fg(ACCENT)),
            Span::styled(filter_display, filter_style),
        ]))
        .block(
            Block::bordered()
                .style(Style::default().bg(SURFACE))
                .border_style(Style::default().fg(ACCENT)),
        ),
        modal_layout[0],
    );

    frame.render_stateful_widget(list, modal_layout[1], &mut state);

    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "↑/↓:Move  •  Enter:Add  •  Esc:Cancel",
            Style::default().fg(MUTED),
        )]))
        .alignment(Alignment::Center)
        .style(Style::default().bg(SURFACE)),
        modal_layout[2],
    );
}

fn render_new(app: &App, frame: &mut Frame) {
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
            Span::styled("✦ ", Style::default().fg(ACCENT_SOFT)),
            Span::styled(
                "New issue",
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!(" {} ", form.project_key),
                Style::default()
                    .fg(PANEL)
                    .bg(ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .block(
            Block::bordered()
                .title(Span::styled(
                    " Compose ",
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ))
                .style(Style::default().bg(PANEL))
                .border_style(Style::default().fg(ACCENT_SOFT)),
        )
        .style(Style::default().bg(PANEL)),
        chunks[0],
    );

    let form_block = Block::bordered()
        .title(Span::styled(
            " New issue details ",
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(PANEL))
        .border_style(Style::default().fg(ACCENT));
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

fn render_form_field(frame: &mut Frame, area: Rect, label: &str, value: &str, active: bool) {
    let block_style = if active {
        Style::default().fg(ACCENT).bg(SURFACE)
    } else {
        Style::default().fg(MUTED).bg(PANEL)
    };

    let label_style = if active {
        Style::default()
            .fg(ACCENT_SOFT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(MUTED).add_modifier(Modifier::BOLD)
    };

    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(vec![
                Span::styled(if active { "▌ " } else { "  " }, block_style),
                Span::styled(label, label_style),
            ]),
            Line::from(Span::styled(value.to_string(), Style::default().fg(TEXT))),
        ]))
        .block(
            Block::bordered()
                .style(Style::default().bg(block_style.bg.unwrap_or(PANEL)))
                .border_style(block_style),
        )
        .style(Style::default().bg(block_style.bg.unwrap_or(PANEL))),
        area,
    );
}

fn help_bar(text: &str) -> Paragraph<'_> {
    let spans = text
        .split("  ")
        .flat_map(|entry| match entry.split_once(':') {
            Some((key, label)) => vec![
                Span::styled(
                    key.to_string(),
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(label.to_string(), Style::default().fg(MUTED)),
                Span::raw("  "),
            ],
            None => vec![
                Span::styled(entry.to_string(), Style::default().fg(MUTED)),
                Span::raw("  "),
            ],
        })
        .collect::<Vec<_>>();

    Paragraph::new(Line::from(spans)).style(Style::default().bg(PANEL))
}

fn render_command_bar(app: &App, frame: &mut Frame, area: Rect) {
    let line = if app.input_mode == crate::app::InputMode::Searching {
        let filter_display = if app.search_filter.is_empty() {
            "Type to filter...".to_string()
        } else {
            app.search_filter.clone()
        };
        let filter_style = if app.search_filter.is_empty() {
            Style::default().fg(MUTED)
        } else {
            Style::default().fg(TEXT)
        };

        Line::from(vec![
            Span::styled("/ ", Style::default().fg(ACCENT)),
            Span::styled(filter_display, filter_style),
            Span::styled(
                "▏",
                Style::default()
                    .fg(ACCENT)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ])
    } else if !app.search_filter.is_empty() {
        let count = app.display_rows.len();
        Line::from(vec![
            Span::styled("/ ", Style::default().fg(TEXT)),
            Span::styled(&app.search_filter, Style::default().fg(TEXT)),
            Span::styled(
                format!("  ({count} results)  Press / to edit, Esc to clear"),
                Style::default().fg(MUTED),
            ),
        ])
    } else if !app.status_message.is_empty() {
        let is_loading = app.loading || app.github_loading || !app.running_tasks.is_empty();
        let is_progress = app.status_message.starts_with('[');
        let spinner = SPINNER_FRAMES[app.spinner_tick % SPINNER_FRAMES.len()];
        let (icon, color) = if app.status_message.starts_with("Failed")
            || app.status_message.starts_with("Error")
        {
            ("✖", ERROR)
        } else if is_loading || is_progress {
            (spinner, WARNING)
        } else {
            ("✔", SUCCESS)
        };

        Line::from(vec![
            Span::styled(format!("{icon} "), Style::default().fg(color)),
            Span::styled(app.status_message.as_str(), Style::default().fg(TEXT)),
        ])
    } else {
        let pairs: &[(&str, &str)] = if app.inline_new_active() {
            &[("Esc", "Cancel"), ("↵", "Create")]
        } else {
            &[
                ("^C", "Quit"),
                ("↵", "View"),
                ("/", "Search"),
                ("o", "PR"),
                ("t", "Ticket"),
                ("p", "Pick up"),
                ("f", "Finish"),
                ("n", "New"),
                ("a", "Label"),
                ("r", "Refresh"),
            ]
        };

        let mut spans: Vec<Span> = pairs
            .iter()
            .enumerate()
            .flat_map(|(index, (key, label))| {
                let mut s = vec![
                    Span::styled(
                        *key,
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" "),
                    Span::styled(*label, Style::default().fg(MUTED)),
                ];
                if index < pairs.len() - 1 {
                    s.push(Span::raw("  "));
                }
                s
            })
            .collect();

        if app.inline_new_active() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled("type summary…", Style::default().fg(MUTED)));
        }

        if app.loading {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!(
                    "{} Loading…",
                    SPINNER_FRAMES[app.spinner_tick % SPINNER_FRAMES.len()]
                ),
                Style::default().fg(MUTED),
            ));
        } else if let Some(last_updated) = app.last_updated {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!(
                    "updated {} ago",
                    crate::app::format_duration(last_updated.elapsed().as_secs())
                ),
                Style::default().fg(MUTED),
            ));
        }

        Line::from(spans)
    };

    frame.render_widget(Paragraph::new(line).style(Style::default().bg(PANEL)), area);
}

fn status_color(status: &str) -> Style {
    let status = status.to_lowercase();
    if status.contains("done") {
        return Style::default().fg(SUCCESS);
    }
    if status.contains("progress") {
        return Style::default().fg(WARNING);
    }
    if status.contains("review") {
        return Style::default().fg(INFO);
    }
    if status.contains("blocked") || status.contains("rejected") {
        return Style::default().fg(ERROR);
    }
    if status.contains("backlog") {
        return Style::default().fg(MUTED);
    }
    if status.contains("todo") || status.contains("to do") {
        return Style::default().fg(ACCENT);
    }
    if status.contains("proposed") || status.contains("plan") {
        return Style::default().fg(MUTED);
    }

    Style::default().fg(TEXT)
}

fn priority_color(priority: &str) -> Style {
    let priority = priority.to_lowercase();
    if priority.contains("highest") || priority.contains("critical") {
        return Style::default().fg(ERROR).add_modifier(Modifier::BOLD);
    }
    if priority.contains("high") {
        return Style::default().fg(ERROR);
    }
    if priority.contains("medium") {
        return Style::default().fg(WARNING);
    }
    if priority.contains("low") {
        return Style::default().fg(MUTED);
    }
    if priority.contains("lowest") {
        return Style::default().fg(MUTED);
    }
    Style::default().fg(TEXT)
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);

    horizontal[1]
}

fn issue_type_icon(issue_type: &str) -> &'static str {
    let issue_type = issue_type.to_lowercase();
    if issue_type.contains("bug") {
        return "¤";
    }
    if issue_type.contains("story") || issue_type.contains("epic") {
        return "§";
    }
    if issue_type.contains("sub") {
        return "↳";
    }
    if issue_type.contains("task") {
        return "◦";
    }

    "•"
}
