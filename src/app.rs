use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::actions::{self, Context, FIX_CI, IMPLEMENT, Issues, Target};
use crate::config::PROJECT;
use crate::model::{
    self, BACKLOG, CheckState, Data, Issue, Kind, Pr, Row, normalize, section_fold,
};
use crate::{github, shell};

const REFRESH: Duration = Duration::from_secs(60);
const CI_REFRESH: Duration = Duration::from_secs(10);
const DIFFS_REFRESH: Duration = Duration::from_secs(1); // one quick tmux call; a closed diff should clear promptly

/// What a background thread reports back; the label identifies it in the status line.
pub enum Msg {
    Issues(Result<Issues>),
    Prs(Result<(String, HashMap<String, Pr>)>),
    /// Fresh state of the pull requests whose CI was running.
    CiUpdate(Result<HashMap<String, Pr>>),
    Done(String, Result<String>),
    Logs(String, Result<String>),
}

/// Something to do with an issue that needs its repository.
#[derive(Clone, Copy)]
pub enum Intent {
    /// Open the issue; asks first when that would put it on my name.
    Switch,
    /// Open the issue and make it mine, after confirmation.
    Claim,
    FixCi,
    Finish,
    Diff,
    /// Only record the repository on the issue.
    Tag,
}

pub enum Mode {
    List,
    Search,
    NewIssue(String),
    PickRepo {
        intent: Intent,
        issue: Box<Issue>,
        candidates: Vec<String>,
        add_label: bool,
        filter: String,
        selected: usize,
    },
    /// Switching to this issue would assign it to me; wait for a yes.
    ConfirmClaim {
        issue: Box<Issue>,
        repo: String,
        add_label: bool,
    },
    Logs {
        title: String,
        lines: Vec<String>,
        scroll: Cell<usize>, // clamped while rendering, when the height is known
    },
}

pub struct App {
    context: Arc<Context>,
    sender: Sender<Msg>,
    repos: Vec<String>,
    pub data: Data,
    pub loaded: bool,
    pub collapsed: HashSet<String>,
    pub query: String,
    pub selected: usize,
    pub scroll: Cell<usize>, // adjusted while rendering, when the height is known
    pub mode: Mode,
    /// Labels of the background work in flight.
    pub running: Vec<String>,
    /// The outcome of the last action, and whether it was an error.
    pub message: Option<(String, bool)>,
    pub now: DateTime<Utc>,
    pub spinner: usize,
    /// Number of refresh requests still in flight.
    refreshing: u8,
    refreshed_at: Instant,
    /// When the pull requests with running CI were last fetched, or a fetch was started.
    ci_checked_at: Instant,
    /// Keys of the issues whose diff is open in a pane, as of `diffs_checked_at`.
    pub open_diffs: HashSet<String>,
    diffs_checked_at: Instant,
    /// Issue keys whose pull request was already waiting for review at the last refresh.
    reviewable: Option<HashSet<String>>,
    /// Ids of the pull requests I have looked at, kept on disk; `None` until the first load of
    /// a first run has decided what counts as seen.
    seen: Option<HashSet<String>>,
}

impl App {
    pub fn new(context: Arc<Context>, sender: Sender<Msg>, repos: Vec<String>) -> Self {
        let seen = std::fs::read_to_string(context.config.state_dir.join("seen-pull-requests"))
            .ok()
            .map(|ids| ids.lines().map(str::to_owned).collect());
        Self {
            seen,
            context,
            sender,
            repos,
            data: Data::default(),
            loaded: false,
            collapsed: HashSet::from([section_fold(BACKLOG)]),
            query: String::new(),
            selected: 0,
            scroll: Cell::new(0),
            mode: Mode::List,
            running: Vec::new(),
            message: None,
            now: Utc::now(),
            spinner: 0,
            refreshing: 0,
            refreshed_at: Instant::now(),
            ci_checked_at: Instant::now(),
            open_diffs: HashSet::new(),
            diffs_checked_at: Instant::now(),
            reviewable: None,
        }
    }

    pub fn rows(&self) -> Vec<Row> {
        model::rows(
            &self.data.issues,
            &self.data.sections,
            &self.collapsed,
            &self.query,
        )
    }

    /// The repositories an issue is tagged with: those whose name matches one of its labels.
    pub fn repos_of(&self, issue: &Issue) -> Vec<String> {
        self.repos
            .iter()
            .filter(|repo| {
                issue
                    .labels
                    .iter()
                    .any(|label| normalize(label) == normalize(repo))
            })
            .cloned()
            .collect()
    }

    pub fn jira_url(&self, key: &str) -> String {
        format!("{}/browse/{key}", self.context.config.jira_url)
    }

    pub fn tick(&mut self) {
        self.now = Utc::now();
        self.spinner = self.spinner.wrapping_add(1);
        if self.refreshed_at.elapsed() >= REFRESH {
            self.refresh();
        }
        if self.ci_checked_at.elapsed() >= CI_REFRESH {
            self.refresh_running_ci();
        }
        if self.diffs_checked_at.elapsed() >= DIFFS_REFRESH {
            self.diffs_checked_at = Instant::now();
            self.open_diffs = actions::open_diffs();
        }
    }

    /// Follow CI by re-fetching only the pull requests it is running on. The full search
    /// takes several seconds and is a heavy query, too much to repeat every few seconds.
    fn refresh_running_ci(&mut self) {
        self.ci_checked_at = Instant::now();
        let running: Vec<String> = self
            .data
            .prs
            .values()
            .filter(|pr| pr.ci() == Some(CheckState::Running))
            .map(|pr| pr.id.clone())
            .collect();
        if running.is_empty() || self.refreshing > 0 {
            return;
        }
        let sender = self.sender.clone();
        thread::spawn(move || {
            let _ = sender.send(Msg::CiUpdate(github::pull_requests_by_id(&running)));
        });
    }

    /// Reload issues and pull requests side by side; the issues arrive well before GitHub answers.
    pub fn refresh(&mut self) {
        if self.refreshing > 0 {
            return;
        }
        self.refreshing = 2;
        let (context, sender) = (self.context.clone(), self.sender.clone());
        thread::spawn(move || {
            let _ = sender.send(Msg::Issues(actions::load_issues(&context)));
        });
        let (context, sender, repos) = (
            self.context.clone(),
            self.sender.clone(),
            self.repos.clone(),
        );
        thread::spawn(move || {
            let _ = sender.send(Msg::Prs(actions::load_prs(&context, &repos)));
        });
    }

    fn background(&mut self, label: String, job: impl FnOnce() -> Result<String> + Send + 'static) {
        self.running.push(label.clone());
        let sender = self.sender.clone();
        thread::spawn(move || {
            let _ = sender.send(Msg::Done(label, job()));
        });
    }

    pub fn handle(&mut self, message: Msg) {
        match message {
            Msg::Issues(Ok(loaded)) => {
                let selected = self.rows().get(self.selected).cloned();
                let same = |row: &Row, other: &Row| {
                    row.kind == other.kind && row.key == other.key && row.section == other.section
                };
                self.data.issues = loaded.issues;
                self.data.sections = loaded.sections;
                self.data.workspaces = loaded.workspaces;
                self.data.me = loaded.me;
                self.loaded = true;
                self.finish_refresh();
                let rows = self.rows();
                self.selected = selected
                    .and_then(|selected| rows.iter().position(|row| same(row, &selected)))
                    .unwrap_or(self.selected.min(rows.len().saturating_sub(1)));
            }
            Msg::Prs(Ok((login, prs))) => {
                self.notify_reviewable(&prs);
                self.update_seen(&prs);
                self.data.github_login = login;
                self.data.prs = prs;
                self.finish_refresh();
            }
            Msg::CiUpdate(Ok(prs)) => self.data.prs.extend(prs),
            Msg::CiUpdate(Err(error)) => {
                self.message = Some((format!("CI refresh failed: {error:#}"), true));
            }
            Msg::Issues(Err(error)) | Msg::Prs(Err(error)) => {
                self.message = Some((format!("Refresh failed: {error:#}"), true));
                self.finish_refresh();
            }
            Msg::Done(label, result) => {
                self.running.retain(|running| *running != label);
                self.message = Some(match result {
                    Ok(outcome) => (outcome, false),
                    Err(error) => (format!("{label} failed: {error:#}"), true),
                });
                self.refresh();
            }
            Msg::Logs(label, result) => {
                self.running.retain(|running| *running != label);
                match result {
                    Ok(logs) => {
                        self.mode = Mode::Logs {
                            title: label,
                            lines: logs.lines().map(str::to_owned).collect(),
                            scroll: Cell::new(usize::MAX), // the failure is at the end
                        };
                    }
                    Err(error) => self.message = Some((format!("{label} failed: {error:#}"), true)),
                }
            }
        }
    }

    fn finish_refresh(&mut self) {
        self.refreshing = self.refreshing.saturating_sub(1);
        self.refreshed_at = Instant::now();
    }

    /// Whether the issue has a pull request for me to review that I have not looked at yet.
    pub fn awaits_first_look(&self, issue: &Issue) -> bool {
        let (Some(seen), Some(pr)) = (&self.seen, self.data.prs.get(&issue.key)) else {
            return false;
        };
        !pr.is_mine && pr.is_actionable() && !seen.contains(&pr.id)
    }

    /// Bring the record of looked-at pull requests in line with the ones that are open now.
    fn update_seen(&mut self, prs: &HashMap<String, Pr>) {
        let seen = match self.seen.take() {
            // Forget pull requests that are closed, so the record does not grow forever.
            Some(seen) => prs
                .values()
                .filter(|pr| seen.contains(&pr.id))
                .map(|pr| pr.id.clone())
                .collect(),
            // The very first run: everything reviewable today counts as seen, or every row
            // would light up at once.
            None => prs
                .values()
                .filter(|pr| !pr.is_mine && pr.is_actionable())
                .map(|pr| pr.id.clone())
                .collect(),
        };
        self.seen = Some(seen);
        self.save_seen();
    }

    /// Record that I looked at the issue's pull request, by opening it or its diff.
    fn mark_seen_pr(&mut self, key: &str) {
        if let (Some(seen), Some(pr)) = (&mut self.seen, self.data.prs.get(key))
            && seen.insert(pr.id.clone())
        {
            self.save_seen();
        }
    }

    fn seen_file(&self) -> std::path::PathBuf {
        self.context.config.state_dir.join("seen-pull-requests")
    }

    fn save_seen(&mut self) {
        let Some(seen) = &self.seen else {
            return;
        };
        let mut ids: Vec<&str> = seen.iter().map(String::as_str).collect();
        ids.sort_unstable();
        let file = self.seen_file();
        let saved = std::fs::create_dir_all(&self.context.config.state_dir)
            .and_then(|()| std::fs::write(&file, ids.join("\n")));
        if let Err(error) = saved {
            self.message = Some((format!("Could not write {}: {error}", file.display()), true));
        }
    }

    /// Draw attention to pull requests that became reviewable by me since the last refresh:
    /// someone else's, not a draft, and not reviewed by me yet (or with a new review requested
    /// from me). The bell marks this tab in tmux and the status line names them. Nothing
    /// happens for what is already there at startup.
    fn notify_reviewable(&mut self, prs: &HashMap<String, Pr>) {
        let reviewable: HashSet<String> = prs
            .iter()
            .filter(|(_, pr)| !pr.is_mine && pr.is_actionable())
            .map(|(key, _)| key.clone())
            .collect();
        if let Some(known) = &self.reviewable {
            let mut new: Vec<String> = reviewable
                .difference(known)
                .map(|key| format!("{key} (#{} by {})", prs[key].number, prs[key].author))
                .collect();
            new.sort();
            if !new.is_empty() {
                shell::ring_bell();
                self.message = Some((format!("Ready for your review: {}", new.join(", ")), false));
            }
        }
        self.reviewable = Some(reviewable);
    }

    fn selected_issue(&self) -> Option<Issue> {
        let row = self.rows().get(self.selected)?.clone();
        let issue = &self.data.issues[row.issue?];
        (!issue.is_epic).then(|| issue.clone()) // an epic heading is not something to work on
    }

    fn selected_pr(&self) -> Option<Pr> {
        self.data.prs.get(&self.selected_issue()?.key).cloned()
    }

    /// Handle a key press; returns whether to quit.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        if control && key.code == KeyCode::Char('c') {
            return true;
        }
        match std::mem::replace(&mut self.mode, Mode::List) {
            Mode::List => return self.list_key(key.code, control),
            Mode::Search => match key.code {
                KeyCode::Esc => self.query.clear(),
                KeyCode::Enter => {}
                code => {
                    edit(&mut self.query, code);
                    self.selected = 0;
                    self.mode = Mode::Search;
                }
            },
            Mode::ConfirmClaim {
                issue,
                repo,
                add_label,
            } => {
                if key.code == KeyCode::Char('y') {
                    self.launch(Intent::Claim, *issue, repo, add_label);
                }
            }
            Mode::NewIssue(mut summary) => match key.code {
                KeyCode::Esc => {}
                KeyCode::Enter => self.create_task(summary),
                code => {
                    edit(&mut summary, code);
                    self.mode = Mode::NewIssue(summary);
                }
            },
            Mode::PickRepo {
                intent,
                issue,
                candidates,
                add_label,
                mut filter,
                mut selected,
            } => {
                let shown: Vec<&String> = candidates
                    .iter()
                    .filter(|repo| model::matches(repo, &filter))
                    .collect();
                match key.code {
                    KeyCode::Esc => return false,
                    KeyCode::Enter => {
                        if let Some(repo) = shown.get(selected) {
                            self.launch(intent, *issue, (*repo).clone(), add_label);
                        }
                        return false;
                    }
                    KeyCode::Down => selected = (selected + 1).min(shown.len().saturating_sub(1)),
                    KeyCode::Up => selected = selected.saturating_sub(1),
                    code => {
                        edit(&mut filter, code);
                        selected = 0;
                    }
                }
                self.mode = Mode::PickRepo {
                    intent,
                    issue,
                    candidates,
                    add_label,
                    filter,
                    selected,
                };
            }
            Mode::Logs {
                title,
                lines,
                scroll,
            } => {
                let position = scroll.get();
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => return false,
                    KeyCode::Char('j') | KeyCode::Down => scroll.set(position.saturating_add(1)),
                    KeyCode::Char('k') | KeyCode::Up => scroll.set(position.saturating_sub(1)),
                    KeyCode::Char('d') => scroll.set(position.saturating_add(20)),
                    KeyCode::Char('u') => scroll.set(position.saturating_sub(20)),
                    KeyCode::Char('g') => scroll.set(0),
                    KeyCode::Char('G') => scroll.set(usize::MAX),
                    _ => {}
                }
                self.mode = Mode::Logs {
                    title,
                    lines,
                    scroll,
                };
            }
        }
        false
    }

    fn list_key(&mut self, code: KeyCode, control: bool) -> bool {
        self.message = None;
        let rows = self.rows();
        let last = rows.len().saturating_sub(1);
        let current = rows.get(self.selected).cloned();
        match code {
            KeyCode::Char('q') => return true,
            KeyCode::Char('j') | KeyCode::Down => self.selected = (self.selected + 1).min(last),
            KeyCode::Char('k') | KeyCode::Up => self.selected = self.selected.saturating_sub(1),
            KeyCode::Char('d') if control => self.selected = (self.selected + 10).min(last),
            KeyCode::Char('u') if control => self.selected = self.selected.saturating_sub(10),
            KeyCode::Char('g') => self.selected = 0,
            KeyCode::Char('G') => self.selected = last,
            KeyCode::Char('h' | 'l' | ' ') => {
                if let Some(row) = current {
                    let fold = section_fold(&row.section);
                    let collapse = match code {
                        KeyCode::Char('h') => true,
                        KeyCode::Char('l') => false,
                        _ => !self.collapsed.contains(&fold),
                    };
                    if collapse {
                        self.collapsed.insert(fold);
                    } else {
                        self.collapsed.remove(&fold);
                    }
                    self.selected = self
                        .rows()
                        .iter()
                        .position(|other| {
                            other.kind == Kind::Section && other.section == row.section
                        })
                        .unwrap_or(0);
                }
            }
            KeyCode::Esc => self.query.clear(),
            KeyCode::Char('/') => self.mode = Mode::Search,
            KeyCode::Char('r') => self.refresh(),
            KeyCode::Char('n') => self.mode = Mode::NewIssue(String::new()),
            KeyCode::Enter => self.start(Intent::Switch),
            KeyCode::Char('C') => self.start(Intent::FixCi),
            KeyCode::Char('f') => self.start(Intent::Finish),
            KeyCode::Char('b') => self.start(Intent::Diff),
            KeyCode::Char('c') => self.show_ci_logs(),
            KeyCode::Char('V') => self.approve(),
            KeyCode::Char('a') => {
                if let Some(issue) = self.selected_issue() {
                    self.pick_repo(Intent::Tag, issue, self.repos.clone(), false);
                }
            }
            KeyCode::Char('<' | '>') => self.move_between_board_and_backlog(code),
            KeyCode::Char('o' | 't') => self.open(code),
            _ => {}
        }
        false
    }

    /// Work out which repository the selected issue lives in, asking when its labels do not say.
    fn start(&mut self, intent: Intent) {
        let Some(issue) = self.selected_issue() else {
            return;
        };
        let labelled = self.repos_of(&issue);
        if let Some(pr) = self.data.prs.get(&issue.key) {
            let repo = pr.repo().to_owned();
            let add_label = !labelled.contains(&repo);
            return self.launch(intent, issue, repo, add_label);
        }
        match labelled.as_slice() {
            [repo] => self.launch(intent, issue, repo.clone(), false),
            [] => self.pick_repo(intent, issue, self.repos.clone(), true),
            _ => self.pick_repo(intent, issue, labelled, false),
        }
    }

    fn pick_repo(
        &mut self,
        intent: Intent,
        issue: Issue,
        candidates: Vec<String>,
        add_label: bool,
    ) {
        self.mode = Mode::PickRepo {
            intent,
            issue: Box::new(issue),
            candidates,
            add_label,
            filter: String::new(),
            selected: 0,
        };
    }

    fn launch(&mut self, intent: Intent, issue: Issue, repo: String, add_label: bool) {
        let key = issue.key.clone();
        if let Intent::Tag = intent {
            // The chosen repository replaces the ones the issue was tagged with; labels that
            // do not name a repository are not ours to touch.
            let is_repo = |label: &String| {
                self.repos
                    .iter()
                    .any(|repo| normalize(repo) == normalize(label))
            };
            let is_chosen = |label: &String| normalize(label) == normalize(&repo);
            let already_tagged = issue.labels.iter().any(is_chosen);
            let replaced: Vec<String> = issue
                .labels
                .iter()
                .filter(|label| is_repo(label) && !is_chosen(label))
                .cloned()
                .collect();
            if already_tagged && replaced.is_empty() {
                self.message = Some((format!("{key} is already tagged with {repo}"), false));
                return;
            }
            let added = (!already_tagged).then(|| repo.clone());
            let context = self.context.clone();
            return self.background(format!("Tagging {key} with {repo}"), move || {
                context
                    .jira
                    .change_labels(&key, added.as_deref(), &replaced)?;
                Ok(match replaced.as_slice() {
                    [] => format!("Tagged {key} with {repo}"),
                    replaced => format!(
                        "Tagged {key} with {repo}, replacing {}",
                        replaced.join(", ")
                    ),
                })
            });
        }
        let parent = issue
            .parent
            .as_ref()
            .and_then(|parent| {
                self.data
                    .issues
                    .iter()
                    .find(|issue| issue.key == parent.key)
            })
            .cloned();
        let pr = self.data.prs.get(&key).cloned();
        let target = Target {
            issue,
            parent,
            pr,
            repo,
            add_label,
        };
        let (context, me) = (self.context.clone(), self.data.me.clone());
        let is_mine = target.issue.in_progress() && target.issue.assignee.as_ref() == Some(&me);
        match intent {
            Intent::Switch if !is_mine => {
                self.mode = Mode::ConfirmClaim {
                    issue: Box::new(target.issue),
                    repo: target.repo,
                    add_label: target.add_label,
                };
            }
            Intent::Switch => self.background(format!("Switching to {key}"), move || {
                actions::switch(&context, &target, None, IMPLEMENT)
            }),
            Intent::Claim => self.background(format!("Switching to {key}"), move || {
                actions::switch(&context, &target, Some(&me), IMPLEMENT)
            }),
            Intent::FixCi => self.background(format!("Opening {key} to fix CI"), move || {
                actions::switch(&context, &target, None, FIX_CI)
            }),
            Intent::Finish => self.background(format!("Finishing {key}"), move || {
                actions::finish(&context, &target)
            }),
            Intent::Tag => unreachable!("handled above"),
            Intent::Diff => {
                self.mark_seen_pr(&key);
                self.background(format!("Diffing {key}"), move || {
                    actions::diff(&context, &target)
                })
            }
        }
    }

    /// Create a task in the same epic as the selected issue.
    fn create_task(&mut self, summary: String) {
        let summary = summary.trim().to_owned();
        if summary.is_empty() {
            return;
        }
        let parent = self
            .selected_issue()
            .and_then(|issue| issue.epic)
            .map(|epic| epic.key);
        let context = self.context.clone();
        self.background(format!("Creating \"{summary}\""), move || {
            let key = context
                .jira
                .create_task(PROJECT, &summary, parent.as_deref())?;
            Ok(format!("Created {key}"))
        });
    }

    fn show_ci_logs(&mut self) {
        let (Some(issue), Some(pr)) = (self.selected_issue(), self.selected_pr()) else {
            return;
        };
        let label = format!("CI logs of {} #{}", issue.key, pr.number);
        self.running.push(label.clone());
        let sender = self.sender.clone();
        thread::spawn(move || {
            let _ = sender.send(Msg::Logs(label, github::failed_logs(&pr)));
        });
    }

    /// `<` puts the selected issue on the board, `>` sends it back to the backlog.
    fn move_between_board_and_backlog(&mut self, code: KeyCode) {
        let Some(issue) = self.selected_issue() else {
            return;
        };
        let to_backlog = code == KeyCode::Char('>');
        if to_backlog == (issue.section == BACKLOG) {
            return; // already there
        }
        let context = self.context.clone();
        let destination = if to_backlog {
            "the backlog"
        } else {
            "the board"
        };
        self.background(
            format!("Moving {} to {destination}", issue.key),
            move || {
                if to_backlog {
                    context.jira.move_to_backlog(&issue.key)?;
                } else {
                    context.jira.move_to_board(&issue.key)?;
                }
                Ok(format!("Moved {} to {destination}", issue.key))
            },
        );
    }

    fn approve(&mut self) {
        let Some(pr) = self.selected_pr() else {
            return;
        };
        self.background(format!("Approving #{}", pr.number), move || {
            github::approve_and_merge(&pr.url)?;
            Ok(format!("Approved #{} and enabled auto-merge", pr.number))
        });
    }

    /// Open the pull request (`o`) or the ticket (`t`) in the browser.
    fn open(&mut self, code: KeyCode) {
        let Some(key) = self.selected_issue().map(|issue| issue.key) else {
            return;
        };
        if code == KeyCode::Char('o') {
            self.mark_seen_pr(&key);
        }
        let url = match code {
            KeyCode::Char('o') => self.data.prs.get(&key).map(|pr| pr.url.clone()),
            _ => Some(self.jira_url(&key)),
        };
        if let Some(url) = url
            && let Err(error) = shell::open_in_browser(&url)
        {
            self.message = Some((format!("{error:#}"), true));
        }
    }
}

fn edit(text: &mut String, code: KeyCode) {
    match code {
        KeyCode::Char(c) => text.push(c),
        KeyCode::Backspace => {
            text.pop();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;
    use crate::config::Config;
    use crate::jira::Jira;

    fn app(state_dir: &std::path::Path) -> App {
        let config = Config {
            jira_url: String::new(),
            jira_email: String::new(),
            jira_token: String::new(),
            repos_dir: "/".into(),
            state_dir: state_dir.to_owned(),
        };
        let jira = Jira::new(&config);
        let (sender, _receiver) = mpsc::channel();
        App::new(Arc::new(Context { config, jira }), sender, Vec::new())
    }

    fn reviewable(id: &str) -> Pr {
        Pr {
            id: id.to_owned(),
            awaits_my_review: true,
            ..Pr::default()
        }
    }

    fn issue(key: &str) -> Issue {
        Issue {
            key: key.to_owned(),
            ..Issue::default()
        }
    }

    #[test]
    fn a_pull_request_stays_new_until_looked_at_even_across_restarts() {
        let state_dir = std::env::temp_dir().join(format!("work-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&state_dir);
        let existing = HashMap::from([("INI-1".to_owned(), reviewable("a"))]);
        let mut later = existing.clone();
        later.insert("INI-2".to_owned(), reviewable("b"));

        // First run ever: what is already reviewable does not light up.
        let mut first = app(&state_dir);
        first.update_seen(&existing);
        first.data.prs = existing;
        assert!(!first.awaits_first_look(&issue("INI-1")));

        // A pull request that shows up later does, and still does after a restart.
        first.update_seen(&later);
        first.data.prs = later.clone();
        assert!(first.awaits_first_look(&issue("INI-2")));
        let mut restarted = app(&state_dir);
        restarted.update_seen(&later);
        restarted.data.prs = later.clone();
        assert!(restarted.awaits_first_look(&issue("INI-2")));

        // Looking at it clears the mark, for good.
        restarted.mark_seen_pr("INI-2");
        assert!(!restarted.awaits_first_look(&issue("INI-2")));
        let mut again = app(&state_dir);
        again.data.prs = later;
        assert!(!again.awaits_first_look(&issue("INI-2")));

        let _ = std::fs::remove_dir_all(&state_dir);
    }
}
