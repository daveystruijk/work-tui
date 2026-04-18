use std::{
    collections::{HashSet, VecDeque},
    time::Instant,
};

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::actions::ActionMessage;
use crate::app::{App, InputMode};
use crate::theme::Theme;

use super::SPINNER_FRAMES;

#[derive(Debug, Clone, Default)]
pub struct StatusBarState {
    pub message: String,
    pub completed_tasks: VecDeque<(String, usize)>,
    pub last_updated: Option<Instant>,
}

impl StatusBarState {
    pub fn handle_action_message(&mut self, msg: &ActionMessage, running_tasks: &HashSet<String>) {
        match msg {
            ActionMessage::Myself(Err(err)) => {
                self.message = format!("Failed to fetch user: {err}");
            }
            ActionMessage::Issues(Ok(_)) => {
                self.last_updated = Some(Instant::now());
            }
            ActionMessage::Issues(Err(err)) => {
                self.message = format!("Failed to load issues: {err}");
            }
            ActionMessage::GithubPrs(_, errors) => {
                self.last_updated = Some(Instant::now());
                if !errors.is_empty() {
                    self.message = format!("Failed: {}", errors.join("; "));
                    return;
                }
                if running_tasks.is_empty() {
                    self.message = "Ready".to_string();
                }
            }
            ActionMessage::GithubPrDetail(issue_key, Err(err)) => {
                self.message = format!("Failed to load PR detail for {issue_key}: {err}");
            }
            ActionMessage::ConvertedToStory(issue_key, Ok(())) => {
                self.message = format!("Converted {issue_key}");
            }
            ActionMessage::ConvertedToStory(issue_key, Err(err)) => {
                self.message = format!("Failed to convert {issue_key}: {err}");
            }
            ActionMessage::CiLogsFetched(issue_key, Err(err)) => {
                self.message = format!("Failed to fetch CI logs for {issue_key}: {err}");
            }
            ActionMessage::FixCiOpened(Ok(_)) => {
                self.message = "Opened opencode to fix CI".to_string();
            }
            ActionMessage::FixCiOpened(Err(err)) => {
                self.message = format!("Failed to fix CI: {err}");
            }
            ActionMessage::PickedUp(Ok(pickup)) => {
                let skipped_note = if pickup.skipped_opencode {
                    " (skipped opencode: uncommitted changes)"
                } else {
                    ""
                };
                self.message = format!("Picked up {}{}", pickup.branch, skipped_note);
            }
            ActionMessage::PickedUp(Err(err)) => {
                self.message = format!("Failed to pick up issue: {err}");
            }
            ActionMessage::BranchDiffOpened(Ok(branch)) => {
                self.message = format!("Opened diff for {branch}");
            }
            ActionMessage::BranchDiffOpened(Err(err)) => {
                self.message = format!("Branch diff failed: {err}");
            }
            ActionMessage::ApproveAutoMerged(Ok(pr_number)) => {
                self.message = format!("Approved & auto-merge enabled for PR #{pr_number}");
            }
            ActionMessage::ApproveAutoMerged(Err(err)) => {
                self.message = format!("Approve/merge failed: {err}");
            }
            ActionMessage::Finished(Ok(pr_url)) => {
                self.message = format!("PR created: {pr_url}");
            }
            ActionMessage::Finished(Err(err)) => {
                self.message = format!("Finish failed: {err}");
            }
            ActionMessage::ChildrenLoaded(parent_key, Err(err)) => {
                self.message = format!("Failed to load children for {parent_key}: {err}");
            }
            ActionMessage::LabelAdded(Ok((issue_key, label))) => {
                self.message = format!("Added label {label} to {issue_key}");
            }
            ActionMessage::LabelAdded(Err(err)) => {
                self.message = format!("Failed to add label: {err}");
            }
            ActionMessage::Progress(progress) => {
                self.message = progress.to_string();
            }
            _ => {}
        }
    }

    pub fn handle_inline_created(&mut self, key: &str, appeared: bool) {
        self.message = if appeared {
            format!("Created {key}")
        } else {
            format!("Created {key} (may take a moment to appear)")
        };
    }

    pub fn handle_task_started(&mut self, running_tasks: &HashSet<String>) {
        self.refresh_task_message(running_tasks);
    }

    pub fn handle_task_finished(&mut self, name: String, running_tasks: &HashSet<String>) {
        self.completed_tasks.push_back((name, 50));
        self.refresh_task_message(running_tasks);
    }

    pub fn tick_completed_tasks(&mut self) {
        self.completed_tasks.retain_mut(|(_, ticks)| {
            *ticks = ticks.saturating_sub(1);
            *ticks > 0
        });
    }

    fn refresh_task_message(&mut self, running_tasks: &HashSet<String>) {
        if !running_tasks.is_empty() {
            let names: Vec<_> = running_tasks.iter().map(|name| name.as_str()).collect();
            self.message = format!("[{}]", names.join(", "));
            return;
        }

        if let Some((name, _)) = self.completed_tasks.back() {
            self.message = format!("{name} done");
        }
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use ratatui::layout::Rect;

    use crate::app::InputMode;

    use super::*;

    use crate::fixtures::{render_to_string, test_app};

    #[test]
    fn snapshots_search_mode_status_bar() {
        let mut app = test_app();
        app.input_mode = InputMode::Searching;
        app.search_filter = "backend".to_string();
        let rendered = render_to_string(48, 1, |frame| {
            render_status_bar(&app, frame, Rect::new(0, 0, 48, 1));
        });

        assert_snapshot!("status_bar_searching", rendered);
    }

    #[test]
    fn snapshots_loading_status_bar() {
        let mut app = test_app();
        app.status_bar.message = "Loading...".to_string();
        app.loading = true;
        app.animation.spinner_tick = 4;
        let rendered = render_to_string(48, 1, |frame| {
            render_status_bar(&app, frame, Rect::new(0, 0, 48, 1));
        });

        assert_snapshot!("status_bar_loading", rendered);
    }

    #[test]
    fn snapshots_updated_timestamp_status_bar() {
        let mut app = test_app();
        app.status_bar.last_updated =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(90));
        let rendered = render_to_string(48, 1, |frame| {
            render_status_bar(&app, frame, Rect::new(0, 0, 48, 1));
        });

        assert_snapshot!("status_bar_updated", rendered);
    }
}

pub fn footer_height(app: &App) -> u16 {
    if has_content(app) {
        1
    } else {
        0
    }
}

fn has_content(app: &App) -> bool {
    app.input_mode == InputMode::Searching
        || !app.search_filter.is_empty()
        || !app.status_bar.message.is_empty()
        || app.status_bar.last_updated.is_some()
}

pub fn render_status_bar(app: &App, frame: &mut Frame, area: Rect) {
    if !has_content(app) {
        return;
    }

    let left_text = if app.input_mode == InputMode::Searching {
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
    } else if !app.status_bar.message.is_empty() {
        let is_loading = app.loading || app.github_loading || !app.running_tasks.is_empty();
        let status_message = app.status_bar.message.as_str();
        let is_progress = status_message.starts_with('[');
        let spinner = SPINNER_FRAMES[app.animation.spinner_tick % SPINNER_FRAMES.len()];
        let (icon, color) =
            if status_message.starts_with("Failed") || status_message.starts_with("Error") {
                ("✖", Theme::Error)
            } else if is_loading || is_progress {
                (spinner, Theme::Warning)
            } else {
                ("✔", Theme::Success)
            };

        Line::from(vec![
            Span::styled(format!("{icon} "), Style::default().fg(color)),
            Span::styled(status_message, Style::default().fg(Theme::Text)),
        ])
    } else {
        Line::default()
    };

    let updated_text = app.status_bar.last_updated.map(|last_updated| {
        format!(
            "updated {} ago  ",
            crate::app::format_duration(last_updated.elapsed().as_secs())
        )
    });
    let right_width = updated_text.as_ref().map_or(0, |text| text.len() as u16);
    let bar_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(right_width)])
        .split(area);

    frame.render_widget(
        Paragraph::new(left_text).style(Style::default().bg(Theme::Panel)),
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
