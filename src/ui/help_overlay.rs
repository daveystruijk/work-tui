use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};

use crate::app::{AppView, InputFocus};
use crate::theme::Theme;

#[derive(Debug, Clone, Default)]
pub struct HelpOverlayView {
    pub scroll: usize,
}

impl HelpOverlayView {
    pub fn scroll_by(&mut self, delta: isize) {
        self.scroll = (self.scroll as isize + delta).max(0) as usize;
    }

    pub fn render(&self, frame: &mut Frame) {
        let area = popup_rect(frame.area());
        frame.render_widget(Clear, area);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(Line::from(Span::styled(
                " Keybindings ",
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

        let lines = build_help_lines();

        let visible_height = content_area.height as usize;
        let total_lines = lines.len();
        let max_scroll = total_lines.saturating_sub(visible_height);
        let clamped_scroll = self.scroll.min(max_scroll);

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
                "j/k:Scroll  Esc/?:Close",
                Style::default().fg(Theme::Muted),
            )))
            .style(Style::default().bg(Color::Black)),
            footer_area,
        );
    }
}

struct KeybindingSection {
    title: &'static str,
    bindings: &'static [(&'static str, &'static str)],
}

const SECTIONS: &[KeybindingSection] = &[
    KeybindingSection {
        title: "Navigation",
        bindings: &[
            ("j / ↓", "Move down"),
            ("k / ↑", "Move up"),
            ("G", "Go to bottom"),
            ("g g", "Go to top"),
            ("Ctrl-D", "Half page down"),
            ("Ctrl-U", "Half page up"),
            ("h", "Collapse story"),
            ("l", "Expand story"),
            ("Space", "Toggle collapse"),
        ],
    },
    KeybindingSection {
        title: "Actions",
        bindings: &[
            ("p", "Pick up issue"),
            ("f", "Finish issue"),
            ("n", "New issue (inline)"),
            ("o", "Open PR in browser"),
            ("t", "Open issue in browser"),
            ("b", "Branch diff"),
            ("V", "Approve & auto-merge PR"),
            ("S", "Toggle story/task type"),
            ("r", "Refresh"),
        ],
    },
    KeybindingSection {
        title: "Popups",
        bindings: &[
            ("/", "Search"),
            ("a", "Label picker"),
            ("F", "Jira filter picker"),
            ("c", "CI logs"),
            ("i", "Import tasks"),
            ("e", "Openspec propose"),
            ("?", "This help screen"),
        ],
    },
    KeybindingSection {
        title: "Global",
        bindings: &[("Ctrl-C", "Quit")],
    },
];

fn build_help_lines() -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for (section_index, section) in SECTIONS.iter().enumerate() {
        if section_index > 0 {
            lines.push(Line::from(""));
        }

        lines.push(Line::from(Span::styled(
            format!("  {}", section.title),
            Style::default()
                .fg(Theme::Accent)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));

        for (key, description) in section.bindings {
            lines.push(Line::from(vec![
                Span::styled(format!("    {key:<14}"), Style::default().fg(Theme::Text)),
                Span::styled(*description, Style::default().fg(Theme::Muted)),
            ]));
        }
    }

    lines
}

pub fn update(app: &mut AppView, key_event: KeyEvent) {
    match key_event.code {
        KeyCode::Esc | KeyCode::Char('?') => {
            app.help_overlay = None;
            app.input_focus = InputFocus::List;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if let Some(overlay) = app.help_overlay.as_mut() {
                overlay.scroll_by(1);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Some(overlay) = app.help_overlay.as_mut() {
                overlay.scroll_by(-1);
            }
        }
        KeyCode::Char('G') => {
            if let Some(overlay) = app.help_overlay.as_mut() {
                overlay.scroll_by(isize::MAX / 2);
            }
        }
        KeyCode::Char('g') => {
            let previous_was_g = app.previous_key == Some(KeyCode::Char('g'));
            if previous_was_g {
                if let Some(overlay) = app.help_overlay.as_mut() {
                    overlay.scroll = 0;
                }
            }
        }
        _ => {}
    }
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
            Constraint::Percentage(20),
            Constraint::Percentage(60),
            Constraint::Percentage(20),
        ])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;

    use crate::fixtures::render_to_string;

    use super::*;

    #[test]
    fn help_overlay_renders() {
        let overlay = HelpOverlayView::default();
        let output = render_to_string(80, 40, |frame| overlay.render(frame));
        assert_snapshot!(output);
    }
}
