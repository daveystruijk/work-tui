use std::time::Instant;

use crossterm::event::KeyEvent;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::actions::Message;
use crate::app::{InputFocus, RunningAction};
use crate::theme::Theme;

use super::SPINNER_FRAMES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertLevel {
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct StatusAlert {
    pub level: AlertLevel,
    pub message: String,
    pub created_at: Instant,
}

/// Read-only shared state passed to StatusBarView for rendering.
pub struct StatusBarRenderContext<'a> {
    pub input_focus: InputFocus,
    pub search_filter: &'a str,
    pub display_row_count: usize,
    pub running_tasks: &'a [RunningAction],
    pub spinner_tick: usize,
}

#[derive(Debug, Clone, Default)]
pub struct StatusBarView {
    pub alerts: Vec<StatusAlert>,
    pub last_updated: Option<Instant>,
}

impl StatusBarView {
    pub fn set_error(&mut self, message: impl Into<String>) {
        self.alerts = vec![StatusAlert {
            level: AlertLevel::Error,
            message: message.into(),
            created_at: Instant::now(),
        }];
    }

    pub fn set_warning(&mut self, message: impl Into<String>) {
        self.alerts = vec![StatusAlert {
            level: AlertLevel::Warning,
            message: message.into(),
            created_at: Instant::now(),
        }];
    }

    /// Remove alerts older than 3 seconds.
    pub fn expire_alerts(&mut self) {
        self.alerts
            .retain(|alert| alert.created_at.elapsed().as_secs() < 3);
    }

    pub fn handle_message(&mut self, msg: &Message) {
        match msg {
            Message::Myself(Err(err)) => self.set_error(format!("Failed to fetch user: {err}")),
            Message::Issues(Ok(_)) => self.last_updated = Some(Instant::now()),
            Message::Issues(Err(err)) => self.set_error(format!("Failed to load issues: {err}")),
            Message::GithubPrs(_, errors) => {
                self.last_updated = Some(Instant::now());
                if !errors.is_empty() {
                    self.set_error(format!("Failed: {}", errors.join("; ")));
                }
            }
            Message::GithubPrDetail(issue_key, Err(err)) => {
                self.set_error(format!("Failed to load PR detail for {issue_key}: {err}"));
            }
            Message::ConvertedToStory(issue_key, Err(err)) => {
                self.set_error(format!("Failed to convert {issue_key}: {err}"));
            }
            Message::CiLogsFetched(issue_key, Err(err)) => {
                self.set_error(format!("Failed to fetch CI logs for {issue_key}: {err}"));
            }
            Message::FixCiOpened(Err(err)) => self.set_error(format!("Failed to fix CI: {err}")),
            Message::OpenspecProposeOpened(Err(err)) => {
                self.set_error(format!("Failed to open openspec propose: {err}"));
            }
            Message::PickedUp(Err(err)) => {
                self.set_error(format!("Failed to pick up issue: {err}"))
            }
            Message::BranchDiffOpened(Err(err)) => {
                self.set_error(format!("Branch diff failed: {err}"));
            }
            Message::ApproveAutoMerged(Err(err)) => {
                self.set_error(format!("Approve/merge failed: {err}"));
            }
            Message::Finished(Err(err)) => self.set_error(format!("Finish failed: {err}")),
            Message::ChildrenLoaded(parent_key, Err(err)) => {
                self.set_error(format!("Failed to load children for {parent_key}: {err}"));
            }
            Message::LabelAdded(Err(err)) => self.set_error(format!("Failed to add label: {err}")),
            _ => {}
        }
    }

    pub fn footer_height(&self, ctx: &StatusBarRenderContext) -> u16 {
        if self.has_content(ctx) {
            1
        } else {
            0
        }
    }

    fn has_content(&self, ctx: &StatusBarRenderContext) -> bool {
        ctx.input_focus == InputFocus::Search
            || !ctx.search_filter.is_empty()
            || !self.alerts.is_empty()
            || !ctx.running_tasks.is_empty()
            || self.last_updated.is_some()
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, ctx: &StatusBarRenderContext) {
        if !self.has_content(ctx) {
            return;
        }

        let left_text = if ctx.input_focus == InputFocus::Search {
            let filter_display = if ctx.search_filter.is_empty() {
                "Type to filter...".to_string()
            } else {
                ctx.search_filter.to_string()
            };
            let filter_style = if ctx.search_filter.is_empty() {
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
        } else if !self.alerts.is_empty() || !ctx.running_tasks.is_empty() {
            self.render_status_items(ctx)
        } else if !ctx.search_filter.is_empty() {
            let count = ctx.display_row_count;
            Line::from(vec![
                Span::styled("/ ", Style::default().fg(Theme::Text)),
                Span::styled(
                    ctx.search_filter.to_string(),
                    Style::default().fg(Theme::Text),
                ),
                Span::styled(
                    format!("  ({count} results)  Press / to edit, Esc to clear"),
                    Style::default().fg(Theme::Muted),
                ),
            ])
        } else {
            self.render_status_items(ctx)
        };

        let updated_text = self.last_updated.map(|last_updated| {
            format!(
                "updated {} ago  ",
                crate::utils::time::format_duration(last_updated.elapsed().as_secs())
            )
        });
        let right_width = updated_text.as_ref().map_or(0, |text| text.len() as u16);
        let bar_layout = ratatui::layout::Layout::default()
            .direction(ratatui::layout::Direction::Horizontal)
            .constraints([
                ratatui::layout::Constraint::Min(0),
                ratatui::layout::Constraint::Length(right_width),
            ])
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
                .style(Style::default().bg(Theme::Panel))
                .alignment(ratatui::layout::Alignment::Right),
                bar_layout[1],
            );
        }
    }

    fn render_status_items(&self, ctx: &StatusBarRenderContext) -> Line<'static> {
        let spinner = SPINNER_FRAMES[ctx.spinner_tick % SPINNER_FRAMES.len()];
        let mut spans = Vec::new();

        for alert in &self.alerts {
            if !spans.is_empty() {
                spans.push(Span::styled(" • ", Style::default().fg(Theme::Muted)));
            }
            spans.extend(render_alert(alert));
        }

        for action in ctx.running_tasks {
            if !spans.is_empty() {
                spans.push(Span::styled(" • ", Style::default().fg(Theme::Muted)));
            }
            spans.extend(render_running_action(action, spinner));
        }

        Line::from(spans)
    }

    #[allow(dead_code)]
    pub fn update(&mut self, _key_event: KeyEvent) {}
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use ratatui::layout::Rect;

    use crate::app::InputFocus;

    use super::*;
    use crate::fixtures::{render_to_string, test_app};

    #[test]
    fn snapshots_search_mode_status_bar() {
        let mut app = test_app();
        app.input_focus = InputFocus::Search;
        app.list.search_filter = "backend".to_string();
        let ctx = StatusBarRenderContext {
            input_focus: app.input_focus,
            search_filter: &app.list.search_filter,
            display_row_count: app.list.display_rows.len(),
            running_tasks: &app.running_tasks,
            spinner_tick: app.animation.spinner_tick,
        };
        let rendered = render_to_string(48, 1, |frame| {
            app.status_bar.render(frame, Rect::new(0, 0, 48, 1), &ctx);
        });

        assert_snapshot!("status_bar_searching", rendered);
    }

    #[test]
    fn snapshots_loading_status_bar() {
        let mut app = test_app();
        app.running_tasks.push(RunningAction {
            id: "refresh".to_string(),
            label: "Refreshing issues".to_string(),
            progress: None,
        });
        app.animation.spinner_tick = 4;
        let ctx = StatusBarRenderContext {
            input_focus: app.input_focus,
            search_filter: &app.list.search_filter,
            display_row_count: app.list.display_rows.len(),
            running_tasks: &app.running_tasks,
            spinner_tick: app.animation.spinner_tick,
        };
        let rendered = render_to_string(48, 1, |frame| {
            app.status_bar.render(frame, Rect::new(0, 0, 48, 1), &ctx);
        });

        assert_snapshot!("status_bar_loading", rendered);
    }

    #[test]
    fn snapshots_updated_timestamp_status_bar() {
        let mut app = test_app();
        app.status_bar.last_updated =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(90));
        let ctx = StatusBarRenderContext {
            input_focus: app.input_focus,
            search_filter: &app.list.search_filter,
            display_row_count: app.list.display_rows.len(),
            running_tasks: &app.running_tasks,
            spinner_tick: app.animation.spinner_tick,
        };
        let rendered = render_to_string(48, 1, |frame| {
            app.status_bar.render(frame, Rect::new(0, 0, 48, 1), &ctx);
        });

        assert_snapshot!("status_bar_updated", rendered);
    }
}

fn render_running_action(action: &RunningAction, spinner: &str) -> Vec<Span<'static>> {
    let message = match &action.progress {
        Some(progress) if progress.total > 0 => {
            format!(
                "{}: {} ({}/{})",
                action.label, progress.message, progress.current, progress.total
            )
        }
        Some(progress) => format!("{}: {}", action.label, progress.message),
        None => action.label.clone(),
    };

    vec![
        Span::styled(format!("{spinner} "), Style::default().fg(Theme::Warning)),
        Span::styled(message, Style::default().fg(Theme::Text)),
    ]
}

fn render_alert(alert: &StatusAlert) -> Vec<Span<'static>> {
    let (icon, color) = match alert.level {
        AlertLevel::Warning => ("!", Theme::Warning),
        AlertLevel::Error => ("✖", Theme::Error),
    };

    vec![
        Span::styled(format!("{icon} "), Style::default().fg(color)),
        Span::styled(alert.message.clone(), Style::default().fg(Theme::Text)),
    ]
}
