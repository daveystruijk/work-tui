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

/// Which ref a newly picked-up branch should branch off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchBase {
    /// Branch off `origin/main` (the default).
    Main,
    /// Branch off the repo's currently checked-out branch.
    Current,
}

/// Branch-base selection state, only present when the linked repo is currently
/// on a non-trunk branch (so the user has a meaningful choice to make).
#[derive(Debug, Clone)]
pub struct BranchBaseChoice {
    /// Name of the repo's currently checked-out branch.
    pub current_branch: String,
    /// The currently highlighted option.
    pub selected: BranchBase,
}

/// The kind of action the confirm dialog is about to perform.
#[derive(Debug, Clone)]
pub enum ConfirmAction {
    PickUp {
        issue_key: String,
        issue_summary: String,
        issue_description: String,
        repo_path: Option<PathBuf>,
        /// `Some` when the linked repo is on a non-trunk branch and the user
        /// can choose the branch base. `None` means branch off `origin/main`.
        base_choice: Option<BranchBaseChoice>,
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

        let footer = if self.has_branch_base_choice() {
            "←/→:Branch off  Enter:Confirm  Esc:Cancel"
        } else {
            "Enter:Confirm  Esc:Cancel"
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                footer,
                Style::default().fg(Theme::Muted),
            )))
            .style(Style::default().bg(Color::Black)),
            footer_area,
        );
    }

    /// Whether the dialog currently offers a branch-base toggle.
    fn has_branch_base_choice(&self) -> bool {
        matches!(
            &self.action,
            ConfirmAction::PickUp {
                base_choice: Some(_),
                ..
            }
        )
    }

    /// Toggle the branch-base selection between `main` and the current branch.
    fn toggle_branch_base(&mut self) {
        if let ConfirmAction::PickUp {
            base_choice: Some(choice),
            ..
        } = &mut self.action
        {
            choice.selected = match choice.selected {
                BranchBase::Main => BranchBase::Current,
                BranchBase::Current => BranchBase::Main,
            };
        }
    }

    fn build_lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        match &self.action {
            ConfirmAction::PickUp {
                issue_key,
                issue_summary,
                repo_path,
                base_choice,
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
                    if let Some(choice) = base_choice {
                        lines.push(Line::from(""));
                        lines.push(branch_base_line(choice));
                    }
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

/// Render the "branch off" toggle showing `main` and the current branch,
/// highlighting the selected option.
fn branch_base_line(choice: &BranchBaseChoice) -> Line<'static> {
    let main_selected = choice.selected == BranchBase::Main;
    let selected_style = Style::default()
        .fg(Theme::Accent)
        .add_modifier(Modifier::BOLD);
    let unselected_style = Style::default().fg(Theme::Muted);

    let main_label = if main_selected {
        format!("[ {} ]", "main")
    } else {
        format!("  {}  ", "main")
    };
    let current_label = if main_selected {
        format!("  {}  ", choice.current_branch)
    } else {
        format!("[ {} ]", choice.current_branch)
    };

    Line::from(vec![
        Span::styled("  Branch off  ", Style::default().fg(Theme::Muted)),
        Span::styled(
            main_label,
            if main_selected {
                selected_style
            } else {
                unselected_style
            },
        ),
        Span::styled("  ", Style::default().fg(Theme::Muted)),
        Span::styled(
            current_label,
            if main_selected {
                unselected_style
            } else {
                selected_style
            },
        ),
    ])
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
        KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::Char('h' | 'l') => {
            if let Some(dialog) = app.confirm_dialog.as_mut() {
                dialog.toggle_branch_base();
            }
        }
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
            base_choice,
            my_account_id,
            ancestors,
        } => {
            // Resolve the git ref the new branch should branch off. Default to
            // origin/main; only use the current branch when the user selected
            // it in the branch-base toggle.
            let base_ref = match base_choice {
                Some(BranchBaseChoice {
                    current_branch,
                    selected: BranchBase::Current,
                }) => current_branch,
                _ => "origin/main".to_string(),
            };
            actions::pick_up::spawn(
                app.message_tx.clone(),
                app.client.clone(),
                issue_key,
                issue_summary,
                issue_description,
                repo_path,
                base_ref,
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
                base_choice: None,
                my_account_id: "abc123".to_string(),
                ancestors: Vec::new(),
            },
        };

        let output = render_to_string(70, 16, |frame| dialog.render(frame));
        assert_snapshot!(output);
    }

    #[test]
    fn confirm_dialog_pick_up_branch_base_choice() {
        let dialog = ConfirmDialogView {
            action: ConfirmAction::PickUp {
                issue_key: "TEST-42".to_string(),
                issue_summary: "Implement feature X".to_string(),
                issue_description: "Some description".to_string(),
                repo_path: Some(PathBuf::from("/home/user/repos/my-project")),
                base_choice: Some(BranchBaseChoice {
                    current_branch: "TEST-7-some-feature".to_string(),
                    selected: BranchBase::Main,
                }),
                my_account_id: "abc123".to_string(),
                ancestors: Vec::new(),
            },
        };

        let output = render_to_string(70, 18, |frame| dialog.render(frame));
        assert_snapshot!(output);
    }

    #[test]
    fn confirm_dialog_pick_up_branch_base_current_selected() {
        let dialog = ConfirmDialogView {
            action: ConfirmAction::PickUp {
                issue_key: "TEST-42".to_string(),
                issue_summary: "Implement feature X".to_string(),
                issue_description: "Some description".to_string(),
                repo_path: Some(PathBuf::from("/home/user/repos/my-project")),
                base_choice: Some(BranchBaseChoice {
                    current_branch: "TEST-7-some-feature".to_string(),
                    selected: BranchBase::Current,
                }),
                my_account_id: "abc123".to_string(),
                ancestors: Vec::new(),
            },
        };

        let output = render_to_string(70, 18, |frame| dialog.render(frame));
        assert_snapshot!(output);
    }

    #[test]
    fn confirm_dialog_branch_base_toggle() {
        let mut app = test_app();
        app.confirm_dialog = Some(ConfirmDialogView {
            action: ConfirmAction::PickUp {
                issue_key: "TEST-42".to_string(),
                issue_summary: "Implement feature X".to_string(),
                issue_description: String::new(),
                repo_path: Some(PathBuf::from("/tmp/repo")),
                base_choice: Some(BranchBaseChoice {
                    current_branch: "TEST-7-some-feature".to_string(),
                    selected: BranchBase::Main,
                }),
                my_account_id: "abc123".to_string(),
                ancestors: Vec::new(),
            },
        });
        app.input_focus = InputFocus::ConfirmDialog;

        let right = KeyEvent::new(KeyCode::Right, crossterm::event::KeyModifiers::NONE);
        update(&mut app, right);

        let selected = match &app.confirm_dialog.as_ref().unwrap().action {
            ConfirmAction::PickUp {
                base_choice: Some(choice),
                ..
            } => choice.selected,
            _ => panic!("expected pick up with base choice"),
        };
        assert_eq!(selected, BranchBase::Current);

        // Toggling again returns to main.
        let left = KeyEvent::new(KeyCode::Left, crossterm::event::KeyModifiers::NONE);
        update(&mut app, left);
        let selected = match &app.confirm_dialog.as_ref().unwrap().action {
            ConfirmAction::PickUp {
                base_choice: Some(choice),
                ..
            } => choice.selected,
            _ => panic!("expected pick up with base choice"),
        };
        assert_eq!(selected, BranchBase::Main);
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
                base_choice: None,
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
