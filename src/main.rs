use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::DefaultTerminal;

mod actions;
mod app;
mod config;
mod github;
mod jira;
mod model;
mod shell;
mod ui;

use app::{App, Msg};

fn main() -> Result<()> {
    let config = config::Config::from_env()?;
    let repos = repositories(&config.repos_dir)?;
    let jira = jira::Jira::new(&config);
    let (sender, receiver) = mpsc::channel();
    let mut app = App::new(Arc::new(actions::Context { config, jira }), sender, repos);
    app.refresh();

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app, &receiver);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, app: &mut App, receiver: &Receiver<Msg>) -> Result<()> {
    loop {
        app.tick();
        while let Ok(message) = receiver.try_recv() {
            app.handle(message);
        }
        terminal.draw(|frame| ui::render(frame, app))?;
        if event::poll(Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && app.key(key)
        {
            return Ok(());
        }
    }
}

/// The names of the git repositories directly inside `dir`, leaving out worktrees of them.
fn repositories(dir: &std::path::Path) -> Result<Vec<String>> {
    let mut repos = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with('.') && entry.path().join(".git").is_dir() {
            // a worktree has a `.git` file
            repos.push(name);
        }
    }
    repos.sort();
    Ok(repos)
}
