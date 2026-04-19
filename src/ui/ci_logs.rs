use std::collections::{HashMap, HashSet};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

use crate::actions::ActionMessage;
use crate::apis::github::CheckStatus;
use crate::apis::github::PrInfo;
use crate::app::AppView;
use crate::theme::Theme;

use super::{wrap_text, SPINNER_FRAMES};

pub async fn handle_input(app: &mut AppView, key_event: KeyEvent) {
    match key_event.code {
        KeyCode::Esc | KeyCode::Char('c') | KeyCode::Char('q') => {
            app.ci_log_popup.close();
            app.input_focus = crate::app::InputFocus::List;
        }
        KeyCode::Enter => app.spawn_fix_ci(),
        KeyCode::Char('j') | KeyCode::Down => app.ci_log_popup.scroll_by(1),
        KeyCode::Char('k') | KeyCode::Up => app.ci_log_popup.scroll_by(-1),
        KeyCode::Char('h') | KeyCode::Left => cycle_tab(app, -1),
        KeyCode::Char('l') | KeyCode::Right => cycle_tab(app, 1),
        KeyCode::Char('d') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            app.ci_log_popup.scroll_by(20);
        }
        KeyCode::Char('u') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            app.ci_log_popup.scroll_by(-20);
        }
        KeyCode::Char('G') => app.ci_log_popup.scroll_by(isize::MAX / 2),
        KeyCode::Char('g')
            if app
                .previous_key
                .is_some_and(|key| key.code == KeyCode::Char('g')) =>
        {
            app.ci_log_popup.scroll = Some(0);
        }
        _ => {}
    }
}

#[derive(Debug, Clone, Default)]
pub struct CiLogsView {
    pub scroll: Option<usize>,
    pub active_tab: usize,
    pub loaded_issues: HashSet<String>,
    pub loading_issues: HashSet<String>,
}

impl CiLogsView {
    pub fn open(&mut self) {
        self.active_tab = 0;
        self.scroll = Some(usize::MAX);
    }

    pub fn close(&mut self) {
        self.scroll = None;
    }

    pub fn scroll_by(&mut self, delta: isize) {
        if let Some(scroll) = self.scroll.as_mut() {
            *scroll = (*scroll as isize + delta).max(0) as usize;
        }
    }

    pub fn clamp_active_tab(&mut self, tab_count: usize) -> usize {
        if tab_count == 0 {
            self.active_tab = 0;
            return 0;
        }

        let active_tab = self.active_tab.min(tab_count - 1);
        self.active_tab = active_tab;
        active_tab
    }

    pub fn cycle_tab(&mut self, delta: isize, tab_count: usize) {
        if tab_count == 0 {
            return;
        }

        let current = self.active_tab as isize;
        self.active_tab = (current + delta).rem_euclid(tab_count as isize) as usize;
        self.scroll = Some(0);
    }

    pub fn handle_action_message(
        &mut self,
        msg: &ActionMessage,
        github_prs: &mut HashMap<String, PrInfo>,
    ) {
        match msg {
            ActionMessage::GithubPrs(_, _) => {
                self.loaded_issues.clear();
                self.loading_issues.clear();
            }
            ActionMessage::CiLogsFetched(issue_key, result) => {
                self.loading_issues.remove(issue_key);
                let Ok(logs) = result else {
                    return;
                };

                if let Some(pr) = github_prs.get_mut(issue_key) {
                    for (run, log) in pr.check_runs.iter_mut().zip(logs.iter()) {
                        run.log_excerpt = log.clone();
                    }
                }
                self.loaded_issues.insert(issue_key.clone());
            }
            _ => {}
        }
    }

    pub fn start_loading(&mut self, issue_key: &str) -> bool {
        if self.loaded_issues.contains(issue_key) || self.loading_issues.contains(issue_key) {
            return false;
        }

        self.loading_issues.insert(issue_key.to_string());
        true
    }
}

pub fn render(app: &mut AppView, frame: &mut Frame) {
    let Some(scroll) = app.ci_log_popup.scroll else {
        return;
    };

    let Some(issue) = app.selected_issue() else {
        return;
    };
    let issue_key = issue.key.clone();

    let Some(pr) = app.github_prs.get(&issue_key) else {
        return;
    };

    let check_runs = &pr.check_runs;
    let logs_loading = app.ci_log_popup.loading_issues.contains(&issue_key);

    let area = popup_rect(frame.area());
    frame.render_widget(Clear, area);

    let title_spans = if check_runs.is_empty() {
        vec![Span::styled(
            " CI Logs ",
            Style::default()
                .fg(Theme::Muted)
                .add_modifier(Modifier::BOLD),
        )]
    } else {
        let active_tab = app.ci_log_popup.clamp_active_tab(check_runs.len());

        let available_title_width = area.width.saturating_sub(2) as usize;
        let separator_width = check_runs.len().saturating_sub(1);
        let max_tab_label_width = available_title_width
            .saturating_sub(1 + separator_width + check_runs.len() * 2)
            .checked_div(check_runs.len())
            .unwrap_or(0)
            .max(1);

        let mut tab_spans: Vec<Span> = Vec::new();
        tab_spans.push(Span::raw(" "));

        for (index, run) in check_runs.iter().enumerate() {
            if index > 0 {
                tab_spans.push(Span::raw(" "));
            }

            let status_color = match run.status {
                CheckStatus::Fail => Theme::Error,
                CheckStatus::Pass => Theme::Success,
                CheckStatus::Pending => Theme::Warning,
            };

            let tab_name = truncate_tab_label(&run.name, max_tab_label_width);
            let tab_style = if index == active_tab {
                Style::default()
                    .bg(status_color)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(status_color)
            };
            tab_spans.push(Span::styled(format!(" {tab_name} "), tab_style));
        }

        tab_spans
    };

    let popup = Block::default()
        .borders(Borders::ALL)
        .title(Line::from(title_spans))
        .style(Style::default().bg(Color::Black))
        .border_style(Style::default().fg(Theme::Muted));
    let inner = popup.inner(area);
    frame.render_widget(popup, area);

    let spinner = SPINNER_FRAMES[app.animation.spinner_tick % SPINNER_FRAMES.len()];
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);

    let content_area = layout[0];
    let footer_area = layout[1];

    let content_width = content_area.width.saturating_sub(2) as usize;

    if check_runs.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "No CI checks",
                Style::default().fg(Theme::Muted),
            )))
            .style(Style::default().bg(Color::Black)),
            content_area,
        );
    } else {
        let active_tab = app.ci_log_popup.clamp_active_tab(check_runs.len());
        let selected_run = &check_runs[active_tab];
        let mut lines: Vec<Line> = Vec::new();

        // Step summary — show failed steps for failed runs, all steps otherwise
        let show_steps: Vec<_> = if selected_run.status == CheckStatus::Fail {
            selected_run
                .steps
                .iter()
                .filter(|step| step.status == CheckStatus::Fail)
                .collect()
        } else {
            selected_run.steps.iter().collect()
        };
        if !show_steps.is_empty() {
            for step in &show_steps {
                let (icon, color) = match step.status {
                    CheckStatus::Fail => ("\u{2717}", Theme::Error),
                    CheckStatus::Pass => ("\u{2713}", Theme::Success),
                    CheckStatus::Pending => ("\u{25cb}", Theme::Warning),
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("   {icon} "), Style::default().fg(color)),
                    Span::styled(&step.name, Style::default().fg(Theme::Muted)),
                ]));
            }
            lines.push(Line::from(""));
        }

        if logs_loading && selected_run.log_excerpt.trim().is_empty() {
            lines.push(Line::from(Span::styled(
                format!("   {spinner} Fetching logs..."),
                Style::default().fg(Theme::Warning),
            )));
        } else if !selected_run.log_excerpt.trim().is_empty() {
            let log_lines: Vec<&str> = selected_run.log_excerpt.trim().lines().collect();
            let line_number_width = log_lines.len().to_string().len();
            for (i, text_line) in log_lines.iter().enumerate() {
                let line_number = format!("{:>width$}", i + 1, width = line_number_width);
                let wrapped_lines = wrap_text(
                    text_line,
                    content_width.saturating_sub(line_number_width + 5),
                    usize::MAX,
                );
                for (j, wrapped) in wrapped_lines.iter().enumerate() {
                    let prefix = if j == 0 {
                        format!("   {line_number} ")
                    } else {
                        format!("   {:width$} ", "", width = line_number_width)
                    };
                    lines.push(Line::from(vec![
                        Span::styled(prefix, Style::default().fg(Theme::Muted)),
                        Span::styled(wrapped.to_string(), Style::default().fg(Theme::Text)),
                    ]));
                }
            }
        } else {
            lines.push(Line::from(Span::styled(
                "   No log output available",
                Style::default().fg(Theme::Muted),
            )));
        }

        let visible_height = content_area.height as usize;
        let total_lines = lines.len();
        let max_scroll = total_lines.saturating_sub(visible_height);
        let clamped_scroll = scroll.min(max_scroll);
        app.ci_log_popup.scroll = Some(clamped_scroll);

        let visible_lines: Vec<Line> = lines
            .into_iter()
            .skip(clamped_scroll)
            .take(visible_height)
            .collect();

        frame.render_widget(
            Paragraph::new(visible_lines).style(Style::default().bg(Color::Black)),
            content_area,
        );

        if max_scroll > 0 {
            let scrollbar_area = Rect {
                x: content_area.x + content_area.width.saturating_sub(1),
                y: content_area.y,
                width: 1,
                height: content_area.height,
            };
            let mut scrollbar_state = ScrollbarState::new(max_scroll).position(clamped_scroll);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None)
                    .track_symbol(Some(" "))
                    .track_style(Style::default().bg(Color::Black))
                    .thumb_style(Style::default().fg(Theme::Muted)),
                scrollbar_area,
                &mut scrollbar_state,
            );
        }
    }

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "h/l:Tab  j/k:Scroll  G/gg:Top/Bottom  Enter:Fix  Esc:Close",
            Style::default().fg(Theme::Muted),
        )))
        .style(Style::default().bg(Color::Black)),
        footer_area,
    );
}

fn cycle_tab(app: &mut AppView, delta: isize) {
    let Some(issue) = app.selected_issue() else {
        return;
    };
    let issue_key = issue.key.clone();
    let Some(pr) = app.github_prs.get(&issue_key) else {
        return;
    };
    let check_run_count = pr.check_runs.len();
    if check_run_count == 0 {
        return;
    }
    app.ci_log_popup.cycle_tab(delta, check_run_count);
}

fn truncate_tab_label(label: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }

    let character_count = label.chars().count();
    if character_count <= max_width {
        return label.to_string();
    }

    if max_width == 1 {
        return "…".to_string();
    }

    let mut truncated: String = label.chars().take(max_width - 1).collect();
    truncated.push('…');
    truncated
}

fn popup_rect(area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(5),
            Constraint::Percentage(90),
            Constraint::Percentage(5),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(10),
            Constraint::Percentage(80),
            Constraint::Percentage(10),
        ])
        .split(vertical[1])[1]
}
