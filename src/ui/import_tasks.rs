use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

use crate::actions::import_tasks::TaskEntry;
use crate::app::AppView;
use crate::theme::Theme;

use super::wrap_text;

pub fn handle_input(app: &mut AppView, key_event: KeyEvent) {
    match key_event.code {
        KeyCode::Esc => {
            app.import_tasks_popup = None;
            app.input_focus = crate::app::InputFocus::List;
        }
        KeyCode::Enter => app.confirm_import_tasks(),
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(popup) = app.import_tasks_popup.as_mut() {
                popup.scroll_by(1);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Some(popup) = app.import_tasks_popup.as_mut() {
                popup.scroll_by(-1);
            }
        }
        _ => {}
    }
}

#[derive(Debug, Clone)]
pub struct ImportTasksView {
    pub tasks: Vec<TaskEntry>,
    pub tasks_path: PathBuf,
    pub issue_key: String,
    pub issue_type_name: String,
    pub project_key: String,
    pub scroll: usize,
}

impl ImportTasksView {
    pub fn scroll_by(&mut self, delta: isize) {
        self.scroll = (self.scroll as isize + delta).max(0) as usize;
    }
}

pub fn render(app: &AppView, frame: &mut Frame) {
    let Some(popup) = &app.import_tasks_popup else {
        return;
    };

    let pending_tasks: Vec<&TaskEntry> = popup.tasks.iter().filter(|t| t.key.is_none()).collect();

    let area = popup_rect(frame.area());
    frame.render_widget(Clear, area);

    let title = format!(
        " Import {} task{} into {} ",
        pending_tasks.len(),
        if pending_tasks.len() == 1 { "" } else { "s" },
        popup.issue_key,
    );

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
    let content_width = content_area.width.saturating_sub(4) as usize;

    let mut lines: Vec<Line> = Vec::new();

    if pending_tasks.len() == 1 {
        lines.push(Line::from(Span::styled(
            "  Will update the current issue with:",
            Style::default().fg(Theme::Muted),
        )));
        lines.push(Line::from(""));
    } else {
        lines.push(Line::from(Span::styled(
            "  Will create subtasks under this issue:",
            Style::default().fg(Theme::Muted),
        )));
        lines.push(Line::from(""));
    }

    for (index, task) in pending_tasks.iter().enumerate() {
        let number = format!("  {}. ", index + 1);
        lines.push(Line::from(vec![
            Span::styled(number, Style::default().fg(Theme::Muted)),
            Span::styled(
                &task.title,
                Style::default()
                    .fg(Theme::Text)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        if !task.description.is_empty() {
            let wrapped = wrap_text(&task.description, content_width.saturating_sub(5), 4);
            for wrapped_line in wrapped {
                lines.push(Line::from(Span::styled(
                    format!("     {wrapped_line}"),
                    Style::default().fg(Theme::Muted),
                )));
            }
        }

        lines.push(Line::from(""));
    }

    let skipped_count = popup.tasks.iter().filter(|t| t.key.is_some()).count();
    if skipped_count > 0 {
        lines.push(Line::from(Span::styled(
            format!(
                "  ({skipped_count} task{} already imported, skipped)",
                if skipped_count == 1 { "" } else { "s" }
            ),
            Style::default().fg(Theme::Muted),
        )));
        lines.push(Line::from(""));
    }

    let visible_height = content_area.height as usize;
    let total_lines = lines.len();
    let max_scroll = total_lines.saturating_sub(visible_height);
    let clamped_scroll = popup.scroll.min(max_scroll);

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

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Enter:Confirm  j/k:Scroll  Esc:Cancel",
            Style::default().fg(Theme::Muted),
        )))
        .style(Style::default().bg(Color::Black)),
        footer_area,
    );
}

fn popup_rect(area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(10),
            Constraint::Percentage(80),
            Constraint::Percentage(10),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(15),
            Constraint::Percentage(70),
            Constraint::Percentage(15),
        ])
        .split(vertical[1])[1]
}
