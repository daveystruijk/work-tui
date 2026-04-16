use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
    Frame,
};

use crate::app::App;
use crate::github::{CheckStatus, MergeableState, PrInfo};
use crate::theme::Theme;

use super::{
    humanize_timestamp, issue_author, issue_field_string, issue_type_icon, labeled_text_line,
    push_wrapped_block, status_color, wrap_text, SIDEBAR_SECTION_MARGIN, SPINNER_FRAMES,
};

pub fn render_sidebar(app: &App, frame: &mut Frame, area: Rect) {
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
    let header_height = super::wrapped_line_count(
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
            let spinner = SPINNER_FRAMES[app.spinner_tick % SPINNER_FRAMES.len()];
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

            let (comments_value, comments_color) = if detail_loading && !detail_loaded {
                (spinner.to_string(), Theme::Warning)
            } else if detail_error.is_some() {
                ("Unavailable".to_string(), Theme::Muted)
            } else {
                let (unresolved, resolved) = comment_counts(pr);
                (
                    format!("{unresolved} unresolved · {resolved} resolved"),
                    Theme::Text,
                )
            };
            github_lines.push(labeled_text_line(
                "Comments",
                comments_value,
                comments_color,
            ));

            if let (Some(files), Some(adds), Some(dels)) =
                (pr.changed_files, pr.additions, pr.deletions)
            {
                github_lines.push(Line::from(vec![
                    Span::styled("Changes   ", Style::default().fg(Theme::Muted)),
                    Span::styled(format!("{files}"), Style::default().fg(Theme::Text)),
                    Span::styled(
                        if files == 1 { " file  " } else { " files  " },
                        Style::default().fg(Theme::Muted),
                    ),
                    Span::styled(format!("+{adds}"), Style::default().fg(Theme::Success)),
                    Span::styled(" / ", Style::default().fg(Theme::Muted)),
                    Span::styled(format!("-{dels}"), Style::default().fg(Theme::Error)),
                ]));
            }

            if let Some(mergeable) = &pr.mergeable {
                let (label, color) = match mergeable {
                    MergeableState::Conflicting => ("Conflicts".to_string(), Theme::Error),
                    MergeableState::Mergeable => ("No conflicts".to_string(), Theme::Success),
                    MergeableState::Unknown => (spinner.to_string(), Theme::Warning),
                };
                github_lines.push(labeled_text_line("Merge", label, color));
            }

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
                            Some(spinner.to_string())
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
                        format!(
                            "{}{} {title} ",
                            top_only_border.horizontal_top, top_only_border.horizontal_top
                        ),
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
