use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::{App, InputMode};
use crate::theme::Theme;

use super::SPINNER_FRAMES;

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use ratatui::layout::Rect;

    use crate::app::InputMode;

    use super::*;

    use crate::fixtures::{render_to_string, test_app};

    #[test]
    fn snapshots_search_mode_status_bar() {
        let mut app = test_app();
        app.input_mode = InputMode::Searching;
        app.search_filter = "backend".to_string();
        let rendered = render_to_string(48, 1, |frame| {
            render_status_bar(&app, frame, Rect::new(0, 0, 48, 1));
        });

        assert_snapshot!("status_bar_searching", rendered);
    }

    #[test]
    fn snapshots_loading_status_bar() {
        let mut app = test_app();
        app.status_message = "Loading...".to_string();
        app.loading = true;
        app.spinner_tick = 4;
        let rendered = render_to_string(48, 1, |frame| {
            render_status_bar(&app, frame, Rect::new(0, 0, 48, 1));
        });

        assert_snapshot!("status_bar_loading", rendered);
    }

    #[test]
    fn snapshots_updated_timestamp_status_bar() {
        let mut app = test_app();
        app.last_updated = Some(std::time::Instant::now() - std::time::Duration::from_secs(90));
        let rendered = render_to_string(48, 1, |frame| {
            render_status_bar(&app, frame, Rect::new(0, 0, 48, 1));
        });

        assert_snapshot!("status_bar_updated", rendered);
    }
}

pub fn footer_height(app: &App) -> u16 {
    if has_content(app) {
        1
    } else {
        0
    }
}

fn has_content(app: &App) -> bool {
    app.input_mode == InputMode::Searching
        || !app.search_filter.is_empty()
        || !app.status_message.is_empty()
        || app.last_updated.is_some()
}

pub fn render_status_bar(app: &App, frame: &mut Frame, area: Rect) {
    if !has_content(app) {
        return;
    }

    let left_text = if app.input_mode == InputMode::Searching {
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
        Line::default()
    };

    let updated_text = app.last_updated.map(|last_updated| {
        format!(
            "updated {} ago  ",
            crate::app::format_duration(last_updated.elapsed().as_secs())
        )
    });
    let right_width = updated_text.as_ref().map_or(0, |text| text.len() as u16);
    let bar_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(0), Constraint::Length(right_width)])
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
            .alignment(Alignment::Right)
            .style(Style::default().bg(Theme::Panel)),
            bar_layout[1],
        );
    }
}
