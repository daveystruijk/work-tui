use std::collections::HashMap;

use chrono::DateTime;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    symbols,
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Cell, Clear, List, ListItem, ListState, Padding, Paragraph, Row, Table,
        TableState, Wrap,
    },
    Frame,
};

use crate::theme::Theme;
use crate::{
    app::{App, DisplayRow, InlineNewState, Screen},
    github::{CheckStatus, PrInfo},
    jira::{Issue, User},
};

const COLUMNS: &[&str] = &["Key", "Summary", "PR", "CI", "Status", "Assignee", "Repo"];
const SIDEBAR_SECTION_MARGIN: u16 = 1;

type CellMap<'a> = HashMap<&'static str, Line<'a>>;

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Max content width for a named column across all rows.
fn max_col_width(row_data: &[(CellMap, Style)], name: &str) -> u16 {
    row_data
        .iter()
        .map(|(cells, _)| cells.get(name).map_or(0, |l| l.width() as u16))
        .max()
        .unwrap_or(0)
}

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
    app.list_area_height = main_chunks[0].height.saturating_sub(1);

    render_sidebar(app, frame, columns[1]);

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

    render_command_bar(app, frame, main_chunks[1]);

    if app.label_picker_active() {
        render_label_picker_modal(app, frame);
    }
}

fn render_sidebar(app: &App, frame: &mut Frame, area: Rect) {
    let sidebar = Block::default()
        .padding(Padding::new(1, 1, 1, 0))
        .style(Style::default().bg(Theme::SidebarBg));
    let inner = sidebar.inner(area);
    frame.render_widget(sidebar, area);

    let Some(issue) = app.selected_issue() else {
        frame.render_widget(
            Paragraph::new(vec![Line::from(Span::styled(
                "No issue selected",
                Style::default().fg(Theme::Muted),
            ))])
            .style(Style::default().bg(Theme::SidebarBg)),
            inner,
        );
        return;
    };

    let issue_type = issue.issue_type().map(|ty| ty.name).unwrap_or_default();
    let icon = issue_type_icon(&issue_type);
    let summary = issue.summary().unwrap_or_default();

    let header_line = Line::from(vec![
        Span::styled(
            format!("{icon} {}", issue.key),
            Style::default()
                .fg(Theme::Accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            summary.clone(),
            Style::default()
                .fg(Theme::Text)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    let header_height = wrapped_line_count(
        &format!("{icon} {} {summary}", issue.key),
        inner.width as usize,
    ) as u16;

    let mut jira_lines = Vec::new();
    let status_name = issue.status().map(|status| status.name).unwrap_or_default();
    let status_style = status_color(&status_name);
    jira_lines.push(Line::from(vec![
        Span::styled("Status    ", Style::default().fg(Theme::Muted)),
        Span::styled("●", status_style),
        Span::raw(" "),
        Span::styled(status_name, status_style),
    ]));

    let (assignee, assignee_color) = issue
        .assignee()
        .map(|user| (user.display_name, Theme::Text))
        .unwrap_or_else(|| ("Unassigned".to_string(), Theme::Muted));
    jira_lines.push(labeled_text_line("Assignee", assignee, assignee_color));

    if let Some(author) = issue_author(issue) {
        jira_lines.push(labeled_text_line("Author", author, Theme::Text));
    }

    if let Some(reporter) = issue.reporter() {
        jira_lines.push(labeled_text_line(
            "Reporter",
            reporter.display_name,
            Theme::Text,
        ));
    }

    if let Some(created) = issue_field_string(issue, "created") {
        jira_lines.push(labeled_text_line(
            "Created",
            humanize_timestamp(&created),
            Theme::Text,
        ));
    }

    if let Some(updated) = issue_field_string(issue, "updated") {
        jira_lines.push(labeled_text_line(
            "Updated",
            humanize_timestamp(&updated),
            Theme::Text,
        ));
    }

    if let Some(description) = issue.description() {
        jira_lines.push(Line::from(""));
        push_wrapped_block(
            &mut jira_lines,
            &description,
            inner.width.saturating_sub(6) as usize,
            8,
            Theme::Text,
            "",
        );
    }

    let mut github_lines = Vec::new();
    let mut ci_lines = Vec::new();

    match app.github_prs.get(&issue.key) {
        Some(pr) => {
            let detail_loading = app.github_pr_detail_loading.contains(&issue.key);
            let detail_error = app.github_pr_detail_errors.get(&issue.key);
            let detail_loaded = app.github_pr_detail_loaded.contains(&issue.key);
            let (pr_state_label, pr_state_color) = if pr.is_draft {
                ("DRAFT", Theme::Muted)
            } else if pr.state.eq_ignore_ascii_case("open") {
                ("OPEN", Theme::Success)
            } else if pr.state.eq_ignore_ascii_case("merged") {
                ("MERGED", Theme::Accent)
            } else {
                (pr.state.as_str(), Theme::Muted)
            };

            github_lines.push(Line::from(vec![
                Span::styled("PR        ", Style::default().fg(Theme::Muted)),
                Span::styled(
                    format!("#{}", pr.number),
                    Style::default()
                        .fg(Theme::Info)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(pr_state_label, Style::default().fg(pr_state_color)),
            ]));

            let comments_value = if detail_loading && !detail_loaded {
                "Loading…".to_string()
            } else if detail_error.is_some() {
                "Unavailable".to_string()
            } else {
                let (unresolved, resolved) = comment_counts(pr);
                format!("{unresolved} unresolved · {resolved} resolved")
            };
            github_lines.push(labeled_text_line("Comments", comments_value, Theme::Text));

            let ci_content_width = inner.width.saturating_sub(6) as usize;
            if !pr.check_runs.is_empty() {
                for run in &pr.check_runs {
                    let (icon, color) = match run.status {
                        CheckStatus::Pass => ("✓", Theme::Success),
                        CheckStatus::Fail => ("✗", Theme::Error),
                        CheckStatus::Pending => ("●", Theme::Warning),
                    };
                    let timing = app.check_run_timing(pr, run).unwrap_or_default();
                    let mut spans = vec![
                        Span::styled(format!(" {icon} "), Style::default().fg(color)),
                        Span::styled(&run.name, Style::default().fg(Theme::Text)),
                    ];
                    if !timing.is_empty() {
                        spans.push(Span::styled(
                            format!("  {timing}"),
                            Style::default().fg(Theme::Muted),
                        ));
                    }
                    ci_lines.push(Line::from(spans));

                    // Render substeps for non-passed runs
                    if run.status != CheckStatus::Pass {
                        let spinner = SPINNER_FRAMES[app.spinner_tick % SPINNER_FRAMES.len()];
                        for step in &run.steps {
                            let (step_icon, step_color) = match step.status {
                                CheckStatus::Pass => ("✓", Theme::Success),
                                CheckStatus::Fail => ("✗", Theme::Error),
                                CheckStatus::Pending => (spinner, Theme::Warning),
                            };
                            let step_timing =
                                app.check_step_timing(pr, run, step).unwrap_or_default();
                            let mut step_spans = vec![
                                Span::styled(
                                    format!("   {step_icon} "),
                                    Style::default().fg(step_color),
                                ),
                                Span::styled(&step.name, Style::default().fg(Theme::Muted)),
                            ];
                            if !step_timing.is_empty() {
                                step_spans.push(Span::styled(
                                    format!("  {step_timing}"),
                                    Style::default().fg(Theme::Muted),
                                ));
                            }
                            ci_lines.push(Line::from(step_spans));
                        }
                    }

                    // Inline error output below failed steps
                    if run.status == CheckStatus::Fail {
                        let error_message = if !run.text.trim().is_empty() {
                            Some(run.text.trim().to_string())
                        } else if !run.summary.trim().is_empty() {
                            Some(run.summary.trim().to_string())
                        } else if !run.details_url.trim().is_empty() {
                            Some(format!("Open: {}", run.details_url))
                        } else if detail_loading {
                            Some("Loading error…".to_string())
                        } else if let Some(error) = detail_error {
                            Some(format!("Failed to load: {error}"))
                        } else {
                            None
                        };
                        if let Some(message) = error_message {
                            for line in wrap_text(&message, ci_content_width.saturating_sub(3), 6) {
                                ci_lines.push(Line::from(vec![
                                    Span::styled("   ", Style::default()),
                                    Span::styled(
                                        format!(" {line} "),
                                        Style::default()
                                            .fg(Theme::Text)
                                            .bg(ratatui::style::Color::Black),
                                    ),
                                ]));
                            }
                        }
                    }
                }
            } else {
                ci_lines.push(Line::from(Span::styled(
                    "No CI results",
                    Style::default().fg(Theme::Muted),
                )));
            }
        }
        None => {
            github_lines.push(Line::from(Span::styled(
                "No linked PR",
                Style::default().fg(Theme::Muted),
            )));
            ci_lines.push(Line::from(Span::styled(
                "No CI results",
                Style::default().fg(Theme::Muted),
            )));
        }
    }

    let mut constraints = vec![Constraint::Length(header_height.max(1))];
    constraints.push(Constraint::Length(SIDEBAR_SECTION_MARGIN));
    constraints.push(Constraint::Length(section_height(&jira_lines)));
    constraints.push(Constraint::Length(SIDEBAR_SECTION_MARGIN));
    constraints.push(Constraint::Length(section_height(&github_lines)));
    constraints.push(Constraint::Length(SIDEBAR_SECTION_MARGIN));
    constraints.push(Constraint::Length(section_height(&ci_lines)));
    constraints.push(Constraint::Min(0));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    frame.render_widget(
        Paragraph::new(vec![header_line])
            .style(Style::default().bg(Theme::SidebarBg))
            .wrap(Wrap { trim: false }),
        chunks[0],
    );
    render_sidebar_section(frame, chunks[2], "Jira", jira_lines, Theme::SurfaceAlt);
    render_sidebar_section(frame, chunks[4], "GitHub", github_lines, Theme::SurfaceAlt);
    render_sidebar_section(frame, chunks[6], "CI", ci_lines, Theme::SurfaceAlt);
}

fn render_sidebar_section<'a>(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    lines: Vec<Line<'a>>,
    border_color: ratatui::style::Color,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let top_only_border = symbols::border::Set {
        horizontal_top: symbols::line::NORMAL.horizontal,
        top_left: symbols::line::NORMAL.horizontal,
        top_right: symbols::line::NORMAL.horizontal,
        ..symbols::border::PLAIN
    };

    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(Theme::SidebarBg).fg(Theme::Muted))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_set(top_only_border)
                    .title(Span::styled(
                        format!(" {title} "),
                        Style::default().fg(border_color),
                    ))
                    .border_style(Style::default().fg(border_color))
                    .padding(Padding::new(1, 2, 0, 0))
                    .style(Style::default().bg(Theme::SidebarBg)),
            ),
        area,
    );
}

fn section_height(lines: &[Line<'_>]) -> u16 {
    (lines.len() as u16).saturating_add(1)
}

fn comment_counts(pr: &PrInfo) -> (usize, usize) {
    let mut unresolved = pr.comments.len();
    let mut resolved = 0;

    for thread in &pr.review_threads {
        let thread_comment_count = thread.comments.len();
        if thread.is_resolved {
            resolved += thread_comment_count;
        } else {
            unresolved += thread_comment_count;
        }
    }

    (unresolved, resolved)
}

fn push_wrapped_block<'a>(
    lines: &mut Vec<Line<'a>>,
    text: &str,
    width: usize,
    max_lines: usize,
    color: ratatui::style::Color,
    prefix: &str,
) {
    for line in wrap_text(text, width, max_lines) {
        lines.push(Line::from(Span::styled(
            format!("{prefix}{line}"),
            Style::default().fg(color),
        )));
    }
}

fn labeled_text_line(label: &str, value: String, color: ratatui::style::Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<10}"), Style::default().fg(Theme::Muted)),
        Span::styled(value, Style::default().fg(color)),
    ])
}

fn issue_field_string(issue: &Issue, field: &str) -> Option<String> {
    issue.field::<String>(field).and_then(|result| result.ok())
}

fn issue_author(issue: &Issue) -> Option<String> {
    issue
        .field::<User>("creator")
        .and_then(|result| result.ok())
        .map(|user| user.display_name)
}

fn humanize_timestamp(timestamp: &str) -> String {
    let parsed = DateTime::parse_from_rfc3339(timestamp)
        .or_else(|_| DateTime::parse_from_str(timestamp, "%Y-%m-%dT%H:%M:%S%.3f%z"));
    let Ok(parsed) = parsed else {
        return timestamp.to_string();
    };
    let local = parsed.with_timezone(&chrono::Local);
    local.format("%Y-%m-%d %H:%M").to_string()
}

fn wrap_text(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    if width == 0 || max_lines == 0 {
        return Vec::new();
    }

    let mut wrapped = Vec::new();
    let paragraphs = if text.trim().is_empty() {
        vec![String::new()]
    } else {
        text.lines()
            .map(str::trim)
            .map(ToString::to_string)
            .collect()
    };

    'outer: for paragraph in paragraphs {
        if paragraph.is_empty() {
            wrapped.push(String::new());
            if wrapped.len() >= max_lines {
                break;
            }
            continue;
        }

        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let candidate_width = if current.is_empty() {
                word.len()
            } else {
                current.len() + 1 + word.len()
            };

            if candidate_width > width && !current.is_empty() {
                wrapped.push(current);
                if wrapped.len() >= max_lines {
                    break 'outer;
                }
                current = word.to_string();
                continue;
            }

            if word.len() > width && current.is_empty() {
                wrapped.push(
                    word.chars()
                        .take(width.saturating_sub(1))
                        .collect::<String>()
                        + "…",
                );
                if wrapped.len() >= max_lines {
                    break 'outer;
                }
                continue;
            }

            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }

        if !current.is_empty() {
            wrapped.push(current);
            if wrapped.len() >= max_lines {
                break;
            }
        }
    }

    if wrapped.len() == max_lines && !text.trim().is_empty() {
        if let Some(last) = wrapped.last_mut() {
            if !last.ends_with('…') {
                last.push('…');
            }
        }
    }

    wrapped
}

fn wrapped_line_count(text: &str, width: usize) -> usize {
    if text.is_empty() {
        return 1;
    }

    wrap_text(text, width.max(1), usize::MAX).len().max(1)
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

fn render_form_field(frame: &mut Frame, area: Rect, label: &str, value: &str, active: bool) {
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

fn help_bar(text: &str) -> Paragraph<'_> {
    let spans = text
        .split("  ")
        .flat_map(|entry| match entry.split_once(':') {
            Some((key, label)) => vec![
                Span::styled(
                    key.to_string(),
                    Style::default()
                        .fg(Theme::Accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(label.to_string(), Style::default().fg(Theme::Muted)),
                Span::raw("  "),
            ],
            None => vec![
                Span::styled(entry.to_string(), Style::default().fg(Theme::Muted)),
                Span::raw("  "),
            ],
        })
        .collect::<Vec<_>>();

    Paragraph::new(Line::from(spans)).style(Style::default().bg(Theme::Panel))
}

fn render_command_bar(app: &App, frame: &mut Frame, area: Rect) {
    let line = if app.input_mode == crate::app::InputMode::Searching {
        let filter_display = if app.search_filter.is_empty() {
            "Type to filter...".to_string()
        } else {
            app.search_filter.clone()
        };
        let filter_style = if app.search_filter.is_empty() {
            Style::default().fg(Theme::Muted)
        } else {
            Style::default().fg(Theme::Text)
        };

        Line::from(vec![
            Span::styled("/ ", Style::default().fg(Theme::Accent)),
            Span::styled(filter_display, filter_style),
            Span::styled(
                "▏",
                Style::default()
                    .fg(Theme::Accent)
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ])
    } else if !app.search_filter.is_empty() {
        let count = app.display_rows.len();
        Line::from(vec![
            Span::styled("/ ", Style::default().fg(Theme::Text)),
            Span::styled(&app.search_filter, Style::default().fg(Theme::Text)),
            Span::styled(
                format!("  ({count} results)  Press / to edit, Esc to clear"),
                Style::default().fg(Theme::Muted),
            ),
        ])
    } else if !app.status_message.is_empty() {
        let is_loading = app.loading || app.github_loading || !app.running_tasks.is_empty();
        let is_progress = app.status_message.starts_with('[');
        let spinner = SPINNER_FRAMES[app.spinner_tick % SPINNER_FRAMES.len()];
        let (icon, color) = if app.status_message.starts_with("Failed")
            || app.status_message.starts_with("Error")
        {
            ("✖", Theme::Error)
        } else if is_loading || is_progress {
            (spinner, Theme::Warning)
        } else {
            ("✔", Theme::Success)
        };

        Line::from(vec![
            Span::styled(format!("{icon} "), Style::default().fg(color)),
            Span::styled(
                app.status_message.as_str(),
                Style::default().fg(Theme::Text),
            ),
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
                        Style::default()
                            .fg(Theme::Accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" "),
                    Span::styled(*label, Style::default().fg(Theme::Muted)),
                ];
                if index < pairs.len() - 1 {
                    s.push(Span::raw("  "));
                }
                s
            })
            .collect();

        if app.inline_new_active() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                "type summary…",
                Style::default().fg(Theme::Muted),
            ));
        }

        if app.loading {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!(
                    "{} Loading…",
                    SPINNER_FRAMES[app.spinner_tick % SPINNER_FRAMES.len()]
                ),
                Style::default().fg(Theme::Muted),
            ));
        } else if let Some(last_updated) = app.last_updated {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!(
                    "updated {} ago",
                    crate::app::format_duration(last_updated.elapsed().as_secs())
                ),
                Style::default().fg(Theme::Muted),
            ));
        }

        Line::from(spans)
    };

    let updated_text = app.last_updated.map(|last_updated| {
        format!(
            "updated {} ago  ",
            crate::app::format_duration(last_updated.elapsed().as_secs())
        )
    });
    let right_width = updated_text.as_ref().map_or(0, |t| t.len() as u16);
    let bar_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(right_width)])
        .split(area);

    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(Theme::Panel)),
        bar_layout[0],
    );
    if let Some(text) = updated_text {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                text,
                Style::default().fg(Theme::Muted),
            )))
            .alignment(Alignment::Right)
            .style(Style::default().bg(Theme::Panel)),
            bar_layout[1],
        );
    }
}

fn status_color(status: &str) -> Style {
    let status = status.to_lowercase();
    if status.contains("done") {
        return Style::default().fg(Theme::Success);
    }
    if status.contains("progress") {
        return Style::default().fg(Theme::Warning);
    }
    if status.contains("review") {
        return Style::default().fg(Theme::Info);
    }
    if status.contains("blocked") || status.contains("rejected") {
        return Style::default().fg(Theme::Error);
    }
    if status.contains("backlog") {
        return Style::default().fg(Theme::Muted);
    }
    if status.contains("todo") || status.contains("to do") {
        return Style::default().fg(Theme::Accent);
    }
    if status.contains("proposed") || status.contains("plan") {
        return Style::default().fg(Theme::Muted);
    }

    Style::default().fg(Theme::Text)
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
