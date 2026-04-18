use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

use crate::apis::github::CheckStatus;
use crate::app::App;
use crate::theme::Theme;

use super::{wrap_text, SPINNER_FRAMES};

pub fn render_ci_log_popup(app: &mut App, frame: &mut Frame) {
    let Some(scroll) = app.ci_log_popup_scroll else {
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
    let logs_loading = app.ci_logs_loading.contains(&issue_key);

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
        let active_tab = app.ci_log_popup_tab.min(check_runs.len() - 1);
        app.ci_log_popup_tab = active_tab;

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

    let spinner = SPINNER_FRAMES[app.spinner_tick % SPINNER_FRAMES.len()];
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
        let active_tab = app.ci_log_popup_tab.min(check_runs.len() - 1);
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
        app.ci_log_popup_scroll = Some(clamped_scroll);

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
