use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::{app::AppView, repos::RepoEntry, theme::Theme};

use super::centered_rect;

#[derive(Debug, Clone, Default)]
pub struct LabelPickerView {
    pub selected: usize,
    pub filter: String,
}

impl LabelPickerView {
    pub fn open() -> Self { Self::default() }

    pub fn filtered_repo_entries<'a>(&self, repo_entries: &'a [RepoEntry]) -> Vec<&'a RepoEntry> {
        if self.filter.is_empty() {
            return repo_entries.iter().collect();
        }

        let query = self.filter.to_lowercase();
        repo_entries
            .iter()
            .filter(|entry| entry.label.to_lowercase().contains(&query))
            .collect()
    }

    pub fn move_selection(&mut self, repo_entries: &[RepoEntry], down: bool) {
        let count = self.filtered_repo_entries(repo_entries).len();
        if count == 0 {
            self.selected = 0;
            return;
        }
        if down {
            self.selected = (self.selected + 1).min(count - 1);
            return;
        }
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn selected_entry<'a>(&self, repo_entries: &'a [RepoEntry]) -> Option<&'a RepoEntry> {
        self.filtered_repo_entries(repo_entries).get(self.selected).copied()
    }

    pub fn type_char(&mut self, ch: char) {
        self.filter.push(ch);
        self.selected = 0;
    }

    pub fn backspace(&mut self) {
        self.filter.pop();
        self.selected = 0;
    }
}

pub async fn handle_input(app: &mut AppView, key_event: KeyEvent) {
    match key_event.code {
        KeyCode::Esc => {
            app.label_picker = None;
            app.input_focus = crate::app::InputFocus::List;
        }
        KeyCode::Enter => {
            if app.add_label_from_picker() {
                app.label_picker = None;
                app.input_focus = crate::app::InputFocus::List;
            }
        }
        KeyCode::Backspace => {
            if let Some(picker) = app.label_picker.as_mut() {
                picker.backspace();
            }
        }
        KeyCode::Down => {
            if let Some(picker) = app.label_picker.as_mut() {
                picker.move_selection(&app.repo_entries, true);
            }
        }
        KeyCode::Up => {
            if let Some(picker) = app.label_picker.as_mut() {
                picker.move_selection(&app.repo_entries, false);
            }
        }
        KeyCode::Char(c) => {
            if let Some(picker) = app.label_picker.as_mut() {
                picker.type_char(c);
            }
        }
        _ => {}
    }
}

pub fn render(app: &AppView, frame: &mut Frame) {
    let Some(picker) = &app.label_picker else {
        return;
    };
    let area = centered_rect(60, 70, frame.area());
    frame.render_widget(Clear, area);

    let popup = Block::bordered()
        .title(Span::styled(
            " Add repo label ",
            Style::default().fg(Theme::Accent).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(Theme::Surface))
        .border_style(Style::default().fg(Theme::Accent));
    let inner = popup.inner(area);
    frame.render_widget(popup, area);

    let filtered = picker.filtered_repo_entries(&app.repo_entries);
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
                        Style::default().fg(Theme::Text).add_modifier(Modifier::BOLD),
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
        .constraints([Constraint::Length(3), Constraint::Min(3), Constraint::Length(3)])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("/ ", Style::default().fg(Theme::Accent)),
            Span::styled(filter_display, filter_style),
        ])),
        modal_layout[0],
    );
    frame.render_stateful_widget(list, modal_layout[1], &mut state);
}
