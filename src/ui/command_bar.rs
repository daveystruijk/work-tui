use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::App;
use crate::theme::Theme;

use super::SPINNER_FRAMES;

pub fn render_command_bar(app: &App, frame: &mut Frame, area: Rect) {
    let line = if app.input_mode == crate::app::InputMode::Searching {
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
    } else if !app.status_message.is_empty() {
        let is_loading = app.loading || app.github_loading || !app.running_tasks.is_empty();
        let is_progress = app.status_message.starts_with('[');
        let spinner = SPINNER_FRAMES[app.spinner_tick % SPINNER_FRAMES.len()];
        let (icon, color) = if app.status_message.starts_with("Failed")
            || app.status_message.starts_with("Error")
        {
            ("✖", Theme::Error)
        } else if is_loading || is_progress {
            (spinner, Theme::Warning)
        } else {
            ("✔", Theme::Success)
        };

        Line::from(vec![
            Span::styled(format!("{icon} "), Style::default().fg(color)),
            Span::styled(
                app.status_message.as_str(),
                Style::default().fg(Theme::Text),
            ),
        ])
    } else {
        let pairs: &[(&str, &str)] = if app.inline_new_active() {
            &[("Esc", "Cancel"), ("↵", "Create")]
        } else {
            &[
                ("^C", "Quit"),
                ("↵", "View"),
                ("/", "Search"),
                ("o", "PR"),
                ("t", "Ticket"),
                ("p", "Pick up"),
                ("f", "Finish"),
                ("n", "New"),
                ("a", "Label"),
                ("r", "Refresh"),
            ]
        };

        let mut spans: Vec<Span> = pairs
            .iter()
            .enumerate()
            .flat_map(|(index, (key, label))| {
                let mut s = vec![
                    Span::styled(
                        *key,
                        Style::default()
                            .fg(Theme::Accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" "),
                    Span::styled(*label, Style::default().fg(Theme::Muted)),
                ];
                if index < pairs.len() - 1 {
                    s.push(Span::raw("  "));
                }
                s
            })
            .collect();

        if app.inline_new_active() {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                "type summary…",
                Style::default().fg(Theme::Muted),
            ));
        }

        if app.loading {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!(
                    "{} Loading…",
                    SPINNER_FRAMES[app.spinner_tick % SPINNER_FRAMES.len()]
                ),
                Style::default().fg(Theme::Muted),
            ));
        } else if let Some(last_updated) = app.last_updated {
            spans.push(Span::raw("  "));
            spans.push(Span::styled(
                format!(
                    "updated {} ago",
                    crate::app::format_duration(last_updated.elapsed().as_secs())
                ),
                Style::default().fg(Theme::Muted),
            ));
        }

        Line::from(spans)
    };

    let updated_text = app.last_updated.map(|last_updated| {
        format!(
            "updated {} ago  ",
            crate::app::format_duration(last_updated.elapsed().as_secs())
        )
    });
    let right_width = updated_text.as_ref().map_or(0, |t| t.len() as u16);
    let bar_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(right_width)])
        .split(area);

    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(Theme::Panel)),
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
