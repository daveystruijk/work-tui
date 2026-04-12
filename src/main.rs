mod actions;
mod app;
mod cache;
mod events;
mod git;
mod github;
mod jira;
mod notify;
mod repos;
mod ui;

use std::{io, time::Duration};

use app::{App, Screen};
use color_eyre::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

type Backend = CrosstermBackend<io::Stdout>;

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let mut app = App::new()?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Draw the initial "Loading..." frame before any network calls
    terminal.draw(|frame| ui::render(&mut app, frame))?;

    app.spawn_initialize();

    let app_result = run_app(&mut terminal, app).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    app_result
}

const AUTO_REFRESH: Duration = Duration::from_secs(60);
const CI_AUTO_REFRESH: Duration = Duration::from_secs(10);

async fn run_app(terminal: &mut Terminal<Backend>, mut app: App) -> Result<()> {
    while !app.should_quit {
        terminal.draw(|frame| ui::render(&mut app, frame))?;

        app.tick_spinner();
        app.tick_completed_tasks();

        // Drain all pending background messages (non-blocking)
        while let Ok(msg) = app.bg_rx.try_recv() {
            app.handle_bg_msg(msg);
        }

        // Auto-refresh: every 10s when CI checks are pending, every 60s otherwise
        if !app.is_busy() {
            if app.has_pending_checks()
                && app.last_ci_refresh.elapsed() >= CI_AUTO_REFRESH
            {
                app.spawn_github_prs_active();
            } else if app.last_ci_refresh.elapsed() >= AUTO_REFRESH {
                app.spawn_refresh();
            }
        }

        // Spin faster while background work or pending CI checks are active
        let poll_ms = if app.is_busy() || app.has_pending_checks() {
            40
        } else {
            100
        };
        if !event::poll(Duration::from_millis(poll_ms))? {
            continue;
        }

        let event = event::read()?;
        if let Event::Key(key_event) = event {
            if key_event.kind != KeyEventKind::Press {
                continue;
            }
            handle_key_event(&mut app, key_event).await;
        }
    }

    Ok(())
}

async fn handle_key_event(app: &mut App, key_event: KeyEvent) {
    if app.screen == Screen::List && app.inline_new_active() {
        handle_inline_new(app, key_event).await;
        return;
    }
    match app.screen {
        Screen::List => handle_list_normal(app, key_event).await,
        Screen::Detail => handle_detail(app, key_event).await,
        Screen::New => handle_new(app, key_event).await,
    }
}

async fn handle_list_normal(app: &mut App, key_event: KeyEvent) {
    if app.label_picker_active() {
        handle_label_picker(app, key_event).await;
        return;
    }
    if key_event.modifiers.contains(KeyModifiers::CONTROL) {
        match key_event.code {
            KeyCode::Char('c') | KeyCode::Char('C') => {
                app.should_quit = true;
                return;
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                app.pending_g = false;
                move_selection_by(app, app.list_area_height as isize / 2);
                return;
            }
            KeyCode::Char('u') | KeyCode::Char('U') => {
                app.pending_g = false;
                move_selection_by(app, -(app.list_area_height as isize / 2));
                return;
            }
            _ => {}
        }
    }

    match key_event.code {
        KeyCode::Char(c) => {
            if app.pending_g && c == 'g' {
                app.pending_g = false;
                app.selected_index = 0;
                adjust_scroll_offset(app);
                return;
            }
            app.pending_g = false;

            match c {
                'b' => {
                    app.status_message = "Opening diff...".to_string();
                    app.spawn_branch_diff();
                }
                'j' => move_selection_down(app),
                'k' => move_selection_up(app),
                'G' => move_selection_to_end(app),
                'g' => {
                    app.pending_g = true;
                }
                'p' => {
                    app.status_message = "Picking up...".to_string();
                    app.spawn_pick_up();
                }
                'o' => match app.open_selected_pr_in_browser().await {
                    Ok(_) => {}
                    Err(err) => app.status_message = format!("{err}"),
                },
                't' => match app.open_selected_issue_in_browser().await {
                    Ok(_) => {}
                    Err(err) => app.status_message = format!("Failed to open issue: {err}"),
                },
                'n' => {
                    if !app.start_inline_new() {
                        if let Err(err) = app.enter_new().await {
                            app.status_message = format!("Failed to open new issue form: {err}");
                        }
                    }
                }
                'a' => app.open_label_picker(),
                'r' => {
                    app.loading = true;
                    app.spawn_refresh();
                }
                'f' => {
                    app.status_message = "Finishing...".to_string();
                    app.spawn_finish();
                }
                'V' => {
                    app.status_message = "Approving & enabling auto-merge...".to_string();
                    app.spawn_approve_merge();
                }
                _ => {}
            }
        }
        KeyCode::Enter => {
            app.pending_g = false;
            if !app.toggle_story_collapse() {
                if app.selected_issue().is_some() {
                    app.enter_detail();
                    app.spawn_load_issue_events();
                }
            }
        }
        KeyCode::Down => {
            app.pending_g = false;
            move_selection_down(app);
        }
        KeyCode::Up => {
            app.pending_g = false;
            move_selection_up(app);
        }
        _ => {
            app.pending_g = false;
        }
    }
}

async fn handle_inline_new(app: &mut App, key_event: KeyEvent) {
    if key_event.modifiers.contains(KeyModifiers::CONTROL) {
        match key_event.code {
            KeyCode::Char('c') | KeyCode::Char('C') => {
                app.cancel_inline_new();
                return;
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                app.status_message = "Creating issue...".to_string();
                app.spawn_submit_inline_new();
                return;
            }
            _ => {}
        }
    }

    match key_event.code {
        KeyCode::Esc => app.cancel_inline_new(),
        KeyCode::Enter => {
            app.status_message = "Creating issue...".to_string();
            app.spawn_submit_inline_new();
        }
        KeyCode::Backspace => {
            if let Some(state) = app.inline_new.as_mut() {
                state.summary.pop();
            }
        }
        KeyCode::Char(c) => {
            if !key_event
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            {
                if let Some(state) = app.inline_new.as_mut() {
                    state.summary.push(c);
                }
            }
        }
        _ => {}
    }
}

async fn handle_detail(app: &mut App, key_event: KeyEvent) {
    if app.label_picker_active() {
        handle_label_picker(app, key_event).await;
        return;
    }
    if key_event.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key_event.code, KeyCode::Char('c') | KeyCode::Char('C'))
    {
        app.should_quit = true;
        return;
    }

    match key_event.code {
        KeyCode::Esc => app.back_to_list(),
        KeyCode::Char(c) => match c {
            'V' => {
                app.status_message = "Approving & enabling auto-merge...".to_string();
                app.spawn_approve_merge();
            }
            _ => match c.to_ascii_lowercase() {
                'p' => {
                    app.status_message = "Picking up...".to_string();
                    app.spawn_pick_up();
                }
                'b' => {
                    app.status_message = "Opening diff...".to_string();
                    app.spawn_branch_diff();
                }
                'o' => match app.open_selected_pr_in_browser().await {
                    Ok(_) => {}
                    Err(err) => app.status_message = format!("{err}"),
                },
                't' => match app.open_selected_issue_in_browser().await {
                    Ok(_) => {}
                    Err(err) => app.status_message = format!("Failed to open issue: {err}"),
                },
                'a' => app.open_label_picker(),
                'f' => {
                    app.status_message = "Finishing...".to_string();
                    app.spawn_finish();
                }
                'j' => app.detail_scroll = app.detail_scroll.saturating_add(1),
                'k' => app.detail_scroll = app.detail_scroll.saturating_sub(1),
                'r' => {
                    if let Some(issue) = app.selected_issue() {
                        let key = issue.key.clone();
                        app.issue_events.remove(&key);
                    }
                    app.spawn_load_issue_events();
                }
                _ => {}
            },
        },
        KeyCode::Down => app.detail_scroll = app.detail_scroll.saturating_add(1),
        KeyCode::Up => app.detail_scroll = app.detail_scroll.saturating_sub(1),
        _ => {}
    }
}

async fn handle_label_picker(app: &mut App, key_event: KeyEvent) {
    if key_event.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key_event.code, KeyCode::Char('c') | KeyCode::Char('C'))
    {
        app.should_quit = true;
        return;
    }

    match key_event.code {
        KeyCode::Esc => app.close_label_picker(),
        KeyCode::Enter => {
            if app.add_label_from_picker() {
                app.close_label_picker();
            }
        }
        KeyCode::Backspace => app.label_picker_backspace(),
        KeyCode::Down => app.move_label_picker_selection(true),
        KeyCode::Up => app.move_label_picker_selection(false),
        KeyCode::Char(c) => app.label_picker_type_char(c),
        _ => {}
    }
}

async fn handle_new(app: &mut App, key_event: KeyEvent) {
    if key_event.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key_event.code, KeyCode::Char('c') | KeyCode::Char('C'))
    {
        app.should_quit = true;
        return;
    }

    match key_event.code {
        KeyCode::Esc => {
            app.new_form = None;
            app.back_to_list();
            return;
        }
        KeyCode::Char('s') | KeyCode::Char('S')
            if key_event.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            match app.submit_new().await {
                Ok(key) => {
                    app.status_message = format!("Created {key}");
                    app.back_to_list();
                }
                Err(err) => app.status_message = format!("Failed to create issue: {err}"),
            }
            return;
        }
        _ => {}
    }

    let Some(form) = app.new_form.as_mut() else {
        return;
    };

    match key_event.code {
        KeyCode::Tab => form.active_field = (form.active_field + 1) % 3,
        KeyCode::BackTab => form.active_field = (form.active_field + 2) % 3,
        _ => handle_new_form_input(form, &key_event),
    }
}

fn handle_new_form_input(form: &mut app::NewForm, key_event: &KeyEvent) {
    match form.active_field {
        0 => match key_event.code {
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('H') => {
                cycle_issue_type(form, false);
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('L') => {
                cycle_issue_type(form, true);
            }
            _ => {}
        },
        1 => match key_event.code {
            KeyCode::Backspace => {
                form.summary.pop();
            }
            KeyCode::Char(c) => {
                if !key_event
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                {
                    form.summary.push(c);
                }
            }
            _ => {}
        },
        2 => match key_event.code {
            KeyCode::Backspace => {
                form.description.pop();
            }
            KeyCode::Enter => form.description.push('\n'),
            KeyCode::Char(c) => {
                if !key_event
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                {
                    form.description.push(c);
                }
            }
            _ => {}
        },
        _ => {}
    }
}

fn cycle_issue_type(form: &mut app::NewForm, forward: bool) {
    let len = form.issue_types.len();
    if len == 0 {
        return;
    }
    form.issue_type_idx = if forward {
        (form.issue_type_idx + 1) % len
    } else {
        (form.issue_type_idx + len - 1) % len
    };
}

const SCROLL_OFF: usize = 3;

fn adjust_scroll_offset(app: &mut App) {
    let height = app.list_area_height as usize;
    if height == 0 {
        return;
    }

    let margin = SCROLL_OFF.min(height / 2);
    let selected = app.selected_index;
    let offset = app.list_scroll_offset;

    // Cursor moved above the top margin — scroll up
    if selected < offset + margin {
        app.list_scroll_offset = selected.saturating_sub(margin);
    }

    // Cursor moved below the bottom margin — scroll down
    if selected + margin >= offset + height {
        app.list_scroll_offset = (selected + margin + 1).saturating_sub(height);
    }

    // Clamp offset so we don't scroll past the end
    let max_offset = app.display_rows.len().saturating_sub(height);
    app.list_scroll_offset = app.list_scroll_offset.min(max_offset);
}

fn move_selection_down(app: &mut App) {
    if app.display_rows.is_empty() {
        app.selected_index = 0;
        return;
    }
    let last = app.display_rows.len() - 1;
    if app.selected_index < last {
        app.selected_index += 1;
    }
    adjust_scroll_offset(app);
}

fn move_selection_up(app: &mut App) {
    if app.selected_index == 0 {
        return;
    }
    app.selected_index -= 1;
    adjust_scroll_offset(app);
}

fn move_selection_to_end(app: &mut App) {
    if app.display_rows.is_empty() {
        return;
    }
    app.selected_index = app.display_rows.len() - 1;
    adjust_scroll_offset(app);
}

fn move_selection_by(app: &mut App, delta: isize) {
    if app.display_rows.is_empty() {
        return;
    }
    let last = app.display_rows.len() - 1;
    let new_index = (app.selected_index as isize + delta).clamp(0, last as isize) as usize;
    app.selected_index = new_index;
    adjust_scroll_offset(app);
}
