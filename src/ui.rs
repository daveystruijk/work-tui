use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState, Wrap,
    },
    Frame,
};

use crate::{
    app::{App, DisplayRow, InlineNewState, Screen},
    events::{EventLevel, EventLoadState, EventSource},
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
        Screen::Detail => render_detail(app, frame),
        Screen::New => render_new(app, frame),
    }
}

fn render_list(app: &mut App, frame: &mut Frame) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Min(1),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(frame.area());

    // Store visible list height (minus borders and header row) for half-page scrolling
    app.list_area_height = chunks[1].height.saturating_sub(4);

    render_info_panel(app, frame, chunks[0]);

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
            Constraint::Min(10),
            Constraint::Length(10),
            Constraint::Length(20),
            Constraint::Length(24),
        ],
    )
    .header(
        Row::new(["Key", "PR", "Status", "Summary", "Type", "Assignee", "Repo"]).style(
            Style::default()
                .fg(ACCENT_SOFT)
                .bg(SURFACE)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .column_spacing(2)
    .row_highlight_style(
        Style::default()
            .fg(TEXT)
            .bg(SURFACE_ALT)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("▸ ")
    .block({
        let title = if app.loading {
            format!(
                " Assigned issues {} Loading… ",
                SPINNER_FRAMES[app.spinner_tick % SPINNER_FRAMES.len()]
            )
        } else {
            " Assigned issues ".to_string()
        };
        Block::bordered()
            .title(Span::styled(
                title,
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(PANEL))
            .border_style(Style::default().fg(ACCENT))
    });
    frame.render_stateful_widget(table, chunks[1], &mut state);
    app.list_scroll_offset = state.offset();

    let help_text = if app.inline_new_active() {
        "Esc:Cancel  Enter:Create  type summary…"
    } else {
        "Ctrl+C:Quit  Enter:View  o:PR  t:Ticket  p:Pick up  f:Finish  n:New  a:Add label  r:Refresh"
    };
    frame.render_widget(help_bar(help_text), chunks[2]);

    render_status_bar(app, frame, chunks[3]);

    if app.label_picker_active() {
        render_label_picker_modal(app, frame);
    }
}

fn render_info_panel(app: &App, frame: &mut Frame, area: Rect) {
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let issue = app.selected_issue();

    // Pane 1: Jira Description
    let jira_content: Text = match issue {
        Some(i) => {
            let summary = i.summary().unwrap_or_default();
            let description = i
                .description()
                .unwrap_or_else(|| "No description".to_string());
            Text::from(vec![
                Line::from(Span::styled(
                    summary,
                    Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(description, Style::default().fg(TEXT))),
            ])
        }
        None => Text::from(Span::styled(
            "No issue selected",
            Style::default().fg(MUTED),
        )),
    };
    frame.render_widget(
        Paragraph::new(jira_content)
            .style(Style::default().fg(TEXT).bg(SURFACE))
            .wrap(Wrap { trim: false })
            .block(
                Block::bordered()
                    .title(Span::styled(
                        " Jira Description ",
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    ))
                    .style(Style::default().bg(SURFACE))
                    .border_style(Style::default().fg(ACCENT_SOFT)),
            ),
        panes[0],
    );

    // Pane 2: GitHub PR Description
    let pr_content: Text = match issue.and_then(|i| app.github_prs.get(&i.key)) {
        Some(pr) => {
            let title_line = Line::from(Span::styled(
                format!("#{} {}", pr.number, pr.title),
                Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
            ));
            if pr.body.is_empty() {
                Text::from(vec![
                    title_line,
                    Line::from(""),
                    Line::from(Span::styled("No description", Style::default().fg(MUTED))),
                ])
            } else {
                Text::from(vec![
                    title_line,
                    Line::from(""),
                    Line::from(Span::styled(pr.body.clone(), Style::default().fg(TEXT))),
                ])
            }
        }
        None => Text::from(Span::styled("No linked PR", Style::default().fg(MUTED))),
    };
    frame.render_widget(
        Paragraph::new(pr_content)
            .style(Style::default().fg(TEXT).bg(SURFACE))
            .wrap(Wrap { trim: false })
            .block(
                Block::bordered()
                    .title(Span::styled(
                        " PR Description ",
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    ))
                    .style(Style::default().bg(SURFACE))
                    .border_style(Style::default().fg(ACCENT_SOFT)),
            ),
        panes[1],
    );
}

fn story_header_row(key: &str, summary: &str, idx: usize, collapsed: bool) -> Row<'static> {
    let row_style = if idx % 2 == 0 {
        Style::default().fg(MUTED).bg(PANEL)
    } else {
        Style::default().fg(MUTED).bg(SURFACE)
    };

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
        Cell::from(""), // Status
        Cell::from(Span::styled(
            first_line,
            Style::default()
                .fg(ACCENT_SOFT)
                .add_modifier(Modifier::BOLD),
        )),
        Cell::from(""), // Type
        Cell::from(""), // Assignee
        Cell::from(""), // Repo
    ])
    .style(row_style)
}

fn inline_new_row(state: Option<&InlineNewState>, idx: usize, depth: u8) -> Row<'static> {
    let row_style = if idx % 2 == 0 {
        Style::default().fg(TEXT).bg(PANEL)
    } else {
        Style::default().fg(TEXT).bg(SURFACE)
    };

    let summary_text = state.map(|s| s.summary.as_str()).unwrap_or("");
    let prefix = if depth > 0 { "  ↳ " } else { "" };

    Row::new(vec![
        Cell::from(Span::styled(
            format!("{prefix}NEW"),
            Style::default().fg(WARNING).add_modifier(Modifier::BOLD),
        )),
        Cell::from(""), // PR
        Cell::from(""), // Status
        Cell::from(Line::from(vec![
            Span::styled(summary_text.to_string(), Style::default().fg(TEXT)),
            Span::styled(
                "▏".to_string(),
                Style::default()
                    .fg(ACCENT)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ])),
        Cell::from(Span::styled("◦ Task", Style::default().fg(MUTED))),
        Cell::from(""), // Assignee
        Cell::from(""), // Repo
    ])
    .style(row_style)
}

fn issue_row(app: &App, issue: &Issue, idx: usize, depth: u8) -> Row<'static> {
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
    let row_style = if idx % 2 == 0 {
        Style::default().fg(TEXT).bg(PANEL)
    } else {
        Style::default().fg(TEXT).bg(SURFACE)
    };
    let is_highlighted = app.highlight_ticks.contains_key(&issue.key);
    let new = app.new_fields.get(&issue.key);

    let pr_cell = match app.github_prs.get(&issue.key) {
        Some(pr) => {
            let (icon, color) = match pr.checks {
                CheckStatus::Pass => ("✓", SUCCESS),
                CheckStatus::Fail => ("✗", ERROR),
                CheckStatus::Pending => ("●", WARNING),
            };
            let mut spans = Vec::new();
            if new.is_some_and(|f| f.pr) {
                spans.push(Span::styled("★ ", Style::default().fg(WARNING)));
            }
            spans.push(Span::styled(format!("{icon} "), Style::default().fg(color)));
            spans.push(Span::styled(
                format!("#{}", pr.number),
                Style::default().fg(INFO),
            ));
            Cell::from(Line::from(spans))
        }
        None => Cell::from(""),
    };

    let key_prefix = if depth > 0 { "  ↳ " } else { "" };
    let key_style = {
        let style = Style::default().fg(ACCENT).add_modifier(Modifier::BOLD);
        if is_highlighted {
            style.add_modifier(Modifier::SLOW_BLINK)
        } else {
            style
        }
    };
    let key_cell = if new.is_some_and(|f| f.key) {
        Cell::from(Line::from(vec![
            Span::styled("★ ", Style::default().fg(WARNING)),
            Span::styled(format!("{}{}", key_prefix, issue.key), key_style),
        ]))
    } else {
        Cell::from(Span::styled(
            format!("{}{}", key_prefix, issue.key),
            key_style,
        ))
    };

    let mut status_spans = Vec::new();
    if new.is_some_and(|f| f.status) {
        status_spans.push(Span::styled("★ ", Style::default().fg(WARNING)));
    }
    status_spans.push(Span::styled("●", status_style));
    status_spans.push(Span::raw(" "));
    status_spans.push(Span::styled(status_name, status_style));

    Row::new(vec![
        key_cell,
        pr_cell,
        Cell::from(Line::from(status_spans)),
        Cell::from(Span::styled(
            summary,
            if is_highlighted {
                Style::default().fg(TEXT).add_modifier(Modifier::SLOW_BLINK)
            } else {
                Style::default().fg(TEXT)
            },
        )),
        Cell::from(format!("{} {}", issue_type_icon(&issue_type), issue_type)),
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

fn render_detail(app: &App, frame: &mut Frame) {
    let issue = match app.selected_issue() {
        Some(issue) => issue,
        None => return,
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Min(5),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .split(frame.area());

    // Split the main content area between description and activity
    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[3]);

    let title = Line::from(vec![
        Span::styled(
            format!(" {} ", issue.key),
            Style::default()
                .fg(PANEL)
                .bg(ACCENT_SOFT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            issue.summary().unwrap_or_default(),
            Style::default().fg(TEXT).add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(title)
            .block(
                Block::bordered()
                    .title(Span::styled(
                        " Issue details ",
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    ))
                    .style(Style::default().bg(PANEL))
                    .border_style(Style::default().fg(ACCENT_SOFT)),
            )
            .style(Style::default().bg(PANEL)),
        chunks[0],
    );

    let info_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);
    let status = issue
        .status()
        .map(|s| s.name)
        .unwrap_or_else(|| "None".to_string());
    let status_style = status_color(&status);
    let priority = issue
        .priority()
        .map(|p| p.name)
        .unwrap_or_else(|| "None".to_string());

    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(vec![
                Span::styled("● ", status_style),
                Span::styled(status, status_style),
            ]),
            Line::from(vec![
                Span::styled("Priority  ", Style::default().fg(MUTED)),
                Span::styled(priority, Style::default().fg(TEXT)),
            ]),
        ]))
        .block(
            Block::bordered()
                .title(Span::styled(
                    " Status ",
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ))
                .style(Style::default().bg(SURFACE))
                .border_style(Style::default().fg(status_style.fg.unwrap_or(INFO))),
        ),
        info_chunks[0],
    );

    let assignee = issue
        .assignee()
        .map(|u| u.display_name)
        .unwrap_or_else(|| "Unassigned".to_string());
    let issue_type = issue
        .issue_type()
        .map(|ty| ty.name)
        .unwrap_or_else(|| "Unknown".to_string());

    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::from(vec![
                Span::styled("Owner     ", Style::default().fg(MUTED)),
                Span::styled(assignee, Style::default().fg(TEXT)),
            ]),
            Line::from(vec![
                Span::styled(
                    format!("{} ", issue_type_icon(&issue_type)),
                    Style::default().fg(ACCENT_SOFT),
                ),
                Span::styled(issue_type, Style::default().fg(TEXT)),
            ]),
        ]))
        .block(
            Block::bordered()
                .title(Span::styled(
                    " Ownership ",
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ))
                .style(Style::default().bg(SURFACE))
                .border_style(Style::default().fg(ACCENT_SOFT)),
        ),
        info_chunks[1],
    );

    render_repo_panel(app, issue, frame, chunks[2]);

    let description = issue
        .description()
        .unwrap_or_else(|| "No description".to_string());

    frame.render_widget(
        Paragraph::new(description)
            .style(Style::default().fg(TEXT).bg(SURFACE))
            .wrap(Wrap { trim: false })
            .scroll((app.detail_scroll, 0))
            .block(
                Block::bordered()
                    .title(Span::styled(
                        format!(
                            " Description • line {} ",
                            app.detail_scroll.saturating_add(1)
                        ),
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    ))
                    .style(Style::default().bg(SURFACE))
                    .border_style(Style::default().fg(ACCENT_SOFT)),
            ),
        body_chunks[0],
    );

    render_activity_panel(app, &issue.key, frame, body_chunks[1]);

    frame.render_widget(
        help_bar(
            "Esc:Back  o:PR  t:Ticket  p:Pick up  f:Finish  a:Add label  r:Refresh  j/k:Scroll",
        ),
        chunks[4],
    );

    render_status_bar(app, frame, chunks[5]);

    if app.label_picker_active() {
        render_label_picker_modal(app, frame);
    }
}

fn render_repo_panel(app: &App, issue: &Issue, frame: &mut Frame, area: Rect) {
    let labels = issue.labels();
    let label_text = if labels.is_empty() {
        "None".to_string()
    } else {
        labels.join(", ")
    };
    let mut lines = vec![Line::from(vec![
        Span::styled("Labels     ", Style::default().fg(MUTED)),
        Span::styled(label_text, Style::default().fg(TEXT)),
    ])];

    if let Some(error) = &app.repo_error {
        lines.push(Line::from(vec![
            Span::styled("⚠ ", Style::default().fg(WARNING)),
            Span::styled(error.as_str(), Style::default().fg(WARNING)),
        ]));
    } else if app.repo_entries.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            "No repositories found in REPOS_DIR",
            Style::default().fg(MUTED),
        )]));
    } else {
        let matches = app.repo_matches(issue);
        if matches.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                "No linked repositories. Press 'a' to add.",
                Style::default().fg(MUTED),
            )]));
        } else {
            for entry in matches {
                let path_text = entry.path.display().to_string();
                lines.push(Line::from(vec![
                    Span::styled("● ", Style::default().fg(ACCENT_SOFT)),
                    Span::styled(
                        entry.label.as_str(),
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled(path_text, Style::default().fg(TEXT)),
                ]));
            }
        }
    }

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .block(
                Block::bordered()
                    .title(Span::styled(
                        " Linked repositories ",
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    ))
                    .style(Style::default().bg(SURFACE))
                    .border_style(Style::default().fg(ACCENT_SOFT)),
            )
            .style(Style::default().fg(TEXT).bg(SURFACE)),
        area,
    );
}

fn render_activity_panel(app: &App, issue_key: &str, frame: &mut Frame, area: Rect) {
    let state = app.issue_events.get(issue_key);

    let (title_suffix, lines) = match state {
        None | Some(EventLoadState::NotLoaded) => (
            "".to_string(),
            vec![Line::from(Span::styled(
                "Press Enter to load events",
                Style::default().fg(MUTED),
            ))],
        ),
        Some(EventLoadState::Loading) => {
            let spinner = SPINNER_FRAMES[app.spinner_tick % SPINNER_FRAMES.len()];
            (
                format!(" {spinner} Loading…"),
                vec![Line::from(Span::styled(
                    "Loading events…",
                    Style::default().fg(MUTED),
                ))],
            )
        }
        Some(EventLoadState::Error(err)) => (
            "".to_string(),
            vec![Line::from(vec![
                Span::styled("⚠ ", Style::default().fg(ERROR)),
                Span::styled(err.as_str(), Style::default().fg(ERROR)),
            ])],
        ),
        Some(EventLoadState::Loaded(events)) => {
            if events.is_empty() {
                (
                    "".to_string(),
                    vec![Line::from(Span::styled(
                        "No events found",
                        Style::default().fg(MUTED),
                    ))],
                )
            } else {
                let event_lines: Vec<Line> = events
                    .iter()
                    .flat_map(|event| {
                        let (icon, color) = event_level_style(&event.level);
                        let source_icon = match event.source {
                            EventSource::GitHub => "  ",
                            EventSource::Jira => "  ",
                        };

                        let timestamp = if event.at.len() >= 16 {
                            &event.at[..16]
                        } else {
                            &event.at
                        };

                        let mut result = vec![Line::from(vec![
                            Span::styled(format!("{icon} "), Style::default().fg(color)),
                            Span::styled(
                                event.title.clone(),
                                Style::default().fg(color).add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(source_icon.to_string(), Style::default().fg(MUTED)),
                            Span::styled(timestamp.to_string(), Style::default().fg(MUTED)),
                        ])];

                        if let Some(detail) = &event.detail {
                            result.push(Line::from(vec![
                                Span::raw("  "),
                                Span::styled(detail.clone(), Style::default().fg(MUTED)),
                            ]));
                        }

                        result
                    })
                    .collect();
                (format!(" • {} events", events.len()), event_lines)
            }
        }
    };

    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .wrap(Wrap { trim: false })
            .scroll((app.detail_scroll, 0))
            .block(
                Block::bordered()
                    .title(Span::styled(
                        format!(" Activity{title_suffix} "),
                        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                    ))
                    .style(Style::default().bg(SURFACE))
                    .border_style(Style::default().fg(ACCENT)),
            )
            .style(Style::default().fg(TEXT).bg(SURFACE)),
        area,
    );
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

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .fg(PANEL)
                .bg(ACCENT_SOFT)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

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
            Constraint::Length(3),
            Constraint::Length(3),
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

    render_status_bar(app, frame, chunks[3]);
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

fn render_status_bar(app: &App, frame: &mut Frame, area: Rect) {
    if app.status_message.is_empty() {
        return;
    }

    let is_loading = app.loading || app.github_loading || !app.running_tasks.is_empty();
    // Progress messages (from actions) start with '[' — treat them as loading
    let is_progress = app.status_message.starts_with('[');
    let spinner = SPINNER_FRAMES[app.spinner_tick % SPINNER_FRAMES.len()];
    let (icon, color) =
        if app.status_message.starts_with("Failed") || app.status_message.starts_with("Error") {
            ("✖", ERROR)
        } else if is_loading || is_progress {
            (spinner, WARNING)
        } else {
            ("✔", SUCCESS)
        };

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" {} ", icon),
                Style::default()
                    .fg(PANEL)
                    .bg(color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(app.status_message.as_str(), Style::default().fg(TEXT)),
        ]))
        .block(
            Block::bordered()
                .style(Style::default().bg(PANEL))
                .border_style(Style::default().fg(color)),
        )
        .style(Style::default().bg(PANEL)),
        area,
    );
}

fn help_bar(text: &str) -> Paragraph<'_> {
    let spans = text
        .split("  ")
        .flat_map(|entry| match entry.split_once(':') {
            Some((key, label)) => vec![
                Span::styled(
                    format!(" {} ", key),
                    Style::default()
                        .fg(PANEL)
                        .bg(ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(format!("{}   ", label), Style::default().fg(MUTED)),
            ],
            None => vec![Span::styled(entry.to_string(), Style::default().fg(MUTED))],
        })
        .collect::<Vec<_>>();

    Paragraph::new(Line::from(spans))
        .block(
            Block::bordered()
                .style(Style::default().bg(PANEL))
                .border_style(Style::default().fg(SURFACE_ALT)),
        )
        .style(Style::default().bg(PANEL))
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

fn event_level_style(level: &EventLevel) -> (&'static str, Color) {
    match level {
        EventLevel::Success => ("✔", SUCCESS),
        EventLevel::Error => ("✖", ERROR),
        EventLevel::Warning => ("◌", WARNING),
        EventLevel::Info => ("↺", INFO),
        EventLevel::Neutral => ("•", MUTED),
    }
}

fn issue_type_icon(issue_type: &str) -> &'static str {
    let issue_type = issue_type.to_lowercase();
    if issue_type.contains("bug") {
        return "◉";
    }
    if issue_type.contains("story") || issue_type.contains("epic") {
        return "📖";
    }
    if issue_type.contains("sub") {
        return "↳";
    }
    if issue_type.contains("task") {
        return "◦";
    }

    "•"
}
