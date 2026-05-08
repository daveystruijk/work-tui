use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::actions;
use crate::apis::jira::Issue;
use crate::app::{AppView, InputFocus};
use crate::theme::Theme;

/// The kind of action the confirm dialog is about to perform.
#[derive(Debug, Clone)]
pub enum ConfirmAction {
    PickUp {
        issue_key: String,
        issue_summary: String,
        issue_description: String,
        repo_path: Option<PathBuf>,
        my_account_id: String,
        ancestors: Vec<Issue>,
    },
    Finish {
        issue_key: String,
        issue_summary: String,
        repo_path: PathBuf,
    },
    BranchDiff {
        issue_key: String,
        repo_path: PathBuf,
    },
}

impl ConfirmAction {
    fn title(&self) -> &'static str {
        match self {
            ConfirmAction::PickUp { .. } => "Pick Up",
            ConfirmAction::Finish { .. } => "Finish",
            ConfirmAction::BranchDiff { .. } => "Branch Diff",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConfirmDialogView {
    pub action: ConfirmAction,
}

impl ConfirmDialogView {
    pub fn render(&self, frame: &mut Frame) {
        let lines = self.build_lines();
        let content_height = lines.len() as u16 + 4; // borders + footer + padding
        let content_width = 60u16;

        let area = centered_fixed_rect(content_width, content_height, frame.area());
        frame.render_widget(Clear, area);

        let title = format!(" {} ", self.action.title());
        let block = Block::default()
            .borders(Borders::ALL)
            .title(Line::from(Span::styled(
                title,
                Style::default()
                    .fg(Theme::Accent)
                    .add_modifier(Modifier::BOLD),
            )))
            .style(Style::default().bg(Color::Black))
            .border_style(Style::default().fg(Theme::Muted));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(inner);

        let content_area = layout[0];
        let footer_area = layout[1];

        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(Color::Black)),
            content_area,
        );

        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Enter:Confirm  Esc:Cancel",
                Style::default().fg(Theme::Muted),
            )))
            .style(Style::default().bg(Color::Black)),
            footer_area,
        );
    }

    fn build_lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        match &self.action {
            ConfirmAction::PickUp {
                issue_key,
                issue_summary,
                repo_path,
                ..
            } => {
                lines.push(Line::from(""));
                lines.push(labeled_line("Action", "Pick up issue"));
                lines.push(labeled_line(
                    "Issue",
                    &format!("{issue_key} {issue_summary}"),
                ));
                if let Some(path) = repo_path {
                    let repo_name = path
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.display().to_string());
                    lines.push(labeled_line("Repo", &repo_name));
                    lines.push(Line::from(""));
                    lines.push(detail_line("Will checkout branch, then assign issue,"));
                    lines.push(detail_line("transition to In Progress, open editor"));
                } else {
                    lines.push(Line::from(""));
                    lines.push(detail_line("Will assign issue and transition"));
                    lines.push(detail_line("to In Progress (no repo linked)"));
                }
                lines.push(Line::from(""));
            }
            ConfirmAction::Finish {
                issue_key,
                issue_summary,
                repo_path,
            } => {
                let repo_name = repo_path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| repo_path.display().to_string());
                lines.push(Line::from(""));
                lines.push(labeled_line("Action", "Finish issue"));
                lines.push(labeled_line(
                    "Issue",
                    &format!("{issue_key} {issue_summary}"),
                ));
                lines.push(labeled_line("Repo", &repo_name));
                lines.push(Line::from(""));
                lines.push(detail_line("Will commit changes, push branch,"));
                lines.push(detail_line("create PR, transition to Review"));
                lines.push(Line::from(""));
            }
            ConfirmAction::BranchDiff {
                issue_key,
                repo_path,
            } => {
                let repo_name = repo_path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| repo_path.display().to_string());
                lines.push(Line::from(""));
                lines.push(labeled_line("Action", "Open branch diff"));
                lines.push(labeled_line("Issue", issue_key));
                lines.push(labeled_line("Repo", &repo_name));
                lines.push(Line::from(""));
                lines.push(detail_line("Will checkout branch and open diff"));
                lines.push(detail_line("against origin/main in tmux"));
                lines.push(Line::from(""));
            }
        }

        lines
    }
}

fn labeled_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<10}"), Style::default().fg(Theme::Muted)),
        Span::styled(value.to_string(), Style::default().fg(Theme::Text)),
    ])
}

fn detail_line(text: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {text}"),
        Style::default().fg(Theme::Muted),
    ))
}

fn centered_fixed_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

pub fn update(app: &mut AppView, key_event: KeyEvent) {
    match key_event.code {
        KeyCode::Esc => {
            app.confirm_dialog = None;
            app.input_focus = InputFocus::List;
        }
        KeyCode::Enter => confirm_action(app),
        _ => {}
    }
}

fn confirm_action(app: &mut AppView) {
    let Some(dialog) = app.confirm_dialog.take() else {
        return;
    };
    app.input_focus = InputFocus::List;

    match dialog.action {
        ConfirmAction::PickUp {
            issue_key,
            issue_summary,
            issue_description,
            repo_path,
            my_account_id,
            ancestors,
        } => {
            actions::pick_up::spawn(
                app.message_tx.clone(),
                app.client.clone(),
                issue_key,
                issue_summary,
                issue_description,
                repo_path,
                my_account_id,
                ancestors,
            );
        }
        ConfirmAction::Finish {
            issue_key,
            issue_summary,
            repo_path,
        } => {
            actions::finish::spawn(
                app.message_tx.clone(),
                app.client.clone(),
                issue_key,
                issue_summary,
                repo_path,
            );
        }
        ConfirmAction::BranchDiff {
            issue_key,
            repo_path,
        } => {
            actions::branch_diff::spawn(app.message_tx.clone(), issue_key, repo_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;

    use crate::fixtures::{render_to_string, test_app};

    use super::*;

    #[test]
    fn confirm_dialog_pick_up_with_repo() {
        let dialog = ConfirmDialogView {
            action: ConfirmAction::PickUp {
                issue_key: "TEST-42".to_string(),
                issue_summary: "Implement feature X".to_string(),
                issue_description: "Some description".to_string(),
                repo_path: Some(PathBuf::from("/home/user/repos/my-project")),
                my_account_id: "abc123".to_string(),
                ancestors: Vec::new(),
            },
        };

        let output = render_to_string(70, 16, |frame| dialog.render(frame));
        assert_snapshot!(output);
    }

    #[test]
    fn confirm_dialog_finish() {
        let dialog = ConfirmDialogView {
            action: ConfirmAction::Finish {
                issue_key: "TEST-42".to_string(),
                issue_summary: "Implement feature X".to_string(),
                repo_path: PathBuf::from("/home/user/repos/my-project"),
            },
        };

        let output = render_to_string(70, 16, |frame| dialog.render(frame));
        assert_snapshot!(output);
    }

    #[test]
    fn confirm_dialog_branch_diff() {
        let dialog = ConfirmDialogView {
            action: ConfirmAction::BranchDiff {
                issue_key: "TEST-42".to_string(),
                repo_path: PathBuf::from("/home/user/repos/my-project"),
            },
        };

        let output = render_to_string(70, 16, |frame| dialog.render(frame));
        assert_snapshot!(output);
    }

    #[test]
    fn confirm_dialog_pick_up_without_repo() {
        let dialog = ConfirmDialogView {
            action: ConfirmAction::PickUp {
                issue_key: "TEST-99".to_string(),
                issue_summary: "No repo task".to_string(),
                issue_description: String::new(),
                repo_path: None,
                my_account_id: "abc123".to_string(),
                ancestors: Vec::new(),
            },
        };

        let output = render_to_string(70, 14, |frame| dialog.render(frame));
        assert_snapshot!(output);
    }

    #[test]
    fn confirm_dialog_esc_cancels() {
        let mut app = test_app();
        app.confirm_dialog = Some(ConfirmDialogView {
            action: ConfirmAction::BranchDiff {
                issue_key: "TEST-1".to_string(),
                repo_path: PathBuf::from("/tmp/repo"),
            },
        });
        app.input_focus = InputFocus::ConfirmDialog;

        let key = KeyEvent::new(KeyCode::Esc, crossterm::event::KeyModifiers::NONE);
        update(&mut app, key);

        assert!(app.confirm_dialog.is_none());
        assert_eq!(app.input_focus, InputFocus::List);
    }
}
