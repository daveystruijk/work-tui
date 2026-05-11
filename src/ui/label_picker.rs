use crossterm::event::{KeyCode, KeyEvent};
use nucleo_matcher::{
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
    Config, Matcher, Utf32Str,
};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::{
    actions,
    app::AppView,
    repos::{self, RepoEntry},
    theme::Theme,
};

use super::centered_rect;

#[derive(Debug, Clone, Default)]
pub struct LabelPickerView {
    pub selected: usize,
    pub filter: String,
}

impl LabelPickerView {
    pub fn filtered_repo_entries<'a>(&self, repo_entries: &'a [RepoEntry]) -> Vec<&'a RepoEntry> {
        if self.filter.is_empty() {
            return repo_entries.iter().collect();
        }

        let mut matcher = Matcher::new(Config::DEFAULT);
        let pattern = Pattern::new(
            &self.filter,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );

        let mut scored: Vec<_> = repo_entries
            .iter()
            .filter_map(|entry| {
                let mut buffer = Vec::new();
                let haystack = Utf32Str::new(&entry.label, &mut buffer);
                pattern
                    .score(haystack, &mut matcher)
                    .map(|score| (score, entry))
            })
            .collect();

        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.into_iter().map(|(_, entry)| entry).collect()
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
        self.filtered_repo_entries(repo_entries)
            .get(self.selected)
            .copied()
    }

    pub fn type_char(&mut self, ch: char) {
        self.filter.push(ch);
        self.selected = 0;
    }

    pub fn backspace(&mut self) {
        self.filter.pop();
        self.selected = 0;
    }

    pub fn render(&self, frame: &mut Frame, repo_entries: &[RepoEntry]) {
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

        let filtered = self.filtered_repo_entries(repo_entries);
        let items: Vec<ListItem> = if filtered.is_empty() {
            let msg = if repo_entries.is_empty() {
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
            state.select(Some(self.selected));
        }
        let list = List::new(items).highlight_style(
            Style::default()
                .fg(Theme::Panel)
                .bg(Theme::AccentSoft)
                .add_modifier(Modifier::BOLD),
        );

        let filter_display = format!(
            " {} ",
            if self.filter.is_empty() {
                "Type to filter...".to_string()
            } else {
                self.filter.clone()
            }
        );
        let filter_style = if self.filter.is_empty() {
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
            ])),
            modal_layout[0],
        );
        frame.render_stateful_widget(list, modal_layout[1], &mut state);
    }
}

pub async fn update(app: &mut AppView, key_event: KeyEvent) {
    match key_event.code {
        KeyCode::Esc => {
            app.label_picker = None;
            app.input_focus = crate::app::InputFocus::List;
        }
        KeyCode::Enter => {
            if add_label_from_picker(app) {
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

fn add_label_from_picker(app: &mut AppView) -> bool {
    let Some(entry) = app
        .label_picker
        .as_ref()
        .and_then(|picker| picker.selected_entry(&app.repo_entries))
        .cloned()
    else {
        app.status_bar.set_warning("No repository selected");
        return false;
    };
    let Some(issue) = app.selected_ticket() else {
        app.status_bar.set_warning("No issue selected");
        return false;
    };
    let issue_key = issue.issue.key.clone();
    let labels = issue.issue.labels();
    let target_normalized = repos::normalize_label(&entry.label);
    let already_has = labels
        .iter()
        .any(|l| repos::normalize_label(l) == target_normalized);
    if already_has {
        app.status_bar
            .set_warning(format!("{issue_key} already labeled with {}", entry.label));
        return false;
    }
    actions::add_label::spawn(
        app.message_tx.clone(),
        app.client.clone(),
        issue_key,
        entry.label.clone(),
        labels,
    );
    true
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use insta::assert_snapshot;

    use crate::{fixtures::render_to_string, repos::RepoEntry};

    use super::*;

    fn sample_entries() -> Vec<RepoEntry> {
        vec![
            RepoEntry {
                label: "frontend-app".to_string(),
                normalized: "frontend-app".to_string(),
                path: PathBuf::from("/home/user/repos/frontend-app"),
                github_slug: Some("org/frontend-app".to_string()),
            },
            RepoEntry {
                label: "backend-api".to_string(),
                normalized: "backend-api".to_string(),
                path: PathBuf::from("/home/user/repos/backend-api"),
                github_slug: Some("org/backend-api".to_string()),
            },
            RepoEntry {
                label: "billing-service".to_string(),
                normalized: "billing-service".to_string(),
                path: PathBuf::from("/home/user/repos/billing-service"),
                github_slug: Some("org/billing-service".to_string()),
            },
            RepoEntry {
                label: "data-pipeline".to_string(),
                normalized: "data-pipeline".to_string(),
                path: PathBuf::from("/home/user/repos/data-pipeline"),
                github_slug: Some("org/data-pipeline".to_string()),
            },
        ]
    }

    #[test]
    fn label_picker_with_entries() {
        let picker = LabelPickerView::default();
        let entries = sample_entries();
        let output = render_to_string(60, 20, |frame| picker.render(frame, &entries));
        assert_snapshot!(output);
    }

    #[test]
    fn label_picker_fuzzy_filter() {
        let mut picker = LabelPickerView::default();
        // "bka" should fuzzy-match "backend-api" (b...k from back, a from api)
        for ch in "bka".chars() {
            picker.type_char(ch);
        }
        let entries = sample_entries();
        let output = render_to_string(60, 20, |frame| picker.render(frame, &entries));
        assert_snapshot!(output);
    }

    #[test]
    fn label_picker_fuzzy_filter_no_matches() {
        let mut picker = LabelPickerView::default();
        for ch in "zzzzz".chars() {
            picker.type_char(ch);
        }
        let entries = sample_entries();
        let output = render_to_string(60, 20, |frame| picker.render(frame, &entries));
        assert_snapshot!(output);
    }
}
