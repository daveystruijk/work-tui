use std::collections::{HashMap, HashSet, VecDeque};

use color_eyre::{eyre::eyre, Result};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::{
    actions::{self, poll_ci_status::CiChange, Progress},
    events::{Event, EventLevel, EventLoadState, EventSource},
    github::{CheckStatus, GithubStatus, PrInfo},
    jira::{Issue, IssueType, JiraClient, JiraConfig},
    notify,
    repos::{self, RepoEntry},
};

/// Messages sent from background actions back to the main event loop.
///
/// Each variant corresponds to a result produced by an action in [`crate::actions`].
pub enum BgMsg {
    /// Current git branch resolved (from [`actions::initialize`]).
    CurrentBranch(String),
    /// Jira user identity resolved (from [`actions::initialize`]).
    Myself(Result<String>),
    /// Issues fetched from Jira (from [`actions::initialize`] / [`actions::refresh`]).
    Issues(Result<Vec<Issue>>),
    /// GitHub PRs fetched for all configured repos (from [`actions::fetch_github_prs`]).
    GithubPrs(Vec<PrInfo>),
    /// Active branches resolved (from [`actions::detect_active_branches`]).
    ActiveBranches(HashMap<String, String>),
    /// Issue events loaded for detail view (from [`actions::load_issue_events`]).
    IssueEvents(String, EventLoadState),
    /// Pick-up completed (from [`actions::pick_up`]).
    PickedUp(Result<String>),
    /// Finish completed — PR created (from [`actions::finish`]).
    Finished(Result<String>),
    /// Inline new issue created (from [`actions::create_inline_issue`]).
    InlineCreated(Result<String>),
    /// Labels updated for auto-labeling (from [`actions::auto_label`]).
    AutoLabeled(String, Result<()>),
    /// Label added to an issue (from [`actions::add_label`]).
    LabelAdded(Result<(String, String)>),
    /// A background task has started. The payload is the human-readable task name.
    TaskStarted(&'static str),
    /// A background task has finished. The payload is the human-readable task name.
    TaskFinished(&'static str),
    /// Generic progress update from any long-running action.
    ///
    /// Rendered in the status bar with step-by-step feedback.
    Progress(Progress),
    /// One or more PR CI statuses changed (from [`actions::poll_ci_status`]).
    CiStatusChanged(Vec<CiChange>),
}

/// A row in the display list — either a story header, an issue, or an inline-new placeholder.
#[derive(Debug, Clone)]
pub enum DisplayRow {
    /// A parent story header (not necessarily in the fetched issues list).
    /// Contains the parent issue key and summary.
    StoryHeader { key: String, summary: String },
    /// An actual issue row. `depth` is 0 for top-level, 1 for subtask under a story.
    Issue {
        index: usize, // index into `self.issues`
        depth: u8,
    },
    /// Inline new-issue placeholder being edited in the list view.
    InlineNew { depth: u8 },
}

/// State for the inline new-issue editor shown in the list view.
#[derive(Debug, Clone)]
pub struct InlineNewState {
    /// The text being typed as the summary.
    pub summary: String,
    /// Parent story key (if creating a subtask under a story).
    pub parent_key: Option<String>,
    /// Project key derived at creation time.
    pub project_key: String,
    /// The display-row index where the InlineNew row was inserted.
    pub row_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    List,
    Detail,
    New,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Editing,
}

#[derive(Debug, Clone)]
pub struct NewForm {
    pub summary: String,
    pub description: String,
    pub issue_type_idx: usize,
    pub issue_types: Vec<IssueType>,
    pub active_field: usize,
    pub project_key: String,
}

#[derive(Debug, Clone)]
pub struct LabelPickerState {
    pub selected: usize,
    pub filter: String,
}

pub struct App {
    pub should_quit: bool,
    pub edit_requested: bool,
    pub screen: Screen,
    pub input_mode: InputMode,
    pub issues: Vec<Issue>,
    pub selected_index: usize,
    pub jql: String,
    pub detail_scroll: u16,
    pub new_form: Option<NewForm>,
    pub repo_entries: Vec<RepoEntry>,
    pub repo_error: Option<String>,
    pub label_picker: Option<LabelPickerState>,
    pub status_message: String,
    pub loading: bool,
    pub client: JiraClient,
    pub jira_base_url: String,
    pub my_account_id: String,
    pub current_branch: String,
    pub pending_g: bool,
    pub list_area_height: u16,
    pub list_scroll_offset: usize,
    /// Maps issue key -> repo label for issues whose branch is currently checked out
    pub active_branches: HashMap<String, String>,
    /// Maps issue key -> GitHub PR status
    pub github_statuses: HashMap<String, GithubStatus>,
    /// Whether GitHub statuses are currently being loaded
    pub github_loading: bool,
    /// Spinner tick counter for animated loading indicators
    pub spinner_tick: usize,
    /// Maps issue key -> loaded event history for detail view
    pub issue_events: HashMap<String, EventLoadState>,
    /// Maps issue key -> latest synthesized activity for overview
    pub latest_activity: HashMap<String, Event>,
    /// Flattened display rows (story headers + issue rows) for the list view
    pub display_rows: Vec<DisplayRow>,
    /// Active inline new-issue editor, if any.
    pub inline_new: Option<InlineNewState>,
    /// Snapshot of issue keys → status names from the previous fetch, used to detect changes.
    pub prev_issue_snapshot: HashMap<String, String>,
    /// Issue keys that should blink because they are new or changed. Value = remaining ticks.
    pub highlight_ticks: HashMap<String, usize>,
    /// Configured GitHub repos to scan for PRs (from GITHUB_REPOS env var)
    pub github_repos: Vec<String>,
    /// Maps issue key -> matched PR info from GitHub
    pub github_prs: HashMap<String, PrInfo>,
    /// Currently running background task names
    pub running_tasks: HashSet<&'static str>,
    /// Recently completed task names (for brief status display), with remaining ticks
    pub completed_tasks: VecDeque<(&'static str, usize)>,
    /// Sender for background tasks to deliver results
    pub bg_tx: mpsc::UnboundedSender<BgMsg>,
    /// Receiver polled in the event loop
    pub bg_rx: mpsc::UnboundedReceiver<BgMsg>,
}

impl App {
    pub fn new() -> Result<Self> {
        let config = JiraConfig::from_env()?;
        let jira_base_url = config.base_url.trim_end_matches('/').to_string();
        let client = JiraClient::new(&config)?;
        let jql = config.default_jql.clone();
        let github_repos = config.github_repos.clone();

        let (bg_tx, bg_rx) = mpsc::unbounded_channel();
        let mut app = Self {
            should_quit: false,
            edit_requested: false,
            screen: Screen::List,
            input_mode: InputMode::Normal,
            issues: Vec::new(),
            selected_index: 0,
            jql,
            detail_scroll: 0,
            new_form: None,
            repo_entries: Vec::new(),
            repo_error: None,
            label_picker: None,
            status_message: "Loading...".to_string(),
            loading: true,
            client,
            jira_base_url,
            my_account_id: String::new(),
            current_branch: String::new(),
            pending_g: false,
            list_area_height: 0,
            list_scroll_offset: 0,
            active_branches: HashMap::new(),
            github_statuses: HashMap::new(),
            github_loading: false,
            spinner_tick: 0,
            issue_events: HashMap::new(),
            latest_activity: HashMap::new(),
            display_rows: Vec::new(),
            inline_new: None,
            prev_issue_snapshot: HashMap::new(),
            highlight_ticks: HashMap::new(),
            github_repos,
            github_prs: HashMap::new(),
            running_tasks: HashSet::new(),
            completed_tasks: VecDeque::new(),
            bg_tx,
            bg_rx,
        };

        app.reload_repo_entries();

        Ok(app)
    }

    /// Kick off all initialization work as background tasks.
    pub fn spawn_initialize(&self) {
        actions::initialize::spawn(self.bg_tx.clone(), self.client.clone(), self.jql.clone());
    }

    /// Spawn a full refresh (issues + PRs + statuses).
    pub fn spawn_refresh(&self) {
        actions::refresh::spawn(self.bg_tx.clone(), self.client.clone(), self.jql.clone());
    }

    /// Spawn GitHub PR fetch for all configured repos.
    pub fn spawn_github_prs(&self) {
        actions::fetch_github_prs::spawn(self.bg_tx.clone(), self.github_repos.clone());
    }

    /// Spawn the periodic CI status polling loop (every 10s).
    pub fn spawn_poll_ci_status(&self) {
        actions::poll_ci_status::spawn(
            self.bg_tx.clone(),
            self.github_repos.clone(),
            std::time::Duration::from_secs(10),
        );
    }

    /// Spawn active branch detection for all issues.
    pub fn spawn_active_branches(&self) {
        let issue_data: Vec<_> = self
            .issues
            .iter()
            .map(|issue| {
                let repos: Vec<_> = self
                    .repo_matches(issue)
                    .into_iter()
                    .map(|r| (r.label.clone(), r.path.clone()))
                    .collect();
                (issue.key.clone(), repos)
            })
            .collect();

        actions::detect_active_branches::spawn(self.bg_tx.clone(), issue_data);
    }

    /// Spawn loading issue events for the detail view.
    pub fn spawn_load_issue_events(&mut self) {
        let Some(issue) = self.selected_issue() else {
            return;
        };
        let issue_key = issue.key.clone();

        if matches!(
            self.issue_events.get(&issue_key),
            Some(EventLoadState::Loaded(_) | EventLoadState::Loading)
        ) {
            return;
        }

        let gh_pr = self.github_statuses.get(&issue_key).cloned();
        let repo_path = self.repo_matches(issue).first().map(|r| r.path.clone());

        // Mark as loading immediately
        self.issue_events
            .insert(issue_key.clone(), EventLoadState::Loading);

        actions::load_issue_events::spawn(
            self.bg_tx.clone(),
            self.client.clone(),
            issue_key,
            gh_pr,
            repo_path,
        );
    }

    /// Spawn auto-labeling for issues with matched PRs but missing repo labels.
    fn spawn_auto_label(&self) {
        let to_label: Vec<_> = self
            .issues
            .iter()
            .filter_map(|issue| {
                let pr = self.github_prs.get(&issue.key)?;
                let repo_name = pr.repo_slug.rsplit('/').next().unwrap_or(&pr.repo_slug);
                let target_normalized = repos::normalize_label(repo_name);
                let labels = issue.labels();
                let already_has = labels
                    .iter()
                    .any(|l| repos::normalize_label(l) == target_normalized);
                if already_has {
                    return None;
                }
                let mut new_labels = labels;
                new_labels.push(repo_name.to_string());
                Some((issue.key.clone(), new_labels))
            })
            .collect();

        actions::auto_label::spawn(self.bg_tx.clone(), self.client.clone(), to_label);
    }

    /// Process a background message. Called from the event loop.
    pub fn handle_bg_msg(&mut self, msg: BgMsg) {
        match msg {
            BgMsg::CurrentBranch(branch) => {
                self.current_branch = branch;
            }
            BgMsg::Myself(result) => match result {
                Ok(account_id) => self.my_account_id = account_id,
                Err(err) => {
                    self.status_message = format!("Failed to fetch user: {err}");
                }
            },
            BgMsg::Issues(result) => match result {
                Ok(issues) => {
                    let prev_snapshot = self
                        .issues
                        .iter()
                        .map(|i| {
                            let status = i.status().map(|s| s.name).unwrap_or_default();
                            (i.key.clone(), status)
                        })
                        .collect::<HashMap<_, _>>();
                    let selected_key = self.selected_issue().map(|i| i.key.clone());
                    self.issues = issues;
                    self.rebuild_display_rows();
                    if !prev_snapshot.is_empty() {
                        for issue in &self.issues {
                            let key = &issue.key;
                            let current_status = issue.status().map(|s| s.name).unwrap_or_default();
                            match prev_snapshot.get(key) {
                                None => {
                                    self.highlight_ticks.insert(key.clone(), 75);
                                }
                                Some(old_status) if *old_status != current_status => {
                                    self.highlight_ticks.insert(key.clone(), 75);
                                }
                                _ => {}
                            }
                        }
                    }
                    self.prev_issue_snapshot = self
                        .issues
                        .iter()
                        .map(|i| {
                            let status = i.status().map(|s| s.name).unwrap_or_default();
                            (i.key.clone(), status)
                        })
                        .collect();
                    let next_index = selected_key
                        .and_then(|key| {
                            self.display_rows.iter().position(|row| match row {
                                DisplayRow::Issue { index, .. } => self.issues[*index].key == key,
                                _ => false,
                            })
                        })
                        .unwrap_or(0);
                    self.selected_index = if self.display_rows.is_empty() {
                        0
                    } else {
                        next_index.min(self.display_rows.len() - 1)
                    };
                    self.loading = false;
                    self.github_statuses.clear();
                    self.issue_events.clear();
                    self.github_prs.clear();
                    // Chain: fetch branches, PRs, statuses
                    self.spawn_active_branches();
                    self.spawn_github_prs();
                }
                Err(err) => {
                    self.loading = false;
                    self.status_message = format!("Failed to load issues: {err}");
                }
            },
            BgMsg::ActiveBranches(active) => {
                self.active_branches = active;
            }
            BgMsg::GithubPrs(all_prs) => {
                self.github_prs.clear();
                self.github_statuses.clear();
                for issue in &self.issues {
                    let key_lower = issue.key.to_lowercase();
                    let matched = all_prs
                        .iter()
                        .find(|pr| pr.head_branch.to_lowercase().starts_with(&key_lower));
                    if let Some(pr) = matched {
                        self.github_prs.insert(issue.key.clone(), pr.clone());
                        self.github_statuses
                            .insert(issue.key.clone(), GithubStatus::Found(pr.clone()));
                    }
                }
                self.spawn_auto_label();
                self.github_loading = false;
                self.refresh_latest_activity();
                if self.running_tasks.is_empty() {
                    self.status_message = "Ready".into();
                }
            }
            BgMsg::IssueEvents(key, state) => {
                self.issue_events.insert(key, state);
            }
            BgMsg::PickedUp(result) => match result {
                Ok(branch) => {
                    self.current_branch = branch.clone();
                    self.status_message = format!("Picked up {branch}");
                    self.spawn_active_branches();
                }
                Err(err) => {
                    self.status_message = format!("Failed to pick up issue: {err}");
                }
            },
            BgMsg::Finished(result) => match result {
                Ok(pr_url) => {
                    self.status_message = format!("PR created: {pr_url}");
                    self.spawn_refresh();
                }
                Err(err) => {
                    self.status_message = format!("Finish failed: {err}");
                }
            },
            BgMsg::InlineCreated(result) => match result {
                Ok(key) => {
                    self.status_message = format!("Created {key}");
                    self.input_mode = InputMode::Normal;
                    self.spawn_refresh();
                }
                Err(err) => {
                    self.status_message = format!("Failed: {err}");
                    self.input_mode = InputMode::Normal;
                    self.cancel_inline_new();
                }
            },
            BgMsg::AutoLabeled(_key, _result) => {
                // Silent — auto-labeling is best-effort
            }
            BgMsg::LabelAdded(result) => match result {
                Ok((issue_key, label)) => {
                    self.status_message = format!("Added label {label} to {issue_key}");
                    self.spawn_refresh();
                }
                Err(err) => {
                    self.status_message = format!("Failed to add label: {err}");
                }
            },
            BgMsg::TaskStarted(name) => {
                self.running_tasks.insert(name);
                self.update_task_status();
            }
            BgMsg::TaskFinished(name) => {
                self.running_tasks.remove(name);
                self.completed_tasks.push_back((name, 50));
                self.update_task_status();
            }
            BgMsg::Progress(progress) => {
                self.status_message = progress.to_string();
            }
            BgMsg::CiStatusChanged(changes) => {
                for change in &changes {
                    let status_label = match &change.new_status {
                        CheckStatus::Pass => "passed ✓",
                        CheckStatus::Fail => "failed ✗",
                        CheckStatus::Pending => "running",
                    };
                    notify::send(
                        "CI Status Changed",
                        &format!(
                            "PR #{} ({}) — CI {}",
                            change.pr_number, change.head_branch, status_label
                        ),
                    );
                }
            }
        }
    }

    /// Spawn pick-up issue in background.
    pub fn spawn_pick_up(&mut self) {
        let Some(issue) = self.selected_issue() else {
            return;
        };
        let issue_key = issue.key.clone();
        let issue_summary = issue.summary().unwrap_or_default();
        let issue_description = issue.description().unwrap_or_default();
        let repos = self.repo_matches(issue);
        if repos.is_empty() {
            self.status_message = format!("Cannot pick up {issue_key}: no linked repo");
            return;
        }

        actions::pick_up::spawn(
            self.bg_tx.clone(),
            self.client.clone(),
            issue_key,
            issue_summary,
            issue_description,
            repos[0].path.clone(),
            self.my_account_id.clone(),
        );
    }

    /// Spawn finish workflow in background.
    pub fn spawn_finish(&mut self) {
        let Some(issue) = self.selected_issue() else {
            return;
        };
        let issue_key = issue.key.clone();
        let issue_summary = issue.summary().unwrap_or_default();
        let repos = self.repo_matches(issue);
        if repos.is_empty() {
            self.status_message = format!("Cannot finish {issue_key}: no linked repo");
            return;
        }

        actions::finish::spawn(
            self.bg_tx.clone(),
            self.client.clone(),
            issue_key,
            issue_summary,
            repos[0].path.clone(),
        );
    }

    /// Spawn inline new issue creation in background.
    pub fn spawn_submit_inline_new(&mut self) {
        let Some(state) = self.inline_new.take() else {
            return;
        };
        let summary = state.summary.trim().to_string();
        if summary.is_empty() {
            self.remove_inline_row(state.row_index);
            self.input_mode = InputMode::Normal;
            self.status_message = "Summary cannot be empty".into();
            return;
        }

        actions::create_inline_issue::spawn(
            self.bg_tx.clone(),
            self.client.clone(),
            state.project_key,
            summary,
            state.parent_key,
        );
    }

    pub async fn refresh_issues(&mut self) -> Result<()> {
        self.loading = true;
        let selected_key = self.selected_issue().map(|issue| issue.key.clone());
        let issues = self.client.search(&self.jql).await?;
        self.loading = false;
        self.issues = issues;
        self.rebuild_display_rows();

        // Restore selection to the same issue key if possible
        let next_index = selected_key
            .and_then(|key| {
                self.display_rows.iter().position(|row| match row {
                    DisplayRow::Issue { index, .. } => self.issues[*index].key == key,
                    _ => false,
                })
            })
            .unwrap_or(0);
        self.selected_index = if self.display_rows.is_empty() {
            0
        } else {
            next_index.min(self.display_rows.len() - 1)
        };

        self.github_statuses.clear();
        self.issue_events.clear();
        self.github_prs.clear();
        self.latest_activity.clear();
        Ok(())
    }

    /// Returns the issue for the currently selected display row, if any.
    pub fn selected_issue(&self) -> Option<&Issue> {
        let row = self.display_rows.get(self.selected_index)?;
        match row {
            DisplayRow::Issue { index, .. } => self.issues.get(*index),
            DisplayRow::StoryHeader { .. } | DisplayRow::InlineNew { .. } => None,
        }
    }

    /// Returns the story key if the current selection is inside a story group.
    /// Walks backwards from `selected_index` to find the enclosing StoryHeader.
    pub fn enclosing_story_key(&self) -> Option<String> {
        for i in (0..=self.selected_index).rev() {
            match &self.display_rows[i] {
                DisplayRow::StoryHeader { key, .. } => return Some(key.clone()),
                DisplayRow::Issue { depth: 0, .. } => return None,
                _ => continue,
            }
        }
        None
    }

    /// Build the flattened display rows from the current issues list.
    /// Groups subtasks under their parent story headers, sorted by key.
    pub fn rebuild_display_rows(&mut self) {
        use std::collections::HashMap as StdMap;

        // Collect parent keys for issues that have a parent
        // parent_key -> (summary, Vec<issue_index>)
        let mut parent_groups: StdMap<String, (String, Vec<usize>)> = StdMap::new();
        let mut standalone_indices: Vec<usize> = Vec::new();

        for (idx, issue) in self.issues.iter().enumerate() {
            if let Some(parent) = issue.parent() {
                let parent_key = parent.key.clone();
                let parent_summary = parent.summary().unwrap_or_default();
                let entry = parent_groups
                    .entry(parent_key)
                    .or_insert_with(|| (parent_summary, Vec::new()));
                entry.1.push(idx);
            } else {
                standalone_indices.push(idx);
            }
        }

        // Sort children within each parent group by creation date (newest first)
        let created_str = |idx: &usize| -> String {
            self.issues[*idx]
                .field::<String>("created")
                .and_then(|r| r.ok())
                .unwrap_or_default()
        };
        for (_, children) in parent_groups.values_mut() {
            children.sort_by(|a, b| created_str(b).cmp(&created_str(a)));
        }

        // Track which standalone issues are themselves parents
        let parent_key_set: HashSet<String> = parent_groups.keys().cloned().collect();

        // Build top-level entries: each is either a standalone issue or a story group.
        // A story group sorts by its parent key.
        enum TopLevel {
            Standalone(usize),
            StoryGroup {
                key: String,
                summary: String,
                /// Index into self.issues if the parent itself is in the list
                parent_issue_idx: Option<usize>,
                children: Vec<usize>,
            },
        }

        let mut top_levels: Vec<TopLevel> = Vec::new();
        let mut emitted_parents: HashSet<String> = HashSet::new();

        for &idx in &standalone_indices {
            let issue_key = &self.issues[idx].key;
            if parent_key_set.contains(issue_key) {
                emitted_parents.insert(issue_key.clone());
                let (_, children) = parent_groups.remove(issue_key.as_str()).unwrap();
                top_levels.push(TopLevel::StoryGroup {
                    key: issue_key.clone(),
                    summary: self.issues[idx].summary().unwrap_or_default(),
                    parent_issue_idx: Some(idx),
                    children,
                });
            } else {
                top_levels.push(TopLevel::Standalone(idx));
            }
        }

        // Remaining parent groups whose parent is NOT in the issues list
        for (parent_key, (summary, children)) in parent_groups {
            top_levels.push(TopLevel::StoryGroup {
                key: parent_key,
                summary,
                parent_issue_idx: None,
                children,
            });
        }

        // Sort all top-level entries by creation date (newest first)
        top_levels.sort_by(|a, b| {
            let date_b = top_level_created(b, &self.issues);
            let date_a = top_level_created(a, &self.issues);
            date_b.cmp(&date_a)
        });

        fn top_level_created(entry: &TopLevel, issues: &[Issue]) -> Option<String> {
            match entry {
                TopLevel::Standalone(idx) => {
                    issues[*idx].field::<String>("created").and_then(|r| r.ok())
                }
                TopLevel::StoryGroup {
                    parent_issue_idx,
                    children,
                    ..
                } => parent_issue_idx
                    .and_then(|idx| issues[idx].field::<String>("created").and_then(|r| r.ok()))
                    .or_else(|| {
                        children
                            .iter()
                            .filter_map(|idx| {
                                issues[*idx].field::<String>("created").and_then(|r| r.ok())
                            })
                            .min()
                    }),
            }
        }

        // Flatten into display rows
        let mut rows = Vec::new();
        for entry in top_levels {
            match entry {
                TopLevel::Standalone(idx) => {
                    rows.push(DisplayRow::Issue {
                        index: idx,
                        depth: 0,
                    });
                }
                TopLevel::StoryGroup {
                    key,
                    summary,
                    parent_issue_idx,
                    children,
                } => {
                    rows.push(DisplayRow::StoryHeader {
                        key: key.clone(),
                        summary,
                    });
                    // Skip the parent issue row — it duplicates the story header
                    if let Some(idx) = parent_issue_idx {
                        let issue_key = &self.issues[idx].key;
                        if *issue_key != key {
                            rows.push(DisplayRow::Issue {
                                index: idx,
                                depth: 1,
                            });
                        }
                    }
                    for child_idx in children {
                        rows.push(DisplayRow::Issue {
                            index: child_idx,
                            depth: 1,
                        });
                    }
                }
            }
        }

        self.display_rows = rows;
    }

    pub async fn submit_new(&mut self) -> Result<String> {
        let form = match self.new_form.take() {
            Some(form) => form,
            None => return Err(eyre!("No new issue form active")),
        };
        let issue_type = form
            .issue_types
            .get(form.issue_type_idx)
            .ok_or_else(|| eyre!("Invalid issue type index"))?;
        let description = if form.description.is_empty() {
            None
        } else {
            Some(form.description.as_str())
        };
        let issue_key = self
            .client
            .create_issue(
                &form.project_key,
                &issue_type.id,
                &form.summary,
                description,
                None,
            )
            .await?;
        self.refresh_issues().await?;
        Ok(issue_key)
    }

    pub fn enter_detail(&mut self) {
        self.detail_scroll = 0;
        self.screen = Screen::Detail;
    }

    pub fn refresh_latest_activity(&mut self) {
        self.latest_activity.clear();
        for issue in &self.issues {
            let status_name = issue.status().map(|s| s.name).unwrap_or_default();

            if let Some(gh_status) = self.github_statuses.get(&issue.key) {
                let event = match gh_status {
                    GithubStatus::Found(pr) => {
                        if pr.state == "MERGED" {
                            Some(Event {
                                at: String::new(),
                                source: EventSource::GitHub,
                                level: EventLevel::Success,
                                title: "PR merged".to_string(),
                                detail: Some(format!("#{}", pr.number)),
                            })
                        } else if pr.state == "CLOSED" {
                            Some(Event {
                                at: String::new(),
                                source: EventSource::GitHub,
                                level: EventLevel::Warning,
                                title: "PR closed".to_string(),
                                detail: Some(format!("#{}", pr.number)),
                            })
                        } else {
                            match &pr.checks {
                                CheckStatus::Fail => Some(Event {
                                    at: String::new(),
                                    source: EventSource::GitHub,
                                    level: EventLevel::Error,
                                    title: "CI failed".to_string(),
                                    detail: Some(format!("#{}", pr.number)),
                                }),
                                CheckStatus::Pending => Some(Event {
                                    at: String::new(),
                                    source: EventSource::GitHub,
                                    level: EventLevel::Warning,
                                    title: "CI running".to_string(),
                                    detail: Some(format!("#{}", pr.number)),
                                }),
                                CheckStatus::Pass => Some(Event {
                                    at: String::new(),
                                    source: EventSource::GitHub,
                                    level: EventLevel::Success,
                                    title: "CI passed".to_string(),
                                    detail: Some(format!("#{}", pr.number)),
                                }),
                            }
                        }
                    }
                    _ => None,
                };
                if let Some(event) = event {
                    self.latest_activity.insert(issue.key.clone(), event);
                    continue;
                }
            }

            let (level, title) = if status_name.to_lowercase().contains("done") {
                (EventLevel::Success, "Done".to_string())
            } else if status_name.to_lowercase().contains("progress") {
                (EventLevel::Warning, "In Progress".to_string())
            } else if status_name.to_lowercase().contains("review") {
                (EventLevel::Info, "In Review".to_string())
            } else if status_name.to_lowercase().contains("blocked") {
                (EventLevel::Error, "Blocked".to_string())
            } else {
                (EventLevel::Neutral, status_name.clone())
            };

            self.latest_activity.insert(
                issue.key.clone(),
                Event {
                    at: String::new(),
                    source: EventSource::Jira,
                    level,
                    title,
                    detail: None,
                },
            );
        }
    }

    /// Open $EDITOR to edit the selected issue's summary and description.
    ///
    /// The file format mirrors a git commit message: the first line is the
    /// summary, followed by a blank line, then the description body.
    /// Returns `(new_summary, new_description)`, or `None` if unchanged.
    pub fn edit_issue_via_editor(&self) -> Result<Option<(String, String)>> {
        let issue = self.selected_issue().ok_or_else(|| eyre!("No issue selected"))?;
        let original_summary = issue.summary().unwrap_or_default();
        let original_description = issue.description().unwrap_or_default();

        let content = format!("{original_summary}\n\n{original_description}");

        let dir = std::env::temp_dir();
        let path = dir.join(format!("{}.txt", issue.key));
        std::fs::write(&path, &content)?;

        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
        let status = std::process::Command::new(&editor)
            .arg(&path)
            .status()?;

        if !status.success() {
            std::fs::remove_file(&path).ok();
            return Err(eyre!("Editor exited with non-zero status"));
        }

        let raw = std::fs::read_to_string(&path)?;
        std::fs::remove_file(&path).ok();

        let new_summary = raw.lines().next().unwrap_or("").trim().to_string();
        let new_description = raw
            .splitn(2, '\n')
            .nth(1)
            .unwrap_or("")
            .trim_start_matches('\n')
            .trim_end()
            .to_string();

        if new_summary.is_empty() {
            return Err(eyre!("Summary cannot be empty"));
        }

        if new_summary == original_summary && new_description == original_description {
            return Ok(None);
        }

        Ok(Some((new_summary, new_description)))
    }

    pub async fn enter_new(&mut self) -> Result<()> {
        let project_key = self.derive_project_key();
        let issue_types = self.client.get_issue_types(&project_key).await?;
        if issue_types.is_empty() {
            return Err(eyre!("No issue types available"));
        }
        self.new_form = Some(NewForm {
            summary: String::new(),
            description: String::new(),
            issue_type_idx: 0,
            issue_types,
            active_field: 0,
            project_key,
        });
        self.screen = Screen::New;
        self.input_mode = InputMode::Editing;
        Ok(())
    }

    pub fn back_to_list(&mut self) {
        self.screen = Screen::List;
        self.input_mode = InputMode::Normal;
        self.new_form = None;
        self.label_picker = None;
        self.cancel_inline_new();
    }

    /// Start an inline new-issue row. If inside a story group, it becomes a
    /// subtask of that story. Otherwise falls back to the full-screen form.
    pub fn start_inline_new(&mut self) -> bool {
        let story_key = self.enclosing_story_key();
        let Some(parent_key) = story_key else {
            return false; // not inside a story — caller should fall back to enter_new()
        };

        let project_key = self.derive_project_key();

        // Find the end of the current story group to insert the new row there
        let insert_at = self.find_story_group_end(self.selected_index) + 1;
        let depth = 1u8;

        self.display_rows
            .insert(insert_at, DisplayRow::InlineNew { depth });

        let state = InlineNewState {
            summary: String::new(),
            parent_key: Some(parent_key),
            project_key,
            row_index: insert_at,
        };
        self.inline_new = Some(state);
        self.selected_index = insert_at;
        self.input_mode = InputMode::Editing;
        true
    }

    /// Submit the inline new issue to Jira, then refresh.

    /// Cancel the inline new issue and remove the placeholder row.
    pub fn cancel_inline_new(&mut self) {
        let Some(state) = self.inline_new.take() else {
            return;
        };
        self.remove_inline_row(state.row_index);
        self.input_mode = InputMode::Normal;
    }

    /// Returns true if an inline new-issue editor is active.
    pub fn inline_new_active(&self) -> bool {
        self.inline_new.is_some()
    }

    /// Find the last row index belonging to the story group that contains `from`.
    fn find_story_group_end(&self, from: usize) -> usize {
        let mut end = from;
        for i in (from + 1)..self.display_rows.len() {
            match &self.display_rows[i] {
                DisplayRow::Issue { depth, .. } if *depth > 0 => end = i,
                DisplayRow::InlineNew { depth } if *depth > 0 => end = i,
                _ => break,
            }
        }
        end
    }

    /// Remove the InlineNew row at the given index and fix up selection.
    fn remove_inline_row(&mut self, row_index: usize) {
        if row_index < self.display_rows.len() {
            if matches!(self.display_rows[row_index], DisplayRow::InlineNew { .. }) {
                self.display_rows.remove(row_index);
            }
        }
        // Clamp selection
        if !self.display_rows.is_empty() {
            self.selected_index = self.selected_index.min(self.display_rows.len() - 1);
        } else {
            self.selected_index = 0;
        }
    }

    fn derive_project_key(&self) -> String {
        if let Some(cap) = self
            .jql
            .split_whitespace()
            .collect::<Vec<_>>()
            .windows(3)
            .find(|window| window[0].eq_ignore_ascii_case("project") && window[1] == "=")
        {
            return cap[2].trim_matches('"').to_string();
        }

        if let Some(project_key) = self
            .selected_issue()
            .and_then(|issue| issue.project())
            .map(|project| project.key)
        {
            return project_key;
        }

        self.issues
            .first()
            .and_then(|issue| issue.project())
            .map(|project| project.key)
            .unwrap_or_else(|| "WORK".to_string())
    }

    pub fn reload_repo_entries(&mut self) {
        match repos::scan_repos() {
            Ok(entries) => {
                self.repo_entries = entries;
                self.repo_error = None;
            }
            Err(err) => {
                let message = format!("Failed to scan repos: {err}");
                self.repo_entries.clear();
                self.repo_error = Some(message.clone());
                self.status_message = message;
            }
        }
    }

    pub fn open_label_picker(&mut self) {
        self.reload_repo_entries();
        if self.repo_entries.is_empty() {
            if self.repo_error.is_none() {
                self.status_message = "No repositories found in REPOS_DIR".to_string();
            }
            return;
        }
        self.label_picker = Some(LabelPickerState {
            selected: 0,
            filter: String::new(),
        });
    }

    pub fn close_label_picker(&mut self) {
        self.label_picker = None;
    }

    pub fn label_picker_active(&self) -> bool {
        self.label_picker.is_some()
    }

    pub fn filtered_repo_entries(&self) -> Vec<&RepoEntry> {
        let Some(picker) = &self.label_picker else {
            return Vec::new();
        };
        if picker.filter.is_empty() {
            return self.repo_entries.iter().collect();
        }
        let query = picker.filter.to_lowercase();
        self.repo_entries
            .iter()
            .filter(|e| e.label.to_lowercase().contains(&query))
            .collect()
    }

    pub fn move_label_picker_selection(&mut self, down: bool) {
        let count = self.filtered_repo_entries().len();
        let Some(picker) = self.label_picker.as_mut() else {
            return;
        };
        if count == 0 {
            picker.selected = 0;
            return;
        }
        if down {
            picker.selected = (picker.selected + 1).min(count - 1);
            return;
        }
        if picker.selected == 0 {
            return;
        }
        picker.selected -= 1;
    }

    pub fn label_picker_entry(&self) -> Option<&RepoEntry> {
        let picker = self.label_picker.as_ref()?;
        self.filtered_repo_entries().get(picker.selected).copied()
    }

    pub fn label_picker_type_char(&mut self, ch: char) {
        let Some(picker) = self.label_picker.as_mut() else {
            return;
        };
        picker.filter.push(ch);
        picker.selected = 0;
    }

    pub fn label_picker_backspace(&mut self) {
        let Some(picker) = self.label_picker.as_mut() else {
            return;
        };
        picker.filter.pop();
        picker.selected = 0;
    }

    pub fn repo_matches(&self, issue: &Issue) -> Vec<&RepoEntry> {
        if self.repo_entries.is_empty() {
            return Vec::new();
        }
        let labels = issue.labels();
        if labels.is_empty() {
            return Vec::new();
        }
        let normalized: HashSet<String> = labels
            .iter()
            .map(|label| repos::normalize_label(label))
            .collect();
        self.repo_entries
            .iter()
            .filter(|entry| normalized.contains(&entry.normalized))
            .collect()
    }

    /// Check each repo for branches matching issue keys and cache the results.
    /// Refresh GitHub PR statuses for all issues that have matching repos.
    /// Sets each issue to Loading first, then resolves them one by one so the
    /// UI can show incremental progress.
    pub fn tick_spinner(&mut self) {
        self.spinner_tick = self.spinner_tick.wrapping_add(1);
        self.highlight_ticks.retain(|_, ticks| {
            *ticks = ticks.saturating_sub(1);
            *ticks > 0
        });
    }

    /// Returns `true` when any background work is in progress.
    pub fn is_busy(&self) -> bool {
        self.loading
            || self.github_loading
            || !self.running_tasks.is_empty()
            || self.status_message.starts_with('[')
    }

    /// Compose `status_message` from currently running and recently completed tasks.
    fn update_task_status(&mut self) {
        if !self.running_tasks.is_empty() {
            let names: Vec<_> = self.running_tasks.iter().copied().collect();
            self.status_message = format!("[{}]", names.join(", "));
        } else if let Some((name, _)) = self.completed_tasks.back() {
            self.status_message = format!("{name} done");
        }
    }

    /// Tick down completed-task display timers; called from the main loop tick.
    pub fn tick_completed_tasks(&mut self) {
        self.completed_tasks.retain_mut(|(_, ticks)| {
            *ticks = ticks.saturating_sub(1);
            *ticks > 0
        });
    }

    pub fn add_label_from_picker(&mut self) -> bool {
        let Some(entry) = self.label_picker_entry().cloned() else {
            self.status_message = "No repository selected".to_string();
            return false;
        };
        let Some(issue) = self.selected_issue() else {
            self.status_message = "No issue selected".to_string();
            return false;
        };
        let issue_key = issue.key.clone();
        let labels = issue.labels();
        let target_normalized = repos::normalize_label(&entry.label);
        let already_has = labels
            .iter()
            .any(|l| repos::normalize_label(l) == target_normalized);
        if already_has {
            self.status_message = format!("{issue_key} already labeled with {}", entry.label);
            return false;
        }
        actions::add_label::spawn(
            self.bg_tx.clone(),
            self.client.clone(),
            issue_key,
            entry.label.clone(),
            labels,
        );
        true
    }

    pub async fn open_selected_issue_in_browser(&mut self) -> Result<()> {
        let issue_key = match self.selected_issue() {
            Some(issue) => issue.key.clone(),
            None => return Err(eyre!("No issue selected")),
        };

        let url = format!("{}/browse/{}", self.jira_base_url, issue_key);
        open_url_in_browser(&url).await?;
        self.status_message = format!("Opened {} in browser", url);
        Ok(())
    }

    pub async fn open_selected_pr_in_browser(&mut self) -> Result<()> {
        let issue_key = match self.selected_issue() {
            Some(issue) => issue.key.clone(),
            None => return Err(eyre!("No issue selected")),
        };

        let pr = self
            .github_prs
            .get(&issue_key)
            .ok_or_else(|| eyre!("No PR found for {issue_key}"))?;

        let url = pr.url.clone();
        let number = pr.number;
        open_url_in_browser(&url).await?;
        self.status_message = format!("Opened PR #{number} in browser");
        Ok(())
    }
}

async fn open_url_in_browser(url: &str) -> Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.arg(url);
        command
    } else if cfg!(target_os = "windows") {
        let mut command = Command::new("cmd");
        command.arg("/C").arg("start").arg("").arg(url);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };

    let output = command.output().await?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        return Err(eyre!("Failed to open browser"));
    }

    Err(eyre!(stderr))
}
