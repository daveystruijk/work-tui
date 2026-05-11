use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
    apis::jira::JiraProject,
    app::{AppView, InputFocus, JiraFilterState},
    theme::Theme,
};

use super::{adjust_scroll_offset, centered_rect, move_selected_index};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum FilterPane {
    #[default]
    Projects,
    Statuses,
}

#[derive(Debug, Clone, Default)]
pub struct FilterPickerView {
    active_pane: FilterPane,
    project_selected: usize,
    status_selected: usize,
    project_area_height: u16,
    status_area_height: u16,
    project_scroll_offset: usize,
    status_scroll_offset: usize,
    project_filter: String,
    project_search_active: bool,
    draft_project_key: Option<String>,
    draft_status_names: HashSet<String>,
    draft_auto_tag_enabled_project_keys: HashSet<String>,
}

impl FilterPickerView {
    pub fn filtered_projects<'a>(&self, filter_state: &'a JiraFilterState) -> Vec<&'a JiraProject> {
        if self.project_filter.is_empty() {
            return filter_state.available_projects.iter().collect();
        }

        let mut matcher = Matcher::new(Config::DEFAULT);
        let pattern = Pattern::new(
            &self.project_filter,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );

        let mut scored: Vec<_> = filter_state
            .available_projects
            .iter()
            .filter_map(|project| {
                let label = format!("{} {}", project.key, project.name);
                let mut buffer = Vec::new();
                let haystack = Utf32Str::new(&label, &mut buffer);
                pattern
                    .score(haystack, &mut matcher)
                    .map(|score| (score, project))
            })
            .collect();
        scored.sort_by(|left, right| right.0.cmp(&left.0));
        scored.into_iter().map(|(_, project)| project).collect()
    }

    fn sync_project_selection(&mut self, filter_state: &JiraFilterState) {
        let filtered = self.filtered_projects(filter_state);
        if filtered.is_empty() {
            self.project_selected = 0;
            self.project_scroll_offset = 0;
            self.status_selected = 0;
            self.status_scroll_offset = 0;
            self.draft_project_key = None;
            self.draft_status_names.clear();
            return;
        }

        self.project_selected = self.project_selected.min(filtered.len() - 1);
        self.adjust_project_scroll_offset(filtered.len());
        let project_key = filtered[self.project_selected].key.clone();
        if self.draft_project_key.as_deref() == Some(project_key.as_str()) {
            return;
        }

        self.draft_project_key = Some(project_key.clone());
        self.status_selected = 0;
        self.status_scroll_offset = 0;
        self.draft_status_names =
            if filter_state.selected_project_key.as_deref() == Some(project_key.as_str()) {
                filter_state.selected_status_names.iter().cloned().collect()
            } else {
                default_status_names_for_project(filter_state, &project_key)
                    .into_iter()
                    .collect()
            };
    }

    fn selected_status_names(&self, filter_state: &JiraFilterState) -> Vec<String> {
        let Some(project_key) = self.draft_project_key.as_deref() else {
            return Vec::new();
        };
        available_statuses_for_project(filter_state, project_key)
            .iter()
            .filter(|status| self.draft_status_names.contains(&status.name))
            .map(|status| status.name.clone())
            .collect()
    }

    pub fn hydrate_status_selection(&mut self, project_key: &str, status_names: Vec<String>) {
        if self.draft_project_key.as_deref() != Some(project_key)
            || !self.draft_status_names.is_empty()
        {
            return;
        }
        self.draft_status_names = status_names.into_iter().collect();
    }

    fn move_project_selection(&mut self, filter_state: &JiraFilterState, down: bool) {
        let count = self.filtered_projects(filter_state).len();
        if count == 0 {
            self.project_selected = 0;
            self.sync_project_selection(filter_state);
            return;
        }
        move_selected_index(
            &mut self.project_selected,
            count,
            if down { 1 } else { -1 },
        );
        self.adjust_project_scroll_offset(count);
        self.sync_project_selection(filter_state);
    }

    fn move_status_selection(&mut self, filter_state: &JiraFilterState, down: bool) {
        let Some(project_key) = self.draft_project_key.as_deref() else {
            self.status_selected = 0;
            self.status_scroll_offset = 0;
            return;
        };
        let count = available_statuses_for_project(filter_state, project_key).len();
        if count == 0 {
            self.status_selected = 0;
            self.status_scroll_offset = 0;
            return;
        }
        move_selected_index(
            &mut self.status_selected,
            count,
            if down { 1 } else { -1 },
        );
        self.adjust_status_scroll_offset(count);
    }

    fn toggle_selected_status(&mut self, filter_state: &JiraFilterState) {
        let Some(project_key) = self.draft_project_key.as_deref() else {
            return;
        };
        let Some(status) =
            available_statuses_for_project(filter_state, project_key).get(self.status_selected)
        else {
            return;
        };

        if !self.draft_status_names.remove(&status.name) {
            self.draft_status_names.insert(status.name.clone());
        }
    }

    fn toggle_all_statuses(&mut self, filter_state: &JiraFilterState) {
        let Some(project_key) = self.draft_project_key.as_deref() else {
            return;
        };
        let status_names = status_names_for_project(filter_state, project_key);
        if status_names.is_empty() {
            return;
        }
        let all_selected = status_names
            .iter()
            .all(|status_name| self.draft_status_names.contains(status_name));
        if all_selected {
            self.draft_status_names.clear();
            return;
        }
        self.draft_status_names = status_names.into_iter().collect();
    }

    fn type_project_filter(&mut self, filter_state: &JiraFilterState, character: char) {
        self.project_filter.push(character);
        self.project_selected = 0;
        self.project_scroll_offset = 0;
        self.sync_project_selection(filter_state);
    }

    fn is_auto_tagging_enabled_for_draft_project(&self) -> bool {
        let Some(project_key) = self.draft_project_key.as_deref() else {
            return false;
        };
        self.draft_auto_tag_enabled_project_keys.contains(project_key)
    }

    fn toggle_auto_tagging_for_draft_project(&mut self) {
        let Some(project_key) = self.draft_project_key.clone() else {
            return;
        };
        if !self.draft_auto_tag_enabled_project_keys.remove(&project_key) {
            self.draft_auto_tag_enabled_project_keys.insert(project_key);
        }
    }

    fn backspace_project_filter(&mut self, filter_state: &JiraFilterState) {
        self.project_filter.pop();
        self.project_selected = 0;
        self.project_scroll_offset = 0;
        self.sync_project_selection(filter_state);
    }

    fn adjust_project_scroll_offset(&mut self, item_count: usize) {
        adjust_scroll_offset(
            &mut self.project_scroll_offset,
            self.project_selected,
            self.project_area_height,
            item_count,
        );
    }

    fn adjust_status_scroll_offset(&mut self, item_count: usize) {
        adjust_scroll_offset(
            &mut self.status_scroll_offset,
            self.status_selected,
            self.status_area_height,
            item_count,
        );
    }

    pub fn render(&mut self, frame: &mut Frame, filter_state: &JiraFilterState) {
        let area = centered_rect(84, 80, frame.area());
        frame.render_widget(Clear, area);

        let popup = Block::bordered()
            .title(Span::styled(
                " Jira filters ",
                Style::default()
                    .fg(Theme::Accent)
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(Theme::Surface))
            .border_style(Style::default().fg(Theme::Accent));
        let inner = popup.inner(area);
        frame.render_widget(popup, area);

        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(3)])
            .split(inner);

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(44), Constraint::Percentage(56)])
            .split(layout[0]);

        let projects = self.filtered_projects(filter_state);
        let project_items: Vec<ListItem> = if projects.is_empty() {
            vec![ListItem::new(Line::from(Span::styled(
                "No matching projects",
                Style::default().fg(Theme::Muted),
            )))]
        } else {
            projects
                .iter()
                .map(|project| {
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("{:<10}", project.key),
                            Style::default()
                                .fg(Theme::Accent)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(project.name.clone(), Style::default().fg(Theme::Text)),
                    ]))
                })
                .collect()
        };

        let project_block = Block::bordered()
            .title(Span::styled(
                " Projects ",
                pane_title_style(self.active_pane == FilterPane::Projects),
            ))
            .border_style(pane_border_style(self.active_pane == FilterPane::Projects))
            .style(Style::default().bg(Theme::Surface));
        let project_inner = project_block.inner(columns[0]);
        frame.render_widget(project_block, columns[0]);

        let project_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .split(project_inner);
        self.project_area_height = project_layout[0].height;

        let mut project_state = ListState::default();
        if !projects.is_empty() {
            self.adjust_project_scroll_offset(projects.len());
            project_state = project_state
                .with_offset(self.project_scroll_offset)
                .with_selected(Some(self.project_selected));
        }
        let project_list = List::new(project_items).highlight_style(
            Style::default()
                .fg(Theme::Panel)
                .bg(Theme::AccentSoft)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_stateful_widget(project_list, project_layout[0], &mut project_state);
        self.project_scroll_offset = project_state.offset();

        let project_filter_display = if self.project_filter.is_empty() {
            "search projects".to_string()
        } else {
            self.project_filter.clone()
        };
        let project_filter_style = if self.project_filter.is_empty() {
            Style::default().fg(Theme::Muted)
        } else {
            Style::default().fg(Theme::Text)
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("/ ", Style::default().fg(Theme::Accent)),
                Span::styled(project_filter_display, project_filter_style),
                Span::styled(
                    if self.project_search_active { "▏" } else { "" },
                    Style::default()
                        .fg(Theme::Accent)
                        .add_modifier(Modifier::SLOW_BLINK),
                ),
            ])),
            project_layout[1],
        );

        let statuses = self
            .draft_project_key
            .as_deref()
            .map(|project_key| available_statuses_for_project(filter_state, project_key))
            .unwrap_or(&[]);
        let status_items: Vec<ListItem> =
            if let Some(project_key) = self.draft_project_key.as_deref() {
                if statuses.is_empty() {
                    vec![ListItem::new(Line::from(Span::styled(
                        format!("Loading statuses for {project_key}..."),
                        Style::default().fg(Theme::Muted),
                    )))]
                } else {
                    statuses
                        .iter()
                        .map(|status| {
                            let checked = if self.draft_status_names.contains(&status.name) {
                                "[x]"
                            } else {
                                "[ ]"
                            };
                            ListItem::new(Line::from(vec![
                                Span::styled(checked, Style::default().fg(Theme::Accent)),
                                Span::raw(" "),
                                Span::styled(status.name.clone(), Style::default().fg(Theme::Text)),
                            ]))
                        })
                        .collect()
                }
            } else {
                vec![ListItem::new(Line::from(Span::styled(
                    "Select a project first",
                    Style::default().fg(Theme::Muted),
                )))]
            };

        let status_count = self.selected_status_names(filter_state).len();
        let mut status_state = ListState::default();
        let status_block = Block::bordered()
            .title(Span::styled(
                format!(" Statuses ({status_count}) "),
                pane_title_style(self.active_pane == FilterPane::Statuses),
            ))
            .border_style(pane_border_style(self.active_pane == FilterPane::Statuses))
            .style(Style::default().bg(Theme::Surface));
        let status_inner = status_block.inner(columns[1]);
        frame.render_widget(status_block, columns[1]);
        self.status_area_height = status_inner.height;
        if !statuses.is_empty() {
            self.status_selected = self.status_selected.min(statuses.len() - 1);
            self.adjust_status_scroll_offset(statuses.len());
            status_state = status_state
                .with_offset(self.status_scroll_offset)
                .with_selected(Some(self.status_selected));
        }
        let status_list = List::new(status_items).highlight_style(
            Style::default()
                .fg(Theme::Panel)
                .bg(Theme::AccentSoft)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_stateful_widget(status_list, status_inner, &mut status_state);
        self.status_scroll_offset = status_state.offset();

        let auto_tag_label = if self.is_auto_tagging_enabled_for_draft_project() {
            "[x] Auto-tag repo labels"
        } else {
            "[ ] Auto-tag repo labels"
        };

        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled("/", Style::default().fg(Theme::Accent)),
                    Span::styled(" search  ", Style::default().fg(Theme::Muted)),
                    Span::styled("Tab", Style::default().fg(Theme::Accent)),
                    Span::styled(" switch pane  ", Style::default().fg(Theme::Muted)),
                    Span::styled("Enter", Style::default().fg(Theme::Accent)),
                    Span::styled(" apply  ", Style::default().fg(Theme::Muted)),
                    Span::styled("Esc", Style::default().fg(Theme::Accent)),
                    Span::styled(" cancel", Style::default().fg(Theme::Muted)),
                ]),
                Line::from(vec![
                    Span::styled("Space", Style::default().fg(Theme::Accent)),
                    Span::styled(" toggle status  ", Style::default().fg(Theme::Muted)),
                    Span::styled("a", Style::default().fg(Theme::Accent)),
                    Span::styled(" toggle all  ", Style::default().fg(Theme::Muted)),
                    Span::styled("T", Style::default().fg(Theme::Accent)),
                    Span::styled(" auto-tag", Style::default().fg(Theme::Muted)),
                ]),
                Line::from(vec![Span::styled(
                    auto_tag_label,
                    Style::default().fg(Theme::Text),
                )]),
            ]),
            layout[1],
        );
    }
}

fn available_statuses_for_project<'a>(
    filter_state: &'a JiraFilterState,
    project_key: &str,
) -> &'a [crate::apis::jira::JiraStatus] {
    filter_state
        .available_statuses
        .get(project_key)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn status_names_for_project(filter_state: &JiraFilterState, project_key: &str) -> Vec<String> {
    available_statuses_for_project(filter_state, project_key)
        .iter()
        .map(|status| status.name.clone())
        .collect()
}

fn default_status_names_for_project(
    filter_state: &JiraFilterState,
    project_key: &str,
) -> Vec<String> {
    let statuses = available_statuses_for_project(filter_state, project_key);
    if statuses.is_empty() {
        return Vec::new();
    }

    let excluded: HashSet<_> = crate::app::DEFAULT_HIDDEN_JIRA_STATUSES
        .iter()
        .map(|status_name| status_name.to_string())
        .collect();
    let mut selected: Vec<String> = statuses
        .iter()
        .filter(|status| !excluded.contains(&status.name.to_ascii_lowercase()))
        .map(|status| status.name.clone())
        .collect();

    if selected.is_empty() {
        selected = statuses.iter().map(|status| status.name.clone()).collect();
    }

    selected
}

fn pane_title_style(is_active: bool) -> Style {
    if is_active {
        return Style::default()
            .fg(Theme::Accent)
            .add_modifier(Modifier::BOLD);
    }
    Style::default().fg(Theme::Muted)
}

fn pane_border_style(is_active: bool) -> Style {
    if is_active {
        return Style::default().fg(Theme::Accent);
    }
    Style::default().fg(Theme::Muted)
}

pub fn open(app: &mut AppView) {
    if app.jira_filter.available_projects.is_empty() {
        app.status_bar.set_warning("No Jira projects available");
        return;
    }

    let mut picker = FilterPickerView::default();
    picker.draft_auto_tag_enabled_project_keys = app.jira_filter.auto_tag_enabled_project_keys.clone();
    if let Some(current_project_key) = app.current_project_key() {
        picker.project_selected = app
            .jira_filter
            .available_projects
            .iter()
            .position(|project| project.key == current_project_key)
            .unwrap_or(0);
    }
    picker.sync_project_selection(&app.jira_filter);
    if let Some(project_key) = picker.draft_project_key.as_deref() {
        let project_key = project_key.to_string();
        app.spawn_project_statuses(&project_key);
    }

    app.filter_picker = Some(picker);
    app.input_focus = InputFocus::JiraFilterPicker;
}

pub async fn update(app: &mut AppView, key_event: KeyEvent) {
    let project_search_active = app
        .filter_picker
        .as_ref()
        .map(|picker| picker.project_search_active)
        .unwrap_or(false);

    if project_search_active {
        update_project_search(app, key_event);
        return;
    }

    match key_event.code {
        KeyCode::Esc => close(app),
        KeyCode::Char('/') => start_project_search(app),
        KeyCode::Tab | KeyCode::Right => switch_pane(app, true),
        KeyCode::Char('h') => switch_pane(app, false),
        KeyCode::Char('l') => switch_pane(app, true),
        KeyCode::Left => switch_pane(app, false),
        KeyCode::Char('j') => move_selection(app, true),
        KeyCode::Char('k') => move_selection(app, false),
        KeyCode::Up => move_selection(app, false),
        KeyCode::Down => move_selection(app, true),
        KeyCode::Backspace => {
            let filter_state = app.jira_filter.clone();
            if let Some(picker) = app.filter_picker.as_mut() {
                if picker.active_pane == FilterPane::Projects {
                    picker.backspace_project_filter(&filter_state);
                    ensure_draft_statuses_loaded(app);
                }
            }
        }
        KeyCode::Enter => {
            let active_pane = app
                .filter_picker
                .as_ref()
                .map(|picker| picker.active_pane)
                .unwrap_or_default();
            if active_pane == FilterPane::Projects {
                switch_pane(app, true);
                ensure_draft_statuses_loaded(app);
                return;
            }
            apply(app);
        }
        KeyCode::Char(' ') => {
            let filter_state = app.jira_filter.clone();
            if let Some(picker) = app.filter_picker.as_mut() {
                if picker.active_pane == FilterPane::Statuses {
                    picker.toggle_selected_status(&filter_state);
                }
            }
        }
        KeyCode::Char('T') => toggle_auto_tagging(app),
        KeyCode::Char('a') => {
            let filter_state = app.jira_filter.clone();
            if let Some(picker) = app.filter_picker.as_mut() {
                if picker.active_pane == FilterPane::Statuses {
                    picker.toggle_all_statuses(&filter_state);
                    return;
                }
            }
        }
        _ => {}
    }
}

fn update_project_search(app: &mut AppView, key_event: KeyEvent) {
    match key_event.code {
        KeyCode::Esc => stop_project_search(app, false),
        KeyCode::Enter => stop_project_search(app, true),
        KeyCode::Backspace => backspace_project_filter(app),
        KeyCode::Char(character) => type_project_filter(app, character, key_event.modifiers),
        _ => {}
    }
}

fn switch_pane(app: &mut AppView, forward: bool) {
    let Some(picker) = app.filter_picker.as_mut() else {
        return;
    };
    picker.active_pane = match picker.active_pane {
        FilterPane::Projects if forward => FilterPane::Statuses,
        FilterPane::Statuses if !forward => FilterPane::Projects,
        active_pane => active_pane,
    };
}

fn move_selection(app: &mut AppView, down: bool) {
    let filter_state = app.jira_filter.clone();
    let Some(active_pane) = app.filter_picker.as_ref().map(|picker| picker.active_pane) else {
        return;
    };
    let Some(picker) = app.filter_picker.as_mut() else {
        return;
    };

    if active_pane == FilterPane::Projects {
        picker.move_project_selection(&filter_state, down);
        ensure_draft_statuses_loaded(app);
        return;
    }
    picker.move_status_selection(&filter_state, down);
}

fn start_project_search(app: &mut AppView) {
    let Some(picker) = app.filter_picker.as_mut() else {
        return;
    };
    picker.project_search_active = true;
    picker.active_pane = FilterPane::Projects;
}

fn stop_project_search(app: &mut AppView, keep_filter: bool) {
    let Some(picker) = app.filter_picker.as_mut() else {
        return;
    };
    picker.project_search_active = false;
    if keep_filter {
        return;
    }
    if picker.project_filter.is_empty() {
        return;
    }
    picker.project_filter.clear();
    let filter_state = app.jira_filter.clone();
    picker.project_selected = 0;
    picker.sync_project_selection(&filter_state);
    ensure_draft_statuses_loaded(app);
}

fn backspace_project_filter(app: &mut AppView) {
    let filter_state = app.jira_filter.clone();
    let Some(picker) = app.filter_picker.as_mut() else {
        return;
    };
    picker.backspace_project_filter(&filter_state);
    ensure_draft_statuses_loaded(app);
}

fn type_project_filter(app: &mut AppView, character: char, modifiers: KeyModifiers) {
    if modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
        return;
    }

    let filter_state = app.jira_filter.clone();
    let Some(picker) = app.filter_picker.as_mut() else {
        return;
    };
    picker.type_project_filter(&filter_state, character);
    ensure_draft_statuses_loaded(app);
}

fn ensure_draft_statuses_loaded(app: &mut AppView) {
    let project_key = app
        .filter_picker
        .as_ref()
        .and_then(|picker| picker.draft_project_key.clone());
    if let Some(project_key) = project_key {
        app.spawn_project_statuses(&project_key);
    }
}

fn apply(app: &mut AppView) {
    let Some(picker) = app.filter_picker.as_ref() else {
        return;
    };
    let Some(project_key) = picker.draft_project_key.clone() else {
        app.status_bar.set_warning("No Jira project selected");
        return;
    };
    let selected_status_names = picker.selected_status_names(&app.jira_filter);
    if selected_status_names.is_empty() {
        app.status_bar
            .set_warning("Select at least one Jira status");
        return;
    }

    app.jira_filter.auto_tag_enabled_project_keys =
        picker.draft_auto_tag_enabled_project_keys.clone();
    app.filter_picker = None;
    app.input_focus = InputFocus::List;
    app.apply_jira_filter(project_key, selected_status_names);
}

fn toggle_auto_tagging(app: &mut AppView) {
    let Some(picker) = app.filter_picker.as_mut() else {
        return;
    };
    picker.toggle_auto_tagging_for_draft_project();
}

fn close(app: &mut AppView) {
    app.filter_picker = None;
    app.input_focus = InputFocus::List;
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;

    use crate::{
        apis::jira::{JiraProject, JiraStatus},
        fixtures::{render_to_string, test_app},
    };

    use super::*;

    fn filter_app() -> AppView {
        let mut app = test_app();
        app.jira_filter.available_projects = vec![
            JiraProject {
                id: "1".into(),
                key: "INI".into(),
                name: "Internal Initiative".into(),
            },
            JiraProject {
                id: "2".into(),
                key: "OPS".into(),
                name: "Operations".into(),
            },
            JiraProject {
                id: "3".into(),
                key: "WEB".into(),
                name: "Website".into(),
            },
        ];
        app.jira_filter.selected_project_key = Some("INI".into());
        app.jira_filter.selected_status_names = vec!["In Progress".into(), "Review".into()];
        app.jira_filter.available_statuses.insert(
            "INI".into(),
            vec![
                JiraStatus {
                    id: "1".into(),
                    name: "Backlog".into(),
                },
                JiraStatus {
                    id: "2".into(),
                    name: "In Progress".into(),
                },
                JiraStatus {
                    id: "3".into(),
                    name: "Review".into(),
                },
                JiraStatus {
                    id: "4".into(),
                    name: "Done".into(),
                },
            ],
        );
        app
    }

    #[test]
    fn filter_picker_snapshot() {
        let mut app = filter_app();
        open(&mut app);
        let output = render_to_string(100, 28, |frame| {
            app.filter_picker
                .as_mut()
                .unwrap()
                .render(frame, &app.jira_filter)
        });
        assert_snapshot!(output);
    }

    #[test]
    fn filter_picker_snapshot_with_project_filter() {
        let mut app = filter_app();
        open(&mut app);
        let filter_state = app.jira_filter.clone();
        let picker = app.filter_picker.as_mut().unwrap();
        picker.type_project_filter(&filter_state, 'w');
        picker.type_project_filter(&filter_state, 'e');
        let output = render_to_string(100, 28, |frame| {
            app.filter_picker
                .as_mut()
                .unwrap()
                .render(frame, &app.jira_filter)
        });
        assert_snapshot!(output);
    }
}
