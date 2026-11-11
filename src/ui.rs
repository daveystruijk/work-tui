use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::app::{App, Mode};
use crate::model::{self, CheckState, Issue, Kind, Pr, Review, Row};

const MUTED: Color = Color::DarkGray;
const SELECTION: Color = Color::Rgb(66, 66, 88);
const PARENT_SHADE: Color = Color::Rgb(42, 42, 55); // sets parents apart from the tickets under them
const DIFF_OPEN: Color = Color::Rgb(30, 50, 40); // the issue's diff is open in a pane
const WORKTREE_OPEN: Color = Color::Rgb(34, 44, 72); // the issue has a worktree
const SIDEBAR_WIDTH: u16 = 46;
const MIN_WIDTH_FOR_SIDEBAR: u16 = 110;
const SPINNER: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const KEYS: &str = "⏎ switch  f finish  n new  a tag  / search  space fold  </> board/backlog  c CI logs  C fix CI  b diff  V approve  o PR  t ticket  r refresh  q quit";

pub fn render(frame: &mut Frame, app: &App) {
    let [main, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
    if let Mode::Logs {
        title,
        lines,
        scroll,
    } = &app.mode
    {
        let block = Block::bordered()
            .title(format!(" {title} "))
            .border_style(Style::new().fg(MUTED));
        let height = block.inner(main).height as usize;
        scroll.set(scroll.get().min(lines.len().saturating_sub(height)));
        let visible: Vec<Line> = lines
            .iter()
            .skip(scroll.get())
            .take(height)
            .map(|line| Line::raw(line.as_str()))
            .collect();
        frame.render_widget(Paragraph::new(visible).block(block), main);
    } else {
        let rows = app.rows();
        if main.width >= MIN_WIDTH_FOR_SIDEBAR {
            let [list, sidebar] =
                Layout::horizontal([Constraint::Min(0), Constraint::Length(SIDEBAR_WIDTH)])
                    .areas(main);
            render_list(frame, app, &rows, list);
            render_sidebar(frame, app, rows.get(app.selected), sidebar);
        } else {
            render_list(frame, app, &rows, main);
        }
        if let Mode::PickRepo {
            issue,
            candidates,
            filter,
            selected,
            ..
        } = &app.mode
        {
            render_repo_picker(frame, &issue.key, candidates, filter, *selected, main);
        }
        if let Mode::ConfirmClaim { issue, .. } = &app.mode {
            render_claim_confirmation(frame, issue, main);
        }
    }
    frame.render_widget(Paragraph::new(footer_line(app)), footer);
}

fn render_list(frame: &mut Frame, app: &App, rows: &[Row], area: Rect) {
    let issues = &app.data.issues;
    let status_width = issues
        .iter()
        .map(|issue| width(&issue.status))
        .max()
        .unwrap_or(0)
        .clamp(6, 14);
    let repos: Vec<String> = issues
        .iter()
        .map(|issue| app.repos_of(issue).join(", "))
        .collect();
    let repo_width = repos
        .iter()
        .map(|repo| width(repo))
        .max()
        .unwrap_or(0)
        .clamp(4, 24);
    let fixed = 2 + 1 + status_width + 2 + 8 + 2 + 4 + 2 + 9 + 2 + 2 + 1 + repo_width;
    let issue_width = (area.width as usize).saturating_sub(fixed);

    let header = Line::from(format!(
        "{}{} {}  {}  {}  {}  CI Repo",
        "  ",
        pad("Issue", issue_width),
        pad("Status", status_width),
        pad("Dev", 8),
        pad("Age", 4),
        pad("PR", 9),
    ))
    .style(Style::new().fg(MUTED));

    let height = (area.height as usize).saturating_sub(1);
    let mut scroll = app.scroll.get().min(app.selected);
    if app.selected >= scroll + height {
        scroll = app.selected + 1 - height;
    }
    app.scroll.set(scroll);

    let mut lines = vec![header];
    if !app.loaded {
        lines.push(Line::from("  Loading…").style(Style::new().fg(MUTED)));
    }
    for (index, row) in rows.iter().enumerate().skip(scroll).take(height) {
        let selection = if index == app.selected {
            Style::new().bg(SELECTION)
        } else if row.kind == Kind::Parent {
            Style::new().bg(PARENT_SHADE)
        } else if app.open_diffs.contains(&row.key) {
            Style::new().bg(DIFF_OPEN)
        } else if app.data.workspaces.contains_key(&row.key) {
            Style::new().bg(WORKTREE_OPEN)
        } else {
            Style::new()
        };
        if row.kind == Kind::Section {
            let folded = app.query.is_empty() // a search shows every match
                && app.collapsed.contains(&model::section_fold(&row.section));
            lines.push(section_line(row, folded, area.width as usize).style(selection));
            continue;
        }
        let has_columns = row.kind == Kind::Issue; // a parent only names what its children share
        let issue = row
            .issue
            .filter(|_| has_columns)
            .map(|issue| &issues[issue]);
        let pr = app.data.prs.get(&row.key).filter(|_| has_columns);
        // Between the key and the title, a badge for the most pressing thing about the issue.
        let badge = |label: &'static str, color: Color| {
            Span::styled(
                label,
                Style::new()
                    .fg(Color::Black)
                    .bg(color)
                    .add_modifier(Modifier::BOLD),
            )
        };
        let newly_reviewable = issue.is_some_and(|issue| app.awaits_first_look(issue));
        let badge = if app.open_diffs.contains(&row.key) {
            badge("  DIFF  ", Color::Green)
        } else if app.data.workspaces.contains_key(&row.key) {
            badge("  WORK  ", Color::LightBlue)
        } else if newly_reviewable {
            badge("  NEW  ", Color::Yellow)
        } else {
            Span::raw("")
        };
        let mut spans = vec![Span::raw("  ")];
        spans.extend(issue_cell(row, badge, issue_width));
        spans.push(Span::raw(" "));
        match issue {
            Some(issue) => {
                let mine = issue.assignee.as_ref() == Some(&app.data.me);
                let assignee = issue
                    .assignee
                    .as_ref()
                    .and_then(|person| person.name.split_whitespace().next());
                let assignee_style = if mine {
                    Style::new().add_modifier(Modifier::BOLD)
                } else {
                    Style::new()
                };
                spans.push(Span::styled(
                    pad(&issue.status, status_width),
                    status_style(&issue.status),
                ));
                spans.push(Span::styled(
                    format!("  {}", pad(assignee.unwrap_or(""), 8)),
                    assignee_style,
                ));
                let (age, age_style) = age(issue.updated, app.now);
                spans.push(Span::styled(format!("  {}", pad(&age, 4)), age_style));
            }
            None => spans.push(Span::raw(" ".repeat(status_width + 2 + 8 + 2 + 4))),
        }
        spans.push(Span::raw("  "));
        let mut pr_spans = pr_cell(pr);
        pr_spans.push(Span::raw("  "));
        pr_spans.push(ci_span(pr.and_then(Pr::ci), app.spinner));
        if pr.is_some_and(|pr| !pr.is_actionable()) {
            for span in &mut pr_spans {
                span.style = Style::new().fg(MUTED); // nothing for me to do here
            }
        }
        spans.extend(pr_spans);
        let repo = issue.map_or("", |_| repos[row.issue.unwrap_or_default()].as_str());
        spans.push(Span::styled(
            format!(" {}", pad(repo, repo_width)),
            Style::new().fg(MUTED),
        ));
        lines.push(Line::from(spans).style(selection));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// `▾ In Progress (6) ────`: the heading of a board column or the backlog.
fn section_line(row: &Row, folded: bool, width: usize) -> Line<'static> {
    let arrow = if folded { "▸" } else { "▾" };
    let heading = format!("{arrow} {} ({}) ", row.title, row.count);
    let rule = "─".repeat(width.saturating_sub(self::width(&heading)));
    Line::from(vec![
        Span::styled(
            heading,
            status_style(&row.title).add_modifier(Modifier::BOLD),
        ),
        Span::styled(rule, Style::new().fg(MUTED)),
    ])
}

fn issue_cell(row: &Row, badge: Span<'static>, width: usize) -> Vec<Span<'static>> {
    let (prefix, title, style) = match row.kind {
        Kind::Parent => (
            "",
            format!("{} ({})", row.title, row.count),
            Style::new().add_modifier(Modifier::BOLD),
        ),
        _ if row.indented => ("  ", row.title.clone(), Style::new()),
        _ => ("", row.title.clone(), Style::new()),
    };
    let badge_gap = if badge.content.is_empty() { "" } else { " " };
    let taken = self::width(prefix) + self::width(&badge.content) + self::width(badge_gap);
    let title_width = width.saturating_sub(taken + self::width(&row.key) + 1);
    vec![
        Span::raw(prefix),
        Span::styled(row.key.clone(), Style::new().fg(Color::Blue)),
        Span::raw(badge_gap),
        badge,
        Span::styled(format!(" {}", pad(&title, title_width)), style),
    ]
}

/// `#123 ⚠2`: the number coloured by review state, then the unresolved review threads.
fn pr_cell(pr: Option<&Pr>) -> Vec<Span<'static>> {
    let Some(pr) = pr else {
        return vec![Span::raw(" ".repeat(9))];
    };
    let number = format!("#{}", pr.number);
    let unresolved = if pr.unresolved > 0 {
        format!(" ⚠{}", pr.unresolved)
    } else {
        String::new()
    };
    let padding = 9usize.saturating_sub(width(&number) + width(&unresolved));
    vec![
        Span::styled(number, review_style(pr)),
        Span::styled(unresolved, Style::new().fg(Color::Yellow)),
        Span::raw(" ".repeat(padding)),
    ]
}

fn review_style(pr: &Pr) -> Style {
    if pr.is_draft {
        return Style::new().fg(MUTED);
    }
    match pr.review {
        Review::Approved => Style::new().fg(Color::Green),
        Review::ChangesRequested => Style::new().fg(Color::Red),
        Review::Awaited => Style::new(),
    }
}

fn ci_span(state: Option<CheckState>, spinner: usize) -> Span<'static> {
    match state {
        Some(CheckState::Passed) => Span::styled("✓ ", Style::new().fg(Color::Green)),
        Some(CheckState::Failed) => Span::styled("✗ ", Style::new().fg(Color::Red)),
        Some(CheckState::Running) => Span::styled(
            format!("{} ", SPINNER[spinner % SPINNER.len()]),
            Style::new().fg(Color::Yellow),
        ),
        Some(CheckState::Skipped) | None => Span::raw("  "),
    }
}

fn render_sidebar(frame: &mut Frame, app: &App, row: Option<&Row>, area: Rect) {
    let block = Block::new()
        .padding(ratatui::widgets::Padding::horizontal(2))
        .style(Style::new().bg(Color::Rgb(42, 42, 55)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(row) = row.filter(|row| row.kind != Kind::Section) else {
        return;
    };
    let text_width = inner.width as usize;
    let muted = Style::new().fg(MUTED);
    let field = |label: &str, value: String| {
        Line::from(vec![Span::styled(pad(label, 10), muted), Span::raw(value)])
    };
    let section = |title: &str, tag: String| {
        let rule = "─".repeat(text_width.saturating_sub(width(title) + width(&tag) + 8));
        Line::from(format!("── {title} {rule} {tag} ──")).style(muted)
    };

    let mut lines = vec![Line::raw("")];
    lines.extend(
        wrap(&row.title, text_width)
            .into_iter()
            .map(|line| Line::from(line).style(Style::new().add_modifier(Modifier::BOLD))),
    );
    lines.push(Line::raw(""));
    lines.push(section("Jira", row.key.clone()));
    if let Some(issue) = row.issue.map(|issue| &app.data.issues[issue]) {
        lines.extend(issue_lines(issue, &field));
    }
    if let Some(url) = app.data.workspaces.get(&row.key) {
        lines.push(field("Workspace", url.clone()));
    }
    if let Some(pr) = app.data.prs.get(&row.key) {
        lines.push(Line::raw(""));
        lines.push(section("GitHub", format!("#{}", pr.number)));
        lines.extend(pr_lines(pr, &field, app.spinner));
    }
    if let Some(issue) = row.issue.map(|issue| &app.data.issues[issue])
        && !issue.description.is_empty()
    {
        lines.push(Line::raw(""));
        lines.extend(
            wrap(&issue.description, text_width)
                .into_iter()
                .map(Line::from),
        );
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn issue_lines(
    issue: &Issue,
    field: &impl Fn(&str, String) -> Line<'static>,
) -> Vec<Line<'static>> {
    let assignee = issue
        .assignee
        .as_ref()
        .map_or("Unassigned", |person| person.name.as_str());
    let mut lines = vec![
        field("Status", issue.status.clone()),
        field("Assignee", assignee.to_owned()),
    ];
    if let Some(epic) = &issue.epic {
        lines.push(field("Epic", format!("{} {}", epic.key, epic.summary)));
    }
    if let Some(parent) = issue.parent.as_ref().filter(|_| issue.is_subtask) {
        lines.push(field(
            "Parent",
            format!("{} {}", parent.key, parent.summary),
        ));
    }
    if !issue.labels.is_empty() {
        lines.push(field("Labels", issue.labels.join(", ")));
    }
    lines
}

fn pr_lines(
    pr: &Pr,
    field: &impl Fn(&str, String) -> Line<'static>,
    spinner: usize,
) -> Vec<Line<'static>> {
    let review = match (pr.is_draft, pr.review) {
        (true, _) => "Draft",
        (_, Review::Approved) => "Approved",
        (_, Review::ChangesRequested) => "Changes requested",
        (_, Review::Awaited) => "Awaiting review",
    };
    let mut state = review.to_owned();
    if pr.has_conflicts {
        state.push_str(", conflicts");
    }
    if pr.auto_merge {
        state.push_str(", auto-merge");
    }
    let mut lines = vec![
        field("Status", state),
        field("Repo", pr.repo().to_owned()),
        field("Author", pr.author.clone()),
        field("Changes", format!("+{} / -{}", pr.additions, pr.deletions)),
    ];
    if pr.unresolved > 0 {
        lines.push(field("Comments", format!("{} unresolved", pr.unresolved)));
    }
    for check in pr
        .checks
        .iter()
        .filter(|check| check.state != CheckState::Skipped)
    {
        lines.push(Line::from(vec![
            ci_span(Some(check.state), spinner),
            Span::raw(check.name.clone()),
        ]));
    }
    lines
}

fn render_claim_confirmation(frame: &mut Frame, issue: &Issue, area: Rect) {
    let width = area.width.min(64);
    let assignee = match &issue.assignee {
        Some(person) => format!("It is currently assigned to {}.", person.name),
        None => "It is currently unassigned.".to_owned(),
    };
    let text = format!(
        "{} {}\n\nThis assigns it to you, moves it to In Progress and puts it on the board. {assignee}",
        issue.key, issue.summary
    );
    let mut lines: Vec<Line> = wrap(&text, width.saturating_sub(4) as usize)
        .into_iter()
        .map(Line::from)
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::from("y assign and open   esc cancel").style(Style::new().fg(MUTED)));
    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    let block = Block::bordered()
        .title(" Put this on your name? ")
        .padding(ratatui::widgets::Padding::horizontal(1));
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn render_repo_picker(
    frame: &mut Frame,
    key: &str,
    candidates: &[String],
    filter: &str,
    selected: usize,
    area: Rect,
) {
    let shown: Vec<&String> = candidates
        .iter()
        .filter(|repo| model::matches(repo, filter))
        .collect();
    let height = (shown.len() as u16 + 3)
        .min(area.height.saturating_sub(2))
        .max(4);
    let popup = Rect {
        x: area.x + area.width.saturating_sub(50) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width: area.width.min(50),
        height,
    };
    let block = Block::bordered().title(format!(" Repository for {key} "));
    let visible = block.inner(popup).height.saturating_sub(1) as usize;
    let first = selected.saturating_sub(visible.saturating_sub(1));
    let mut lines = vec![Line::from(format!("/{filter}█"))];
    for (index, repo) in shown.iter().enumerate().skip(first).take(visible) {
        let style = if index == selected {
            Style::new().bg(SELECTION)
        } else {
            Style::new()
        };
        lines.push(Line::from(format!(" {repo}")).style(style));
    }
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
}

fn footer_line(app: &App) -> Line<'static> {
    let muted = Style::new().fg(MUTED);
    match &app.mode {
        Mode::Search => return Line::from(format!("/{}█", app.query)),
        Mode::NewIssue(summary) => return Line::from(format!("New task: {summary}█")),
        Mode::PickRepo { .. } => {
            return Line::from("type to filter  ↑/↓ select  ⏎ choose  esc cancel").style(muted);
        }
        Mode::ConfirmClaim { .. } => {
            return Line::from("y assign and open  esc cancel").style(muted);
        }
        Mode::Logs { .. } => {
            return Line::from("j/k scroll  d/u page  g/G top/bottom  q close").style(muted);
        }
        Mode::List => {}
    }
    if !app.running.is_empty() {
        let spinner = SPINNER[app.spinner % SPINNER.len()];
        return Line::from(format!("{spinner} {}", app.running.join(", ")))
            .style(Style::new().fg(Color::Yellow));
    }
    match &app.message {
        Some((message, true)) => Line::from(message.clone()).style(Style::new().fg(Color::Red)),
        Some((message, false)) => Line::from(message.clone()).style(Style::new().fg(Color::Green)),
        None if !app.query.is_empty() => {
            Line::from(format!("/{}  (esc clears)", app.query)).style(muted)
        }
        None => Line::from(KEYS).style(muted),
    }
}

fn status_style(status: &str) -> Style {
    let status = status.to_lowercase();
    let color = if status.contains("progress") {
        Color::Yellow
    } else if status.contains("review") {
        Color::LightBlue
    } else if status.contains("rejected") || status.contains("blocked") {
        Color::Red
    } else if status.contains("done") || status.contains("staging") {
        Color::Green
    } else {
        MUTED
    };
    Style::new().fg(color)
}

/// `5m`, `3h`, `12d`: how long ago, in the largest unit that fits. The colour fades in four
/// steps: the last hour, the last day, the last week, and anything older.
fn age(time: Option<DateTime<Utc>>, now: DateTime<Utc>) -> (String, Style) {
    let Some(time) = time else {
        return (String::new(), Style::new());
    };
    let minutes = (now - time).num_minutes().max(0);
    match minutes {
        0..60 => (
            format!("{minutes}m"),
            Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        60..1440 => (format!("{}h", minutes / 60), Style::new().fg(Color::White)),
        1440..10080 => (format!("{}d", minutes / 1440), Style::new().fg(Color::Gray)),
        _ => (format!("{}d", minutes / 1440), Style::new().fg(MUTED)),
    }
}

fn width(text: &str) -> usize {
    text.chars().count()
}

/// Exactly `width` characters: padded with spaces, or cut off with an ellipsis.
fn pad(text: &str, width: usize) -> String {
    let length = self::width(text);
    if length <= width {
        return format!("{text}{}", " ".repeat(width - length));
    }
    let kept: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}…")
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if !line.is_empty() && self::width(&line) + 1 + self::width(word) > width {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::mpsc;

    use chrono::TimeZone;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::actions::Context;
    use crate::config::Config;
    use crate::jira::Jira;
    use crate::model::{Check, Data, Parent, Person};

    fn app() -> App {
        let config = Config {
            jira_url: "https://example.atlassian.net".to_owned(),
            jira_email: String::new(),
            jira_token: String::new(),
            repos_dir: "/".into(),
            state_dir: "/nonexistent/work-test-state".into(),
        };
        let jira = Jira::new(&config);
        let (sender, _receiver) = mpsc::channel();
        let mut app = App::new(
            Arc::new(Context { config, jira }),
            sender,
            vec!["backend".to_owned()],
        );
        let now = Utc.with_ymd_and_hms(2026, 1, 10, 12, 0, 0).unwrap();
        let casey = Person {
            account_id: "1".to_owned(),
            name: "Casey Dev".to_owned(),
        };
        let issue =
            |key: &str, summary: &str, status: &str, parent: Option<&str>, hours: i64| Issue {
                key: key.to_owned(),
                summary: summary.to_owned(),
                status: status.to_owned(),
                status_id: String::new(),
                is_subtask: false,
                is_epic: false,
                section: if status == "Plan" { "Backlog" } else { status }.to_owned(),
                assignee: (status != "Plan").then(|| casey.clone()),
                labels: vec!["backend".to_owned()],
                parent: parent.map(|key| Parent {
                    key: key.to_owned(),
                    summary: "Exports".to_owned(),
                }),
                epic: parent.map(|key| Parent {
                    key: key.to_owned(),
                    summary: "Exports".to_owned(),
                }),
                listed: true,
                updated: Some(now - chrono::Duration::hours(hours)),
                description: "Add a button to the toolbar that downloads the current view as CSV."
                    .to_owned(),
            };
        let check = |name: &str, state| Check {
            name: name.to_owned(),
            state,
            run_id: None,
        };
        app.now = now;
        app.loaded = true;
        app.collapsed.clear();
        app.data = Data {
            sections: ["Review", "In Progress", "Backlog"]
                .map(str::to_owned)
                .to_vec(),
            issues: vec![
                issue(
                    "INI-2",
                    "Add the export button",
                    "In Progress",
                    Some("INI-1"),
                    2,
                ),
                issue("INI-3", "Export as CSV", "Review", Some("INI-1"), 30),
                issue("INI-4", "Fix the login redirect", "Plan", None, 300),
            ],
            prs: HashMap::from([(
                "INI-2".to_owned(),
                Pr {
                    number: 42,
                    repo_slug: "acme/backend".to_owned(),
                    author: "casey".to_owned(),
                    is_mine: true,
                    review: Review::ChangesRequested,
                    additions: 72,
                    deletions: 14,
                    unresolved: 1,
                    checks: vec![
                        check("lint", CheckState::Passed),
                        check("build", CheckState::Failed),
                    ],
                    ..Pr::default()
                },
            )]),
            workspaces: HashMap::from([("INI-2".to_owned(), "http://localhost:9001".to_owned())]),
            me: casey,
            github_login: "casey".to_owned(),
        };
        app
    }

    fn rendered(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn list_with_sidebar() {
        let mut app = app();
        app.selected = 2;
        insta::assert_snapshot!(rendered(&app, 130, 22));
    }

    #[test]
    fn narrow_list_with_collapsed_group_and_search() {
        let mut app = app();
        app.collapsed.insert(model::section_fold("Review"));
        insta::assert_snapshot!("collapsed", rendered(&app, 80, 6));
        app.query = "csv".to_owned();
        insta::assert_snapshot!("search", rendered(&app, 80, 6));
    }

    #[test]
    fn claim_confirmation() {
        let mut app = app();
        app.mode = Mode::ConfirmClaim {
            issue: Box::new(app.data.issues[2].clone()),
            repo: "backend".to_owned(),
            add_label: false,
        };
        insta::assert_snapshot!(rendered(&app, 80, 14));
    }

    #[test]
    fn badges_sit_between_key_and_title() {
        let mut app = app();
        app.open_diffs.insert("INI-3".to_owned());
        insta::assert_snapshot!(rendered(&app, 80, 8));
    }

    #[test]
    fn repo_picker() {
        let mut app = app();
        app.mode = Mode::PickRepo {
            intent: crate::app::Intent::Switch,
            issue: Box::new(app.data.issues[2].clone()),
            candidates: vec![
                "backend".to_owned(),
                "frontend".to_owned(),
                "infra".to_owned(),
            ],
            add_label: true,
            filter: "end".to_owned(),
            selected: 1,
        };
        insta::assert_snapshot!(rendered(&app, 80, 12));
    }
}
