mod actions;
mod apis;
mod app;
mod cache;
mod config;
#[cfg(test)]
mod fixtures;
mod git;
mod issue;
mod repos;
mod theme;
mod ui;
mod utils;

use std::{io, time::Duration};

use app::{AppView, InputFocus};
use color_eyre::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use notify::{RecursiveMode, Watcher};
use ratatui::{backend::CrosstermBackend, Terminal};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

type Backend = CrosstermBackend<io::Stdout>;

fn init_logging() -> WorkerGuard {
    let file_appender = tracing_appender::rolling::never(".", "debug.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::registry()
        .with(EnvFilter::new("warn"))
        .with(
            fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false)
                .with_target(false),
        )
        .init();
    guard
}

#[tokio::main]
async fn main() -> Result<()> {
    let _log_guard = init_logging();
    color_eyre::install()?;

    let config = config::AppConfig::from_env()?;

    // File watcher for openspec changes.
    // The watcher must stay alive for the duration of the app.
    let (_watcher, fs_rx) = setup_file_watcher(&config.repos_dir);

    let mut app = AppView::new(config)?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Draw the initial "Loading..." frame before any network calls
    terminal.draw(|frame| ui::render(&mut app, frame))?;

    app.spawn_initialize();

    let app_result = run_app(&mut terminal, app, fs_rx).await;

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

/// Set up a file watcher on openspec changes and send a signal when any
/// `tasks.json` file is created or modified.
fn setup_file_watcher(
    repos_dir: &std::path::Path,
) -> (
    Option<notify::RecommendedWatcher>,
    tokio::sync::mpsc::UnboundedReceiver<()>,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    (try_setup_watcher(tx, repos_dir), rx)
}

fn try_setup_watcher(
    tx: tokio::sync::mpsc::UnboundedSender<()>,
    repos_dir: &std::path::Path,
) -> Option<notify::RecommendedWatcher> {
    let Some(changes_dir) = actions::import_tasks::openspec_changes_dir(repos_dir) else {
        return None;
    };

    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let Ok(event) = result else {
            return;
        };
        let has_tasks_json_path = event
            .paths
            .iter()
            .any(|p| p.file_name().and_then(|n| n.to_str()) == Some("tasks.json"));
        if !has_tasks_json_path {
            return;
        }

        let _ = tx.send(());
    })
    .ok()?;

    watcher.watch(&changes_dir, RecursiveMode::Recursive).ok()?;

    Some(watcher)
}

async fn run_app(
    terminal: &mut Terminal<Backend>,
    mut app: AppView,
    mut fs_rx: tokio::sync::mpsc::UnboundedReceiver<()>,
) -> Result<()> {
    while !app.should_quit {
        terminal.draw(|frame| ui::render(&mut app, frame))?;

        app.tick_spinner();
        app.status_bar.expire_alerts();

        // Drain all pending background messages (non-blocking)
        while let Ok(msg) = app.message_rx.try_recv() {
            app.handle_message(msg);
        }

        // Drain file watcher events (debounced: one scan per loop tick)
        let mut rescan_imports = false;
        while fs_rx.try_recv().is_ok() {
            rescan_imports = true;
        }
        if rescan_imports {
            app.spawn_scan_import_tasks();
        }

        // Auto-refresh: every 10s when CI checks are pending, every 60s otherwise
        if !app.is_busy() {
            if app.has_pending_checks() && app.last_ci_refresh.elapsed() >= CI_AUTO_REFRESH {
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
        match event {
            Event::Key(key_event) => {
                if key_event.kind != KeyEventKind::Press {
                    continue;
                }
                handle_key_event(&mut app, key_event).await;
            }
            Event::Mouse(mouse_event) => match app.input_focus {
                InputFocus::CiLogPopup => match mouse_event.kind {
                    MouseEventKind::ScrollDown => app.ci_log_popup.scroll_by(3),
                    MouseEventKind::ScrollUp => app.ci_log_popup.scroll_by(-3),
                    _ => {}
                },
                InputFocus::List | InputFocus::Search | InputFocus::InlineNew => {
                    match mouse_event.kind {
                        MouseEventKind::ScrollDown => app.list.scroll_viewport(3),
                        MouseEventKind::ScrollUp => app.list.scroll_viewport(-3),
                        MouseEventKind::Down(MouseButton::Left) => {
                            let clicked_row = mouse_event.row as usize;
                            let data_row = clicked_row.saturating_sub(1);
                            let target = app.list.scroll_offset + data_row;
                            if target < app.list.display_rows.len() {
                                app.list.selected_index = target;
                                app.list.adjust_scroll_offset();
                                app.prefetch_selected_pr_detail();
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    Ok(())
}

async fn handle_key_event(app: &mut AppView, key_event: KeyEvent) {
    if key_event.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key_event.code, KeyCode::Char('c') | KeyCode::Char('C'))
    {
        app.should_quit = true;
        return;
    }

    match app.input_focus {
        InputFocus::Search | InputFocus::InlineNew | InputFocus::List => {
            ui::list::update(app, key_event).await
        }
        InputFocus::ImportTasksPopup => ui::import_tasks::update(app, key_event),
        InputFocus::CiLogPopup => ui::ci_logs::update(app, key_event).await,
        InputFocus::LabelPicker => ui::label_picker::update(app, key_event).await,
    }

    app.previous_key = Some(key_event.code);
}
