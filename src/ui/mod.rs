mod ci_logs;
mod import_tasks;
mod list;
mod sidebar;
mod status_bar;

pub use ci_logs::CiLogPopupState;
pub use import_tasks::ImportTasksPopupState;
pub use list::ListViewState;
pub use sidebar::SidebarState;
pub use status_bar::StatusBarState;

use std::collections::HashMap;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    Frame,
};

use crate::theme::Theme;
use crate::{
    apis::jira::{Issue, User},
    app::App,
};

pub const COLUMNS: &[&str] = &["Key", "Summary", "PR", "CI", "Status", "Assignee", "Repo"];
pub const SIDEBAR_SECTION_MARGIN: u16 = 1;

pub type CellMap<'a> = HashMap<&'static str, Line<'a>>;

pub const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[derive(Debug, Clone, Default)]
pub struct UiAnimationState {
    pub spinner_tick: usize,
}

impl UiAnimationState {
    pub fn tick_spinner(&mut self) {
        self.spinner_tick = self.spinner_tick.wrapping_add(1);
    }
}

/// Max content width for a named column across all rows.
pub fn max_col_width(row_data: &[(CellMap, Style)], name: &str) -> u16 {
    row_data
        .iter()
        .map(|(cells, _)| cells.get(name).map_or(0, |l| l.width() as u16))
        .max()
        .unwrap_or(0)
}

pub fn render(app: &mut App, frame: &mut Frame) {
    let area = frame.area();

    // Horizontal split: list column (flexible) | sidebar column (fixed 44 wide)
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(44)])
        .split(area);

    // Vertical split on the list column only: list area | footer
    let footer_height = status_bar::footer_height(app);
    let list_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(footer_height)])
        .split(columns[0]);

    list::render_list(app, frame, list_chunks[0]);

    if footer_height > 0 {
        status_bar::render_status_bar(app, frame, list_chunks[1]);
    }

    // Sidebar gets the full height (no footer)
    sidebar::render_sidebar(app, frame, columns[1]);

    // Popup overlays
    use crate::app::InputFocus;
    match app.input_focus {
        InputFocus::ImportTasksPopup => import_tasks::render_import_tasks_popup(app, frame),
        InputFocus::CiLogPopup => ci_logs::render_ci_log_popup(app, frame),
        InputFocus::LabelPicker => {} // rendered inside render_list
        _ => {}
    }
}

pub fn labeled_text_line(
    label: &str,
    value: String,
    color: ratatui::style::Color,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<10}"), Style::default().fg(Theme::Muted)),
        Span::styled(value, Style::default().fg(color)),
    ])
}

pub fn issue_field_string(issue: &Issue, field: &str) -> Option<String> {
    issue.field::<String>(field).and_then(|result| result.ok())
}

pub fn issue_author(issue: &Issue) -> Option<String> {
    issue
        .field::<User>("creator")
        .and_then(|result| result.ok())
        .map(|user| user.display_name)
}

pub fn humanize_timestamp(timestamp: &str) -> String {
    let parsed = chrono::DateTime::parse_from_rfc3339(timestamp)
        .or_else(|_| chrono::DateTime::parse_from_str(timestamp, "%Y-%m-%dT%H:%M:%S%.3f%z"));
    let Ok(parsed) = parsed else {
        return timestamp.to_string();
    };
    let local = parsed.with_timezone(&chrono::Local);
    local.format("%Y-%m-%d %H:%M").to_string()
}

pub fn wrap_text(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    if width == 0 || max_lines == 0 {
        return Vec::new();
    }

    let mut wrapped = Vec::new();
    let paragraphs = if text.trim().is_empty() {
        vec![String::new()]
    } else {
        text.lines()
            .map(str::trim)
            .map(ToString::to_string)
            .collect()
    };

    'outer: for paragraph in paragraphs {
        if paragraph.is_empty() {
            wrapped.push(String::new());
            if wrapped.len() >= max_lines {
                break;
            }
            continue;
        }

        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let candidate_width = if current.is_empty() {
                word.len()
            } else {
                current.len() + 1 + word.len()
            };

            if candidate_width > width && !current.is_empty() {
                wrapped.push(current);
                if wrapped.len() >= max_lines {
                    break 'outer;
                }
                current = word.to_string();
                continue;
            }

            if word.len() > width && current.is_empty() {
                wrapped.push(
                    word.chars()
                        .take(width.saturating_sub(1))
                        .collect::<String>()
                        + "…",
                );
                if wrapped.len() >= max_lines {
                    break 'outer;
                }
                continue;
            }

            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }

        if !current.is_empty() {
            wrapped.push(current);
            if wrapped.len() >= max_lines {
                break;
            }
        }
    }

    if wrapped.len() == max_lines && !text.trim().is_empty() {
        if let Some(last) = wrapped.last_mut() {
            if !last.ends_with('…') {
                last.push('…');
            }
        }
    }

    wrapped
}

pub fn wrapped_line_count(text: &str, width: usize) -> usize {
    if text.is_empty() {
        return 1;
    }

    wrap_text(text, width.max(1), usize::MAX).len().max(1)
}

pub fn push_wrapped_block<'a>(
    lines: &mut Vec<Line<'a>>,
    text: &str,
    width: usize,
    max_lines: usize,
    color: ratatui::style::Color,
    prefix: &str,
) {
    for line in wrap_text(text, width, max_lines) {
        lines.push(Line::from(Span::styled(
            format!("{prefix}{line}"),
            Style::default().fg(color),
        )));
    }
}

pub fn status_color(status: &str) -> Style {
    let status = status.to_lowercase();
    if status.contains("done") {
        return Style::default().fg(Theme::Success);
    }
    if status.contains("progress") {
        return Style::default().fg(Theme::Warning);
    }
    if status.contains("review") {
        return Style::default().fg(Theme::Info);
    }
    if status.contains("blocked") || status.contains("rejected") {
        return Style::default().fg(Theme::Error);
    }
    if status.contains("backlog") {
        return Style::default().fg(Theme::Muted);
    }
    if status.contains("todo") || status.contains("to do") {
        return Style::default().fg(Theme::Accent);
    }
    if status.contains("proposed") || status.contains("plan") {
        return Style::default().fg(Theme::Muted);
    }

    Style::default().fg(Theme::Text)
}

pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1]);

    horizontal[1]
}

pub fn issue_type_icon(issue_type: &str) -> &'static str {
    let issue_type = issue_type.to_lowercase();
    if issue_type.contains("bug") {
        return "¤";
    }
    if issue_type.contains("story") || issue_type.contains("epic") {
        return "§";
    }
    if issue_type.contains("sub") {
        return "↳";
    }
    if issue_type.contains("task") {
        return "◦";
    }

    "•"
}
