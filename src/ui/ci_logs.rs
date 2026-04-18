use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
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

    let logs_loading = app.ci_logs_loading.contains(&issue_key);

    let area = popup_rect(frame.area());
    frame.render_widget(Clear, area);

    let popup = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(
            " CI Logs ",
            Style::default()
                .fg(Theme::Error)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(ratatui::style::Color::Black))
        .border_style(Style::default().fg(Theme::Error));
    let inner = popup.inner(area);
    frame.render_widget(popup, area);

    let content_width = inner.width.saturating_sub(2) as usize;
    let spinner = SPINNER_FRAMES[app.spinner_tick % SPINNER_FRAMES.len()];

    let mut lines: Vec<Line> = Vec::new();

    for run in &pr.check_runs {
        if run.status != CheckStatus::Fail {
            continue;
        }

        if !lines.is_empty() {
            lines.push(Line::from(""));
        }

        // Run header
        lines.push(Line::from(vec![
            Span::styled(
                " \u{2717} ",
                Style::default()
                    .fg(Theme::Error)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                run.name.clone(),
                Style::default()
                    .fg(Theme::Text)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::from(""));

        // Failed steps summary
        let failed_steps: Vec<_> = run
            .steps
            .iter()
            .filter(|s| s.status == CheckStatus::Fail)
            .collect();
        if !failed_steps.is_empty() {
            for step in &failed_steps {
                lines.push(Line::from(vec![
                    Span::styled("   \u{2717} ", Style::default().fg(Theme::Error)),
                    Span::styled(&step.name, Style::default().fg(Theme::Muted)),
                ]));
            }
            lines.push(Line::from(""));
        }

        // Show spinner while logs are being fetched
        if logs_loading && run.failed_log_excerpt.trim().is_empty() {
            lines.push(Line::from(Span::styled(
                format!("   {spinner} Fetching logs..."),
                Style::default().fg(Theme::Warning),
            )));
            continue;
        }

        // Log content
        if !run.failed_log_excerpt.trim().is_empty() {
            let log_lines: Vec<&str> = run.failed_log_excerpt.trim().lines().collect();
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
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "No failed CI checks",
            Style::default().fg(Theme::Muted),
        )));
    }

    // Layout: body + footer
    let footer_height = 1u16;
    let body_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height.saturating_sub(footer_height + 1),
    };
    let footer_area = Rect {
        x: inner.x,
        y: inner.y + inner.height.saturating_sub(footer_height),
        width: inner.width,
        height: footer_height,
    };

    // Clamp scroll to content
    let visible_height = body_area.height as usize;
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
        Paragraph::new(visible_lines).style(Style::default().bg(ratatui::style::Color::Black)),
        body_area,
    );

    // Scrollbar
    if max_scroll > 0 {
        let scrollbar_area = Rect {
            x: body_area.x + body_area.width.saturating_sub(1),
            y: body_area.y,
            width: 1,
            height: body_area.height,
        };
        let mut scrollbar_state = ScrollbarState::new(max_scroll).position(clamped_scroll);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some(" "))
                .track_style(Style::default().bg(ratatui::style::Color::Black))
                .thumb_style(Style::default().fg(Theme::Muted)),
            scrollbar_area,
            &mut scrollbar_state,
        );
    }

    // Footer
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "j/k:Scroll  G/gg:Top/Bottom  Enter:Fix in opencode  Esc:Close",
            Style::default().fg(Theme::Muted),
        )]))
        .style(Style::default().bg(ratatui::style::Color::Black)),
        footer_area,
    );
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
