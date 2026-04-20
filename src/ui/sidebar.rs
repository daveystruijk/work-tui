use std::collections::{HashMap, HashSet};

use crossterm::event::KeyEvent;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Wrap},
    Frame,
};

use crate::actions::Message;
use crate::apis::github::{
    CheckRun, CheckStatus, CheckStep, MergeableState, PrInfo, ReviewDecision,
};
use crate::app::AppView;
use crate::theme::Theme;

use super::{
    humanize_timestamp, issue_author, issue_field_string, issue_type_icon, labeled_text_line,
    push_wrapped_block, status_color, SIDEBAR_SECTION_MARGIN, SPINNER_FRAMES,
};

/// Read-only shared state passed to SidebarView for rendering.
pub struct SidebarRenderContext<'a> {
    pub app: &'a AppView,
}

#[derive(Debug, Clone, Default)]
pub struct SidebarView {
    pub detail_loading: HashSet<String>,
    pub detail_loaded: HashSet<String>,
    pub detail_errors: HashMap<String, String>,
}

impl SidebarView {
    pub fn begin_pr_refresh(
        &mut self,
        github_prs: &HashMap<String, PrInfo>,
    ) -> HashMap<String, PrInfo> {
        let previous_prs = self
            .detail_loaded
            .iter()
            .filter_map(|key| github_prs.get(key).map(|pr| (key.clone(), pr.clone())))
            .collect();

        self.detail_loading.clear();
        self.detail_loaded.clear();
        self.detail_errors.clear();

        previous_prs
    }

    pub fn handle_message(&mut self, msg: &Message, github_prs: &mut HashMap<String, PrInfo>) {
        match msg {
            Message::GithubPrDetail(issue_key, result) => {
                self.detail_loading.remove(issue_key);
                match result {
                    Ok(detail) => {
                        if let Some(pr) = github_prs.get_mut(issue_key) {
                            pr.apply_detail(detail.clone());
                            self.detail_loaded.insert(issue_key.clone());
                            self.detail_errors.remove(issue_key);
                        }
                    }
                    Err(err) => {
                        self.detail_errors
                            .insert(issue_key.clone(), err.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    pub fn handle_pr_refresh(
        &mut self,
        github_prs: &mut HashMap<String, PrInfo>,
        previous_prs: &HashMap<String, PrInfo>,
    ) {
        for (issue_key, old_pr) in previous_prs {
            let Some(new_pr) = github_prs.get_mut(issue_key) else {
                continue;
            };
            if !check_runs_changed(&old_pr.check_runs, &new_pr.check_runs) {
                new_pr.apply_detail_from(old_pr);
                self.detail_loaded.insert(issue_key.clone());
            }
        }
    }

    pub fn start_loading_detail(&mut self, issue_key: &str) {
        self.detail_errors.remove(issue_key);
        self.detail_loading.insert(issue_key.to_string());
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, ctx: &SidebarRenderContext) {
        let app = ctx.app;
        let sidebar = Block::default()
            .padding(Padding::new(1, 1, 1, 0))
            .style(Style::default().bg(Theme::SidebarBg));
        let inner = sidebar.inner(area);
        frame.render_widget(sidebar, area);

        let Some(issue) = app.list.selected_issue(&app.issues, &app.story_children) else {
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
                format!("{icon} "),
                Style::default()
                    .fg(Theme::Accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                summary.clone(),
                Style::default()
                    .fg(Theme::Text)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        let header_height =
            super::wrapped_line_count(&format!("{icon} {summary}"), inner.width as usize) as u16;

        let mut jira_lines = Vec::new();
        let status_name = issue.status().map(|status| status.name).unwrap_or_default();
        let status_style = status_color(&status_name);
        let jira_status_color = status_style.fg.unwrap_or(Theme::Muted);
        jira_lines.push(labeled_text_line(
            "Status",
            status_name.clone(),
            jira_status_color,
        ));

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
            let desc_width = inner.width.saturating_sub(6) as usize;
            let max_desc_lines = 20;
            let total_desc_lines =
                super::wrap_text(&description, desc_width.max(1), usize::MAX).len();
            jira_lines.push(Line::from(""));
            push_wrapped_block(
                &mut jira_lines,
                &description,
                desc_width,
                max_desc_lines,
                Theme::Text,
                "",
            );
            let hidden_lines = total_desc_lines.saturating_sub(max_desc_lines);
            if hidden_lines > 0 {
                jira_lines.push(Line::from(Span::styled(
                    format!("(+{hidden_lines} more lines)"),
                    Style::default().fg(Theme::Muted),
                )));
            }
        }

        let mut github_lines = Vec::new();
        let mut github_id: Option<String> = None;
        let mut ci_lines = Vec::new();

        match app.github_prs.get(&issue.key) {
            Some(pr) => {
                let detail_loading = self.detail_loading.contains(&issue.key);
                let detail_error = self.detail_errors.get(&issue.key);
                let detail_loaded = self.detail_loaded.contains(&issue.key);
                let spinner = SPINNER_FRAMES[app.animation.spinner_tick % SPINNER_FRAMES.len()];

                github_id = Some(format!("#{}", pr.number));

                let (status_label, status_color) = if pr.is_draft {
                    ("Draft", Theme::Muted)
                } else if pr.state.eq_ignore_ascii_case("merged") {
                    ("Merged", Theme::Accent)
                } else {
                    match &pr.review_decision {
                        Some(ReviewDecision::Approved) => ("Approved", Theme::Success),
                        Some(ReviewDecision::ChangesRequested) => {
                            ("Changes Requested", Theme::Error)
                        }
                        _ if detail_loading && !detail_loaded => (spinner, Theme::Warning),
                        _ => ("Review", Theme::Info),
                    }
                };
                github_lines.push(labeled_text_line(
                    "Status",
                    status_label.to_string(),
                    status_color,
                ));

                let repo_name = pr
                    .repo_slug
                    .split_once('/')
                    .map(|(_, name)| name)
                    .unwrap_or(&pr.repo_slug);
                github_lines.push(labeled_text_line(
                    "Repo",
                    repo_name.to_string(),
                    Theme::Text,
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
                        Span::styled(format!("+{adds}"), Style::default().fg(Theme::Muted)),
                        Span::styled(" / ", Style::default().fg(Theme::Muted)),
                        Span::styled(format!("-{dels}"), Style::default().fg(Theme::Muted)),
                    ]));
                }

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

                if let Some(mergeable) = &pr.mergeable {
                    let (label, color) = match mergeable {
                        MergeableState::Conflicting => ("Conflicts".to_string(), Theme::Error),
                        MergeableState::Mergeable => ("No conflicts".to_string(), Theme::Success),
                        MergeableState::Unknown => (spinner.to_string(), Theme::Warning),
                    };
                    github_lines.push(labeled_text_line("Merge", label, color));
                }

                let (auto_merge_label, auto_merge_color) = if pr.auto_merge_enabled {
                    ("On", Theme::Success)
                } else {
                    ("Off", Theme::Error)
                };
                github_lines.push(labeled_text_line(
                    "Automerge",
                    auto_merge_label.to_string(),
                    auto_merge_color,
                ));

                if !pr.check_runs.is_empty() {
                    for run in &pr.check_runs {
                        let (icon, color) = match run.status {
                            CheckStatus::Pass => ("✓", Theme::Success),
                            CheckStatus::Fail => ("✗", Theme::Error),
                            CheckStatus::Pending => ("●", Theme::Warning),
                        };
                        let timing = check_run_timing(app, pr, run).unwrap_or_default();
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

                        if run.status != CheckStatus::Pass {
                            for step in &run.steps {
                                let is_running = is_running_check_step(step);
                                let (step_icon, step_color) = match step.status {
                                    CheckStatus::Pass => ("✓", Theme::Success),
                                    CheckStatus::Fail => ("✗", Theme::Error),
                                    CheckStatus::Pending if is_running => (spinner, Theme::Warning),
                                    CheckStatus::Pending => ("●", Theme::Warning),
                                };
                                let step_timing =
                                    check_step_timing(app, pr, run, step).unwrap_or_default();
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
                    }
                } else {
                    ci_lines.push(Line::from(Span::styled(
                        "No CI results",
                        Style::default().fg(Theme::Muted),
                    )));
                }
            }
            None => {}
        }

        let has_github = !github_lines.is_empty();
        let has_ci = !ci_lines.is_empty();

        let mut constraints = vec![
            Constraint::Length(header_height.max(1)),
            Constraint::Length(SIDEBAR_SECTION_MARGIN),
            Constraint::Length(section_height(&jira_lines)),
        ];
        if has_github {
            constraints.push(Constraint::Length(SIDEBAR_SECTION_MARGIN));
            constraints.push(Constraint::Length(section_height(&github_lines)));
        }
        if has_ci {
            constraints.push(Constraint::Length(SIDEBAR_SECTION_MARGIN));
            constraints.push(Constraint::Length(section_height(&ci_lines)));
        }
        constraints.push(Constraint::Min(0));

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(inner);

        let mut chunk_index = 0;

        frame.render_widget(
            Paragraph::new(vec![header_line])
                .style(Style::default().bg(Theme::SidebarBg))
                .wrap(Wrap { trim: false }),
            chunks[chunk_index],
        );
        chunk_index += 2; // skip margin

        render_sidebar_section_with_status(
            frame,
            chunks[chunk_index],
            "Jira",
            jira_lines,
            Theme::SurfaceAlt,
            Some((&issue.key, Theme::Muted)),
        );
        chunk_index += 1;

        if has_github {
            chunk_index += 1; // skip margin
            render_sidebar_section_with_status(
                frame,
                chunks[chunk_index],
                "GitHub",
                github_lines,
                Theme::SurfaceAlt,
                github_id.as_deref().map(|id| (id, Theme::Muted)),
            );
            chunk_index += 1;
        }

        if has_ci {
            chunk_index += 1; // skip margin
            render_sidebar_section(
                frame,
                chunks[chunk_index],
                "CI",
                ci_lines,
                Theme::SurfaceAlt,
            );
        }
    }

    #[allow(dead_code)]
    pub fn update(&mut self, _key_event: KeyEvent) {}
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use ratatui::layout::Rect;

    use crate::fixtures::{render_to_string, sidebar_app, test_app};

    use super::*;

    #[test]
    fn snapshots_empty_sidebar() {
        let app = test_app();
        let ctx = SidebarRenderContext { app: &app };
        let rendered = render_to_string(44, 22, |frame| {
            app.sidebar.render(frame, Rect::new(0, 0, 44, 22), &ctx);
        });

        assert_snapshot!("sidebar_empty", rendered);
    }

    #[test]
    fn snapshots_sidebar_with_pr() {
        let app = sidebar_app();
        let ctx = SidebarRenderContext { app: &app };
        let rendered = render_to_string(44, 26, |frame| {
            app.sidebar.render(frame, Rect::new(0, 0, 44, 26), &ctx);
        });

        assert_snapshot!("sidebar_with_pr", rendered);
    }
}

fn render_sidebar_section<'a>(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    lines: Vec<Line<'a>>,
    border_color: ratatui::style::Color,
) {
    render_sidebar_section_with_status(frame, area, title, lines, border_color, None);
}

fn render_sidebar_section_with_status<'a>(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    lines: Vec<Line<'a>>,
    border_color: ratatui::style::Color,
    status: Option<(&str, ratatui::style::Color)>,
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

    if let Some((status_text, status_color)) = status {
        let padded = format!(" {status_text} ");
        let total_width = padded.len() as u16;
        let right_x = area.right().saturating_sub(total_width + 2);
        if right_x > area.x {
            let status_area = Rect::new(right_x, area.y, total_width, 1);
            frame.render_widget(
                Span::styled(padded, Style::default().fg(status_color)),
                status_area,
            );
        }
    }
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

fn is_running_check_step(step: &crate::apis::github::CheckStep) -> bool {
    step.status == CheckStatus::Pending && step.started_at.is_some() && step.completed_at.is_none()
}

fn check_runs_changed(old_runs: &[CheckRun], new_runs: &[CheckRun]) -> bool {
    if old_runs.len() != new_runs.len() {
        return true;
    }

    for (old, new) in old_runs.iter().zip(new_runs.iter()) {
        if old.name != new.name || old.status != new.status {
            return true;
        }
    }

    false
}

/// Compute a timing string for a single check run.
fn check_run_timing(app: &AppView, pr: &PrInfo, run: &CheckRun) -> Option<String> {
    match run.status {
        CheckStatus::Pass | CheckStatus::Fail => run.completed_at.as_deref().map(|completed| {
            let elapsed = crate::utils::time::parse_duration_secs(
                run.started_at.as_deref().unwrap_or(completed),
                completed,
            );
            match elapsed {
                Some(secs) => crate::utils::time::format_duration(secs),
                None => "done".to_string(),
            }
        }),
        CheckStatus::Pending => {
            let elapsed = run
                .started_at
                .as_deref()
                .and_then(crate::utils::time::elapsed_since_iso)?;
            let cache_key = format!("{}/{}", pr.repo_slug, run.name);
            let eta = app.check_durations.get(&cache_key).map(|&historical| {
                let remaining = historical.saturating_sub(elapsed);
                format!(" (~{})", crate::utils::time::format_duration(remaining))
            });
            Some(format!(
                "{}{}",
                crate::utils::time::format_duration(elapsed),
                eta.unwrap_or_default()
            ))
        }
    }
}

/// Compute a timing string for a single check step.
fn check_step_timing(
    app: &AppView,
    pr: &PrInfo,
    run: &CheckRun,
    step: &CheckStep,
) -> Option<String> {
    match step.status {
        CheckStatus::Pass | CheckStatus::Fail => step.completed_at.as_deref().map(|completed| {
            let elapsed = crate::utils::time::parse_duration_secs(
                step.started_at.as_deref().unwrap_or(completed),
                completed,
            );
            match elapsed {
                Some(secs) => crate::utils::time::format_duration(secs),
                None => "done".to_string(),
            }
        }),
        CheckStatus::Pending => {
            let elapsed = step
                .started_at
                .as_deref()
                .and_then(crate::utils::time::elapsed_since_iso)?;
            let cache_key = format!("{}/{}/{}", pr.repo_slug, run.name, step.name);
            let eta = app.check_durations.get(&cache_key).map(|&historical| {
                let remaining = historical.saturating_sub(elapsed);
                format!(" (~{})", crate::utils::time::format_duration(remaining))
            });
            Some(format!(
                "{}{}",
                crate::utils::time::format_duration(elapsed),
                eta.unwrap_or_default()
            ))
        }
    }
}
