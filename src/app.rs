use std::collections::{HashMap, HashSet};

use chrono::Utc;
use color_eyre::{eyre::eyre, Result};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::{
    actions::{self, ActionMessage},
    apis::{
        github::{CheckRun, CheckStatus, CheckStep, GithubStatus, PrInfo},
        jira::{Issue, JiraClient, JiraConfig},
    },
    cache::{self, Cache},
    events::{Event, EventLevel, EventSource},
    repos::{self, RepoEntry},
    ui::{CiLogPopupState, ListViewState, SidebarState, StatusBarState, UiAnimationState},
};

/// Compute duration in seconds between two ISO 8601 timestamps.
fn parse_duration_secs(start: &str, end: &str) -> Option<u64> {
    let s = start.parse::<chrono::DateTime<chrono::Utc>>().ok()?;
    let e = end.parse::<chrono::DateTime<chrono::Utc>>().ok()?;
    Some(e.signed_duration_since(s).num_seconds().unsigned_abs())
}

/// Seconds elapsed since an ISO 8601 timestamp.
fn elapsed_since_iso(ts: &str) -> Option<u64> {
    let started = ts.parse::<chrono::DateTime<chrono::Utc>>().ok()?;
    Some(
        Utc::now()
            .signed_duration_since(started)
            .num_seconds()
            .unsigned_abs(),
    )
}

/// Format seconds as a human-readable duration (e.g. "2m", "1m30s").
pub fn format_duration(secs: u64) -> String {
    if secs < 60 {
        return format!("{secs}s");
    }
    let m = secs / 60;
    let s = secs % 60;
    if s == 0 {
        format!("{m}m")
    } else {
        format!("{m}m{s:02}s")
    }
}

/// A row in the display list — either a story header, an issue, or an inline-new placeholder.
#[derive(Debug, Clone)]
pub enum DisplayRow {
    /// A parent story header (not necessarily in the fetched issues list).
    /// Contains the parent issue key, summary, and nesting depth.
    StoryHeader {
        key: String,
        summary: String,
        depth: u8,
    },
    /// An actual issue row. `depth` is 0 for top-level, 1+ for subtask under a story.
    Issue {
        /// Index into `self.issues` for top-level issues, or into
        /// `self.story_children[parent_key]` for dynamically loaded children.
        index: usize,
        depth: u8,
        /// If set, this issue comes from `story_children[parent_key]` rather than `self.issues`.
        child_of: Option<String>,
    },
    /// Inline new-issue placeholder being edited in the list view.
    InlineNew { depth: u8 },
    /// Spinner row shown while children are being fetched.
    Loading { depth: u8 },
    /// "No issues" placeholder shown when an expanded story has no children.
    Empty { depth: u8 },
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Editing,
    Searching,
}

pub struct App {
    pub should_quit: bool,

    pub screen: Screen,
    pub input_mode: InputMode,
    pub issues: Vec<Issue>,
    pub selected_index: usize,
    pub jql: String,
    pub repo_entries: Vec<RepoEntry>,
    pub repo_error: Option<String>,
    pub list_view: ListViewState,
    pub status_bar: StatusBarState,
    pub loading: bool,
    pub client: JiraClient,
    pub jira_base_url: String,
    pub my_account_id: String,
    pub current_branch: String,
    pub pending_g: bool,
    /// Maps issue key -> repo label for issues whose branch is currently checked out
    pub active_branches: HashMap<String, String>,
    /// Maps issue key -> GitHub PR status
    pub github_statuses: HashMap<String, GithubStatus>,
    /// Whether GitHub statuses are currently being loaded
    pub github_loading: bool,
    pub animation: UiAnimationState,
    /// Maps issue key -> latest synthesized activity for overview
    pub latest_activity: HashMap<String, Event>,
    /// Flattened display rows (story headers + issue rows) for the list view
    pub display_rows: Vec<DisplayRow>,
    /// Active inline new-issue editor, if any.
    pub inline_new: Option<InlineNewState>,
    /// Current search/filter text for the issue list.
    pub search_filter: String,

    /// Story keys that are currently collapsed (children hidden).
    pub collapsed_stories: HashSet<String>,
    /// Dynamically loaded child issues for expanded stories, keyed by parent key.
    pub story_children: HashMap<String, Vec<Issue>>,
    pub sidebar: SidebarState,
    /// Maps issue key -> matched PR info from GitHub
    pub github_prs: HashMap<String, PrInfo>,
    /// Historical CI check durations in seconds, keyed by "repo_slug/check_name".
    pub check_durations: HashMap<String, u64>,
    /// Currently running background task names
    pub running_tasks: HashSet<String>,
    /// Sender for background tasks to deliver results
    pub bg_tx: mpsc::UnboundedSender<ActionMessage>,
    /// Receiver polled in the event loop
    pub bg_rx: mpsc::UnboundedReceiver<ActionMessage>,
    /// Last time a CI/PR refresh was triggered (for auto-refresh throttling)
    pub last_ci_refresh: std::time::Instant,
    pub ci_log_popup: CiLogPopupState,
}

impl App {
    pub fn new() -> Result<Self> {
        let config = JiraConfig::from_env()?;
        let jira_base_url = config.base_url.trim_end_matches('/').to_string();
        let client = JiraClient::new(&config)?;
        let jql = config.default_jql.clone();
        let (bg_tx, bg_rx) = mpsc::unbounded_channel();
        let mut app = Self {
            should_quit: false,
            screen: Screen::List,
            input_mode: InputMode::Normal,
            issues: Vec::new(),
            selected_index: 0,
            jql,
            repo_entries: Vec::new(),
            repo_error: None,
            list_view: ListViewState::default(),
            status_bar: StatusBarState::default(),
            loading: true,
            client,
            jira_base_url,
            my_account_id: String::new(),
            current_branch: String::new(),
            pending_g: false,
            active_branches: HashMap::new(),
            github_statuses: HashMap::new(),
            github_loading: false,
            animation: UiAnimationState::default(),
            latest_activity: HashMap::new(),
            display_rows: Vec::new(),
            inline_new: None,
            search_filter: String::new(),
            collapsed_stories: HashSet::new(),
            story_children: HashMap::new(),
            sidebar: SidebarState::default(),
            github_prs: HashMap::new(),
            check_durations: HashMap::new(),
            running_tasks: HashSet::new(),
            bg_tx,
            bg_rx,
            last_ci_refresh: std::time::Instant::now(),
            ci_log_popup: CiLogPopupState::default(),
        };

        app.status_bar.message = "Loading...".to_string();

        app.check_durations = cache::load().check_durations;

        app.reload_repo_entries();

        Ok(app)
    }

    /// Kick off all initialization work as background tasks.
    pub fn spawn_initialize(&self) {
        actions::initialize::spawn(self.bg_tx.clone(), self.client.clone(), self.jql.clone());
    }

    /// Spawn a full refresh (issues + PRs + statuses).
    pub fn spawn_refresh(&mut self) {
        actions::refresh::spawn(self.bg_tx.clone(), self.client.clone(), self.jql.clone());
        self.last_ci_refresh = std::time::Instant::now();
    }

    /// Collect unique GitHub slugs from repos that match current issue labels.
    fn matched_repo_slugs(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        for issue in &self.issues {
            for entry in self.repo_matches(issue) {
                if let Some(slug) = &entry.github_slug {
                    seen.insert(slug.clone());
                }
            }
        }
        seen.into_iter().collect()
    }

    /// Spawn GitHub PR fetch for repos matching current issue labels.
    pub fn spawn_github_prs(&mut self) {
        actions::fetch_github_prs::spawn(self.bg_tx.clone(), self.matched_repo_slugs());
        self.last_ci_refresh = std::time::Instant::now();
    }

    /// Spawn GitHub PR fetch only for repos that currently have linked PRs.
    pub fn spawn_github_prs_active(&mut self) {
        let active_repos: Vec<String> = self
            .github_prs
            .values()
            .map(|pr| pr.repo_slug.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        actions::fetch_github_prs::spawn(self.bg_tx.clone(), active_repos);
        self.last_ci_refresh = std::time::Instant::now();
    }

    pub fn prefetch_selected_pr_detail(&mut self) {
        let Some(issue) = self.selected_issue() else {
            return;
        };
        let issue_key = issue.key.clone();
        if self.sidebar.detail_loaded.contains(&issue_key)
            || self.sidebar.detail_loading.contains(&issue_key)
        {
            return;
        }
        let Some(pr) = self.github_prs.get(&issue_key) else {
            return;
        };

        self.sidebar.start_loading_detail(&issue_key);
        actions::fetch_github_pr_detail::spawn(
            self.bg_tx.clone(),
            issue_key,
            pr.repo_slug.clone(),
            pr.number,
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

    /// Spawn repo linking for issues that have no repo label match.
    ///
    /// Searches GitHub for open PRs across the org and labels issues whose
    /// PR branch matches an issue key and whose repo matches a local directory.
    fn spawn_link_jira_repos(&self) {
        let unlinked: Vec<_> = self
            .issues
            .iter()
            .filter(|issue| self.repo_matches(issue).is_empty())
            .map(|issue| (issue.key.clone(), issue.labels()))
            .collect();

        let repo_normalized_names: Vec<_> = self
            .repo_entries
            .iter()
            .map(|entry| (entry.label.clone(), entry.normalized.clone()))
            .collect();

        let github_org = self
            .repo_entries
            .iter()
            .filter_map(|entry| {
                entry
                    .github_slug
                    .as_deref()
                    .and_then(|slug| slug.split('/').next())
                    .map(|org| org.to_string())
            })
            .next()
            .unwrap_or_default();

        actions::link_jira_repos::spawn(
            self.bg_tx.clone(),
            self.client.clone(),
            unlinked,
            repo_normalized_names,
            github_org,
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
    pub fn handle_bg_msg(&mut self, msg: ActionMessage) {
        self.list_view.handle_action_message(&msg);
        self.status_bar
            .handle_action_message(&msg, &self.running_tasks);
        self.sidebar
            .handle_action_message(&msg, &mut self.github_prs);
        self.ci_log_popup
            .handle_action_message(&msg, &mut self.github_prs);

        match msg {
            ActionMessage::CurrentBranch(branch) => {
                self.current_branch = branch;
            }
            ActionMessage::Myself(result) => {
                if let Ok(account_id) = result {
                    self.my_account_id = account_id;
                }
            }
            ActionMessage::Issues(result) => self.handle_issues_message(result),
            ActionMessage::GithubPrs(all_prs, _) => {
                self.handle_github_prs_message(all_prs);
            }
            ActionMessage::GithubPrDetail(_, _) => {}
            ActionMessage::ActiveBranches(active) => {
                self.active_branches = active;
            }
            ActionMessage::PickedUp(result) => {
                if let Ok(pickup) = result {
                    self.current_branch = pickup.branch;
                    self.spawn_active_branches();
                }
            }
            ActionMessage::BranchDiffOpened(_) => {}
            ActionMessage::ApproveAutoMerged(_) => {}
            ActionMessage::Finished(result) => {
                if result.is_ok() {
                    self.spawn_refresh();
                }
            }
            ActionMessage::InlineCreated(result) => self.handle_inline_created_message(result),
            ActionMessage::AutoLabeled(_key, _result) => {}
            ActionMessage::LabelAdded(result) => {
                if result.is_ok() {
                    self.spawn_refresh();
                }
            }
            ActionMessage::ChildrenLoaded(parent_key, result) => {
                self.handle_children_loaded_message(parent_key, result);
            }
            ActionMessage::ConvertedToStory(_, result) => {
                if result.is_ok() {
                    self.spawn_refresh();
                }
            }
            ActionMessage::CiLogsFetched(_, _) => {}
            ActionMessage::FixCiOpened(result) => {
                if let Ok(branch) = result {
                    self.current_branch = branch;
                    self.spawn_active_branches();
                }
            }
            ActionMessage::OpenspecProposeOpened(_) => {}
            ActionMessage::TaskStarted(name) => {
                self.running_tasks.insert(name);
                self.status_bar.handle_task_started(&self.running_tasks);
            }
            ActionMessage::TaskFinished(name) => {
                self.running_tasks.remove(&name);
                self.status_bar
                    .handle_task_finished(name, &self.running_tasks);
            }
            ActionMessage::Progress(_) => {}
        }
    }

    fn handle_issues_message(&mut self, result: Result<Vec<Issue>>) {
        match result {
            Ok(issues) => {
                let selected_key = self.selected_issue().map(|issue| issue.key.clone());
                self.issues = issues;
                self.story_children.clear();
                self.rebuild_display_rows();
                let next_index = selected_key
                    .and_then(|key| {
                        self.display_rows.iter().position(|row| {
                            self.issue_for_display_row(row)
                                .map(|issue| issue.key == key)
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(0);
                self.selected_index = if self.display_rows.is_empty() {
                    0
                } else {
                    next_index.min(self.display_rows.len() - 1)
                };
                self.loading = false;
                self.spawn_active_branches();
                self.spawn_github_prs();
                self.spawn_link_jira_repos();
            }
            Err(_) => {
                self.loading = false;
            }
        }
    }

    fn handle_github_prs_message(&mut self, all_prs: Vec<PrInfo>) {
        let previous_prs = self.sidebar.begin_pr_refresh(&self.github_prs);

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

        self.sidebar
            .handle_pr_refresh(&mut self.github_prs, &previous_prs);
        self.record_check_durations();
        self.save_cache();
        self.spawn_auto_label();
        self.github_loading = false;
        self.refresh_latest_activity();
        self.prefetch_selected_pr_detail();
    }

    fn handle_inline_created_message(&mut self, result: Result<String>) {
        match result {
            Ok(key) => {
                self.input_mode = InputMode::Normal;
                let found_index = self.display_rows.iter().position(|row| {
                    self.issue_for_display_row(row)
                        .map(|issue| issue.key == key)
                        .unwrap_or(false)
                });
                if let Some(index) = found_index {
                    self.selected_index = index;
                }
                self.status_bar
                    .handle_inline_created(&key, found_index.is_some());
            }
            Err(err) => {
                self.status_bar.message = format!("Failed: {err}");
                self.input_mode = InputMode::Normal;
                self.cancel_inline_new();
            }
        }
    }

    fn handle_children_loaded_message(&mut self, parent_key: String, result: Result<Vec<Issue>>) {
        let Ok(children) = result else {
            return;
        };

        for child in &children {
            if is_expandable_type(child) {
                self.collapsed_stories.insert(child.key.clone());
            }
        }
        self.story_children.insert(parent_key, children);
        self.rebuild_display_rows();
    }

    /// Spawn pick-up issue in background.
    pub fn spawn_pick_up(&mut self) {
        let Some(issue) = self.selected_issue() else {
            return;
        };
        let issue_key = issue.key.clone();
        let issue_summary = issue.summary().unwrap_or_default();
        let issue_description = issue.description().unwrap_or_default();
        let repo_path = match self.repo_matches(issue).first() {
            Some(entry) => entry.path.clone(),
            None => {
                self.status_bar.message = format!("Cannot pick up {issue_key}: no linked repo");
                return;
            }
        };

        self.status_bar.message = "Picking up...".to_string();
        actions::pick_up::spawn(
            self.bg_tx.clone(),
            self.client.clone(),
            issue_key,
            issue_summary,
            issue_description,
            repo_path,
            self.my_account_id.clone(),
        );
    }

    /// Spawn branch diff in background: checkout branch and open difftool in tmux.
    pub fn spawn_branch_diff(&mut self) {
        let Some(issue) = self.selected_issue() else {
            return;
        };
        let issue_key = issue.key.clone();
        let repo_path = match self.repo_matches(issue).first() {
            Some(entry) => entry.path.clone(),
            None => {
                self.status_bar.message = format!("Cannot open diff for {issue_key}: no linked repo");
                return;
            }
        };

        self.status_bar.message = "Opening diff...".to_string();
        actions::branch_diff::spawn(self.bg_tx.clone(), issue_key, repo_path);
    }

    /// Spawn approve + auto-merge for the selected issue's PR.
    pub fn spawn_approve_merge(&mut self) {
        let Some(issue) = self.selected_issue() else {
            return;
        };
        let issue_key = issue.key.clone();
        let Some(pr) = self.github_prs.get(&issue_key) else {
            self.status_bar.message = format!("No PR found for {issue_key}");
            return;
        };

        self.status_bar.message = "Approving & enabling auto-merge...".to_string();
        actions::approve_merge::spawn(self.bg_tx.clone(), pr.repo_slug.clone(), pr.number);
    }

    /// Spawn finish workflow in background.
    pub fn spawn_finish(&mut self) {
        let Some(issue) = self.selected_issue() else {
            return;
        };
        let issue_key = issue.key.clone();
        let issue_summary = issue.summary().unwrap_or_default();
        let repo_path = match self.repo_matches(issue).first() {
            Some(entry) => entry.path.clone(),
            None => {
                self.status_bar.message = format!("Cannot finish {issue_key}: no linked repo");
                return;
            }
        };

        self.status_bar.message = "Finishing...".to_string();
        actions::finish::spawn(
            self.bg_tx.clone(),
            self.client.clone(),
            issue_key,
            issue_summary,
            repo_path,
        );
    }

    /// Spawn issue type toggle: Task → Story, or Story → Task if it has no children.
    pub fn spawn_toggle_story_type(&mut self) {
        let Some(issue) = self.selected_issue() else {
            return;
        };
        let issue_type_name = issue
            .issue_type()
            .map(|t| t.name.to_lowercase())
            .unwrap_or_default();
        let issue_key = issue.key.clone();

        if issue_type_name.contains("story") || issue_type_name.contains("epic") {
            let has_children = self
                .story_children
                .get(&issue_key)
                .map(|children| !children.is_empty())
                .unwrap_or(false);
            if has_children {
                self.status_bar.message = format!("{issue_key} has children — remove them first");
                return;
            }
            self.status_bar.message = "Reverting to task...".to_string();
            actions::convert_to_story::spawn(
                self.bg_tx.clone(),
                self.client.clone(),
                issue_key,
                "Task",
            );
            return;
        }

        self.status_bar.message = "Converting to story...".to_string();
        actions::convert_to_story::spawn(
            self.bg_tx.clone(),
            self.client.clone(),
            issue_key,
            "Story",
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
            self.status_bar.message = "Summary cannot be empty".into();
            return;
        }

        actions::create_inline_issue::spawn(
            self.bg_tx.clone(),
            self.client.clone(),
            self.jql.clone(),
            state.project_key,
            summary,
            state.parent_key,
        );
    }

    /// Returns the issue for the currently selected display row, if any.
    pub fn selected_issue(&self) -> Option<&Issue> {
        self.issue_for_row(self.selected_index)
    }

    /// Returns the issue for a given display row index, if any.
    pub fn issue_for_row(&self, row_index: usize) -> Option<&Issue> {
        let row = self.display_rows.get(row_index)?;
        self.issue_for_display_row(row)
    }

    /// Returns the issue for a given display row, if any.
    fn issue_for_display_row(&self, row: &DisplayRow) -> Option<&Issue> {
        match row {
            DisplayRow::Issue {
                index,
                child_of: None,
                ..
            } => self.issues.get(*index),
            DisplayRow::Issue {
                index,
                child_of: Some(parent_key),
                ..
            } => self.story_children.get(parent_key)?.get(*index),
            DisplayRow::StoryHeader { key, .. } => self.find_issue_by_key(key),
            DisplayRow::InlineNew { .. }
            | DisplayRow::Loading { .. }
            | DisplayRow::Empty { .. } => None,
        }
    }

    /// Look up an issue by key across all issue sources.
    fn find_issue_by_key(&self, key: &str) -> Option<&Issue> {
        self.issues
            .iter()
            .find(|issue| issue.key == key)
            .or_else(|| {
                self.story_children
                    .values()
                    .flat_map(|children| children.iter())
                    .find(|issue| issue.key == key)
            })
    }

    /// Returns the story key and depth if the current selection is inside a
    /// story group. Walks backwards from `selected_index` to find the nearest
    /// enclosing StoryHeader.
    fn enclosing_story_key_and_depth(&self) -> Option<(String, u8)> {
        let current_depth = match &self.display_rows.get(self.selected_index)? {
            DisplayRow::Issue { depth, .. }
            | DisplayRow::InlineNew { depth }
            | DisplayRow::Loading { depth }
            | DisplayRow::Empty { depth } => *depth,
            DisplayRow::StoryHeader { .. } => return None,
        };
        if current_depth == 0 {
            return None;
        }
        for i in (0..self.selected_index).rev() {
            match &self.display_rows[i] {
                DisplayRow::StoryHeader { key, depth, .. } if *depth < current_depth => {
                    return Some((key.clone(), *depth));
                }
                DisplayRow::Issue { depth: 0, .. } => return None,
                _ => continue,
            }
        }
        None
    }

    /// Returns the story key and its depth for inline creation: if the
    /// selection is on a `StoryHeader`, returns that header's key/depth;
    /// otherwise walks backward to find the nearest enclosing story.
    fn selected_story_or_enclosing(&self) -> Option<(String, u8)> {
        if let Some(DisplayRow::StoryHeader { key, depth, .. }) =
            self.display_rows.get(self.selected_index)
        {
            return Some((key.clone(), *depth));
        }
        self.enclosing_story_key_and_depth()
    }

    /// Toggle collapse state for the story at the current selection.
    /// Returns true if a toggle happened (i.e. selection was on a StoryHeader).
    pub fn toggle_story_collapse(&mut self) -> bool {
        let key = match self.display_rows.get(self.selected_index) {
            Some(DisplayRow::StoryHeader { key, .. }) => key.clone(),
            _ => return false,
        };
        if self.collapsed_stories.remove(&key) {
            // Was collapsed → now expanding. Fetch children if needed.
            self.spawn_fetch_children_for_story(&key);
        } else {
            // Was expanded → now collapsing.
            self.collapsed_stories.insert(key);
        }
        self.rebuild_display_rows();
        true
    }

    /// Expand the story at the current selection (no-op if already expanded or not a story).
    pub fn expand_story(&mut self) -> bool {
        let key = match self.display_rows.get(self.selected_index) {
            Some(DisplayRow::StoryHeader { key, .. }) => key.clone(),
            _ => return false,
        };
        if self.collapsed_stories.remove(&key) {
            self.spawn_fetch_children_for_story(&key);
            self.rebuild_display_rows();
            return true;
        }
        false
    }

    /// Collapse the story at the current selection (no-op if already collapsed or not a story).
    pub fn collapse_story(&mut self) -> bool {
        let key = match self.display_rows.get(self.selected_index) {
            Some(DisplayRow::StoryHeader { key, .. }) => key.clone(),
            _ => return false,
        };
        if !self.collapsed_stories.contains(&key) {
            self.collapsed_stories.insert(key);
            self.rebuild_display_rows();
            return true;
        }
        false
    }

    /// Fetch children for a story's child issues that might themselves be parents.
    /// This enables multi-level expansion.
    fn spawn_fetch_children_for_story(&mut self, parent_key: &str) {
        // Find child issues under this story that have an issue type suggesting
        // they could be stories (Story, Epic, Task with children, etc.)
        // We fetch children for ALL child issues of this story to discover sub-stories.
        if self.list_view.loading_children.contains(parent_key)
            || self.story_children.contains_key(parent_key)
        {
            return;
        }
        self.list_view.start_loading_children(parent_key);
        actions::fetch_children::spawn(
            self.bg_tx.clone(),
            self.client.clone(),
            parent_key.to_string(),
        );
    }

    /// Build the flattened display rows from the current issues list.
    /// Groups subtasks under their parent story headers, sorted by key.
    /// Supports multi-level nesting via `story_children`.
    pub fn rebuild_display_rows(&mut self) {
        use std::collections::HashMap as StdMap;

        // Apply search filter: collect indices of issues matching the query
        let matching_indices: Option<HashSet<usize>> = if self.search_filter.is_empty() {
            None
        } else {
            let query = self.search_filter.to_lowercase();
            Some(
                self.issues
                    .iter()
                    .enumerate()
                    .filter(|(_, issue)| {
                        issue.key.to_lowercase().contains(&query)
                            || issue
                                .summary()
                                .unwrap_or_default()
                                .to_lowercase()
                                .contains(&query)
                            || issue
                                .assignee()
                                .map(|u| u.display_name.to_lowercase().contains(&query))
                                .unwrap_or(false)
                            || issue
                                .status()
                                .map(|s| s.name.to_lowercase().contains(&query))
                                .unwrap_or(false)
                    })
                    .map(|(idx, _)| idx)
                    .collect(),
            )
        };

        // Collect parent keys for issues that have a parent
        // parent_key -> (summary, Vec<issue_index>)
        let mut parent_groups: StdMap<String, (String, Vec<usize>)> = StdMap::new();
        let mut standalone_indices: Vec<usize> = Vec::new();

        for (idx, issue) in self.issues.iter().enumerate() {
            if let Some(ref matching) = matching_indices {
                if !matching.contains(&idx) {
                    continue;
                }
            }
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

        // Sort children within each parent group by status, then creation date descending
        for (_, children) in parent_groups.values_mut() {
            children.sort_by(|a, b| {
                status_rank(&self.issues[*a])
                    .cmp(&status_rank(&self.issues[*b]))
                    .then_with(|| {
                        issue_created_str(&self.issues[*b])
                            .cmp(&issue_created_str(&self.issues[*a]))
                    })
            });
        }

        // Track which standalone issues are themselves parents
        let parent_key_set: HashSet<String> = parent_groups.keys().cloned().collect();

        // Build top-level entries: each is either a standalone issue or a story group.
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

        // Sort all top-level entries by status, then creation date descending
        top_levels.sort_by(|a, b| {
            let rank_a = top_level_status_rank(a, &self.issues);
            let rank_b = top_level_status_rank(b, &self.issues);
            rank_a.cmp(&rank_b).then_with(|| {
                top_level_created(b, &self.issues).cmp(&top_level_created(a, &self.issues))
            })
        });

        fn top_level_created(entry: &TopLevel, issues: &[Issue]) -> String {
            match entry {
                TopLevel::Standalone(idx) => issue_created_str(&issues[*idx]),
                TopLevel::StoryGroup {
                    parent_issue_idx,
                    children,
                    ..
                } => {
                    let parent_created =
                        parent_issue_idx.map(|idx| issue_created_str(&issues[idx]));
                    let child_max = children
                        .iter()
                        .map(|idx| issue_created_str(&issues[*idx]))
                        .max();
                    parent_created
                        .into_iter()
                        .chain(child_max)
                        .max()
                        .unwrap_or_default()
                }
            }
        }

        fn issue_created_str(issue: &Issue) -> String {
            issue
                .field::<String>("created")
                .and_then(|r| r.ok())
                .unwrap_or_default()
        }

        fn top_level_status_rank(entry: &TopLevel, issues: &[Issue]) -> u8 {
            match entry {
                TopLevel::Standalone(idx) => status_rank(&issues[*idx]),
                TopLevel::StoryGroup {
                    parent_issue_idx,
                    children,
                    ..
                } => {
                    let child_min = children
                        .iter()
                        .map(|idx| status_rank(&issues[*idx]))
                        .min()
                        .unwrap_or(u8::MAX);
                    let parent_rank = parent_issue_idx
                        .map(|idx| status_rank(&issues[idx]))
                        .unwrap_or(u8::MAX);
                    child_min.min(parent_rank)
                }
            }
        }

        // Flatten into display rows
        let mut rows = Vec::new();
        for entry in top_levels {
            match entry {
                TopLevel::Standalone(idx) => {
                    let issue = &self.issues[idx];
                    let issue_key = issue.key.clone();
                    let expandable =
                        is_expandable_type(issue) || self.story_children.contains_key(&issue_key);
                    if expandable {
                        // Default to collapsed until explicitly expanded
                        if !self.story_children.contains_key(&issue_key)
                            && !self.list_view.loading_children.contains(&issue_key)
                        {
                            self.collapsed_stories.insert(issue_key.clone());
                        }
                        let summary = issue.summary().unwrap_or_default();
                        rows.push(DisplayRow::StoryHeader {
                            key: issue_key.clone(),
                            summary,
                            depth: 0,
                        });
                        if !self.collapsed_stories.contains(&issue_key) {
                            self.append_nested_children(&issue_key, 1, &mut rows);
                        }
                    } else {
                        rows.push(DisplayRow::Issue {
                            index: idx,
                            depth: 0,
                            child_of: None,
                        });
                    }
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
                        depth: 0,
                    });
                    if !self.collapsed_stories.contains(&key) {
                        // Skip the parent issue row — it duplicates the story header
                        if let Some(idx) = parent_issue_idx {
                            let issue_key = &self.issues[idx].key;
                            if *issue_key != key {
                                rows.push(DisplayRow::Issue {
                                    index: idx,
                                    depth: 1,
                                    child_of: None,
                                });
                            }
                        }
                        for child_idx in children {
                            let child_issue = &self.issues[child_idx];
                            let child_key = child_issue.key.clone();
                            let expandable = is_expandable_type(child_issue)
                                || self.story_children.contains_key(&child_key);
                            if expandable {
                                // Default nested stories to collapsed
                                if !self.story_children.contains_key(&child_key)
                                    && !self.list_view.loading_children.contains(&child_key)
                                {
                                    self.collapsed_stories.insert(child_key.clone());
                                }
                                let child_summary = child_issue.summary().unwrap_or_default();
                                rows.push(DisplayRow::StoryHeader {
                                    key: child_key.clone(),
                                    summary: child_summary,
                                    depth: 1,
                                });
                                if !self.collapsed_stories.contains(&child_key) {
                                    self.append_nested_children(&child_key, 2, &mut rows);
                                }
                            } else {
                                rows.push(DisplayRow::Issue {
                                    index: child_idx,
                                    depth: 1,
                                    child_of: None,
                                });
                            }
                        }
                    }
                }
            }
        }

        self.display_rows = rows;
        if !self.display_rows.is_empty() && self.selected_index >= self.display_rows.len() {
            self.selected_index = self.display_rows.len() - 1;
        }
    }

    /// Append children for a nested story header, handling loading/empty states.
    fn append_nested_children(&self, parent_key: &str, depth: u8, rows: &mut Vec<DisplayRow>) {
        if self.list_view.loading_children.contains(parent_key) {
            rows.push(DisplayRow::Loading { depth });
            return;
        }
        let Some(children) = self.story_children.get(parent_key) else {
            // Not loading and no children stored — show loading since fetch
            // will be triggered by toggle_story_collapse.
            rows.push(DisplayRow::Loading { depth });
            return;
        };
        if children.is_empty() {
            rows.push(DisplayRow::Empty { depth });
            return;
        }
        for (idx, child) in children.iter().enumerate() {
            let child_key = child.key.clone();
            let expandable =
                is_expandable_type(child) || self.story_children.contains_key(&child_key);
            if expandable {
                let child_summary = child.summary().unwrap_or_default();
                rows.push(DisplayRow::StoryHeader {
                    key: child_key.clone(),
                    summary: child_summary,
                    depth,
                });
                if !self.collapsed_stories.contains(&child_key) {
                    self.append_nested_children(&child_key, depth + 1, rows);
                }
            } else {
                rows.push(DisplayRow::Issue {
                    index: idx,
                    depth,
                    child_of: Some(parent_key.to_string()),
                });
            }
        }
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

    /// Start an inline new-issue row. If inside a story group (or on a story
    /// header), it becomes a subtask of that story. Otherwise creates a
    /// top-level issue.
    pub fn start_inline_new(&mut self) -> bool {
        let story_key = self.selected_story_or_enclosing();
        let project_key = self.derive_project_key();

        let (insert_at, depth, parent_key) = if let Some((parent, story_depth)) = story_key {
            let child_depth = story_depth + 1;
            let group_end = self.find_story_group_end(self.selected_index);
            // If the last row in the group is an Empty placeholder, replace it
            // so the inline editor appears nested inside the story group.
            let replace_empty = matches!(
                self.display_rows.get(group_end),
                Some(DisplayRow::Empty { .. })
            );
            if replace_empty {
                (group_end, child_depth, Some(parent))
            } else {
                (group_end + 1, child_depth, Some(parent))
            }
        } else {
            let at = self.selected_index + 1;
            (at, 0u8, None)
        };

        if matches!(
            self.display_rows.get(insert_at),
            Some(DisplayRow::Empty { .. })
        ) {
            self.display_rows[insert_at] = DisplayRow::InlineNew { depth };
        } else {
            self.display_rows
                .insert(insert_at, DisplayRow::InlineNew { depth });
        }

        let state = InlineNewState {
            summary: String::new(),
            parent_key,
            project_key,
            row_index: insert_at,
        };
        self.inline_new = Some(state);
        self.selected_index = insert_at;
        self.input_mode = InputMode::Editing;
        true
    }

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
        let base_depth = match &self.display_rows[from] {
            DisplayRow::StoryHeader { depth, .. }
            | DisplayRow::Issue { depth, .. }
            | DisplayRow::InlineNew { depth }
            | DisplayRow::Loading { depth }
            | DisplayRow::Empty { depth } => *depth,
        };
        let mut end = from;
        for i in (from + 1)..self.display_rows.len() {
            let row_depth = match &self.display_rows[i] {
                DisplayRow::StoryHeader { depth, .. }
                | DisplayRow::Issue { depth, .. }
                | DisplayRow::InlineNew { depth }
                | DisplayRow::Loading { depth }
                | DisplayRow::Empty { depth } => *depth,
            };
            if row_depth > base_depth {
                end = i;
            } else {
                break;
            }
        }
        end
    }

    /// Remove the InlineNew row at the given index and fix up selection.
    /// If the inline row was the only child of a story group, restores the
    /// `Empty` placeholder so the group doesn't visually collapse.
    fn remove_inline_row(&mut self, row_index: usize) {
        if row_index < self.display_rows.len() {
            if let DisplayRow::InlineNew { depth } = self.display_rows[row_index] {
                let is_only_child_of_story = depth > 0
                    && row_index > 0
                    && matches!(
                        self.display_rows.get(row_index - 1),
                        Some(DisplayRow::StoryHeader { .. })
                    )
                    && !matches!(
                        self.display_rows.get(row_index + 1),
                        Some(
                            DisplayRow::Issue { depth: d, .. }
                            | DisplayRow::Loading { depth: d }
                            | DisplayRow::Empty { depth: d }
                        ) if *d > 0
                    );
                if is_only_child_of_story {
                    self.display_rows[row_index] = DisplayRow::Empty { depth };
                } else {
                    self.display_rows.remove(row_index);
                }
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
                self.status_bar.message = message;
            }
        }
    }

    pub fn open_label_picker(&mut self) {
        self.reload_repo_entries();
        if self.repo_entries.is_empty() {
            if self.repo_error.is_none() {
                self.status_bar.message = "No repositories found in REPOS_DIR".to_string();
            }
            return;
        }
        self.list_view.open_label_picker();
    }

    pub fn close_label_picker(&mut self) {
        self.list_view.close_label_picker();
    }

    pub fn move_label_picker_selection(&mut self, down: bool) {
        self.list_view
            .move_label_picker_selection(&self.repo_entries, down);
    }

    pub fn label_picker_entry(&self) -> Option<&RepoEntry> {
        self.list_view.label_picker_entry(&self.repo_entries)
    }

    pub fn label_picker_type_char(&mut self, ch: char) {
        self.list_view.label_picker_type_char(ch);
    }

    pub fn label_picker_backspace(&mut self) {
        self.list_view.label_picker_backspace();
    }

    pub fn open_ci_log_popup(&mut self) {
        let Some(issue) = self.selected_issue() else {
            self.status_bar.message = "No issue selected".to_string();
            return;
        };
        let issue_key = issue.key.clone();
        let Some(pr) = self.github_prs.get(&issue_key) else {
            self.status_bar.message = "No linked PR".to_string();
            return;
        };
        if pr.check_runs.is_empty() {
            self.status_bar.message = "No CI checks".to_string();
            return;
        }
        self.ci_log_popup.open();
        self.spawn_ci_log_fetch(&issue_key);
    }

    /// Spawn CI log fetch if logs aren't already cached or in-flight.
    fn spawn_ci_log_fetch(&mut self, issue_key: &str) {
        let Some(pr) = self.github_prs.get(issue_key) else {
            return;
        };
        if !self.ci_log_popup.start_loading(issue_key) {
            return;
        }
        actions::fetch_ci_logs::spawn(
            self.bg_tx.clone(),
            issue_key.to_string(),
            pr.repo_slug.clone(),
            pr.check_runs.clone(),
        );
    }

    /// Checkout the PR branch and open opencode with the CI error as prompt.
    pub fn spawn_fix_ci(&mut self) {
        let Some(issue) = self.selected_issue() else {
            self.status_bar.message = "No issue selected".to_string();
            return;
        };
        let issue_key = issue.key.clone();
        let repo_path = match self.repo_matches(issue).first() {
            Some(entry) => entry.path.clone(),
            None => {
                self.status_bar.message = format!("Cannot fix CI for {issue_key}: no linked repo");
                return;
            }
        };
        let Some(pr) = self.github_prs.get(&issue_key) else {
            self.status_bar.message = "No linked PR".to_string();
            return;
        };

        let head_branch = pr.head_branch.clone();
        let ci_error: String = pr
            .check_runs
            .iter()
            .filter(|r| r.status == CheckStatus::Fail)
            .map(|r| {
                if !r.log_excerpt.trim().is_empty() {
                    format!("## {}\n{}", r.name, r.log_excerpt.trim())
                } else {
                    format!("## {} (no logs available)", r.name)
                }
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        self.close_ci_log_popup();
        self.status_bar.message = "Checking out branch and opening opencode...".to_string();
        actions::fix_ci::spawn(self.bg_tx.clone(), repo_path, head_branch, ci_error);
    }

    /// Open an opencode session to propose an openspec change for the selected issue.
    pub fn spawn_openspec_propose(&mut self) {
        let Some(issue) = self.selected_issue() else {
            return;
        };
        let issue_key = issue.key.clone();
        let issue_summary = issue.summary().unwrap_or_default();
        let issue_description = issue.description().unwrap_or_default();

        let repo_slugs: Vec<String> = self
            .repo_matches(issue)
            .iter()
            .filter_map(|entry| entry.github_slug.clone())
            .collect();

        let mut parent_stories = Vec::new();
        let mut current_issue = self.selected_issue().cloned();
        while let Some(parent) = current_issue.as_ref().and_then(|i| i.parent()) {
            parent_stories.push(actions::openspec_propose::StoryContext {
                summary: parent.summary().unwrap_or_default(),
                description: parent.description().unwrap_or_default(),
            });
            current_issue = Some(parent);
        }

        let repos_dir = match std::env::var("REPOS_DIR") {
            Ok(dir) => std::path::PathBuf::from(dir),
            Err(_) => {
                self.status_bar.message = "REPOS_DIR is not set".to_string();
                return;
            }
        };

        self.status_bar.message = "Opening openspec propose...".to_string();
        actions::openspec_propose::spawn(
            self.bg_tx.clone(),
            repos_dir,
            issue_key,
            issue_summary,
            issue_description,
            parent_stories,
            repo_slugs,
        );
    }

    pub fn close_ci_log_popup(&mut self) {
        self.ci_log_popup.close();
    }

    pub fn scroll_ci_log_popup(&mut self, delta: isize) {
        self.ci_log_popup.scroll_by(delta);
    }

    pub fn cycle_ci_log_tab(&mut self, delta: isize) {
        let Some(issue) = self.selected_issue() else {
            return;
        };
        let issue_key = issue.key.clone();
        let Some(pr) = self.github_prs.get(&issue_key) else {
            return;
        };
        let check_run_count = pr.check_runs.len();
        if check_run_count == 0 {
            return;
        }
        self.ci_log_popup.cycle_tab(delta, check_run_count);
    }

    pub fn start_search(&mut self) {
        self.input_mode = InputMode::Searching;
    }

    pub fn confirm_search(&mut self) {
        self.input_mode = InputMode::Normal;
    }

    pub fn cancel_search(&mut self) {
        self.search_filter.clear();
        self.input_mode = InputMode::Normal;
        self.rebuild_display_rows();
    }

    pub fn search_type_char(&mut self, ch: char) {
        self.search_filter.push(ch);
        self.selected_index = 0;
        self.rebuild_display_rows();
    }

    pub fn search_backspace(&mut self) {
        self.search_filter.pop();
        self.selected_index = 0;
        self.rebuild_display_rows();
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
        self.animation.tick_spinner();
    }

    /// Returns `true` when any background work is in progress.
    pub fn is_busy(&self) -> bool {
        self.loading
            || self.github_loading
            || !self.running_tasks.is_empty()
            || self.status_bar.message.starts_with('[')
    }

    /// Returns `true` when any tracked PR has pending CI checks.
    pub fn has_pending_checks(&self) -> bool {
        self.github_prs
            .values()
            .any(|pr| pr.checks == CheckStatus::Pending)
    }

    /// Build a Cache from current app state and save to disk.
    fn save_cache(&self) {
        cache::save(&Cache {
            check_durations: self.check_durations.clone(),
        });
    }

    /// Record durations of completed check runs into the history cache.
    fn record_check_durations(&mut self) {
        for pr in self.github_prs.values() {
            for run in &pr.check_runs {
                let (Some(started), Some(completed)) = (&run.started_at, &run.completed_at) else {
                    continue;
                };
                let Some(duration) = parse_duration_secs(started, completed) else {
                    continue;
                };
                let cache_key = format!("{}/{}", pr.repo_slug, run.name);
                self.check_durations.insert(cache_key, duration);
            }
        }
    }

    /// Compute the ETA string for a PR's pending checks.
    /// Returns e.g. "~3m" based on historical durations minus elapsed time.
    pub fn pr_eta(&self, pr: &PrInfo) -> Option<String> {
        let pending_runs: Vec<_> = pr
            .check_runs
            .iter()
            .filter(|r| r.status == CheckStatus::Pending)
            .collect();
        if pending_runs.is_empty() {
            return None;
        }
        // Find the maximum remaining time across all pending checks
        let mut max_remaining: Option<u64> = None;
        for run in &pending_runs {
            let cache_key = format!("{}/{}", pr.repo_slug, run.name);
            let Some(&historical) = self.check_durations.get(&cache_key) else {
                continue;
            };
            let elapsed = run
                .started_at
                .as_deref()
                .and_then(elapsed_since_iso)
                .unwrap_or(0);
            let remaining = historical.saturating_sub(elapsed);
            max_remaining = Some(max_remaining.map_or(remaining, |cur: u64| cur.max(remaining)));
        }
        max_remaining.map(|r| format!("~{}", format_duration(r)))
    }

    /// Compute a timing string for a single check run.
    /// - Completed: returns humanized completion time (e.g. "2m ago")
    /// - Pending: returns elapsed timer + optional ETA (e.g. "1m32s (~3m)")
    /// - No timing data: returns None
    pub fn check_run_timing(&self, pr: &PrInfo, run: &CheckRun) -> Option<String> {
        match run.status {
            CheckStatus::Pass | CheckStatus::Fail => run.completed_at.as_deref().map(|completed| {
                let elapsed =
                    parse_duration_secs(run.started_at.as_deref().unwrap_or(completed), completed);
                match elapsed {
                    Some(secs) => format_duration(secs),
                    None => "done".to_string(),
                }
            }),
            CheckStatus::Pending => {
                let elapsed = run.started_at.as_deref().and_then(elapsed_since_iso)?;
                let cache_key = format!("{}/{}", pr.repo_slug, run.name);
                let eta = self.check_durations.get(&cache_key).map(|&historical| {
                    let remaining = historical.saturating_sub(elapsed);
                    format!(" (~{})", format_duration(remaining))
                });
                Some(format!(
                    "{}{}",
                    format_duration(elapsed),
                    eta.unwrap_or_default()
                ))
            }
        }
    }

    /// Compute a timing string for a single check step.
    pub fn check_step_timing(
        &self,
        pr: &PrInfo,
        run: &CheckRun,
        step: &CheckStep,
    ) -> Option<String> {
        match step.status {
            CheckStatus::Pass | CheckStatus::Fail => {
                step.completed_at.as_deref().map(|completed| {
                    let elapsed = parse_duration_secs(
                        step.started_at.as_deref().unwrap_or(completed),
                        completed,
                    );
                    match elapsed {
                        Some(secs) => format_duration(secs),
                        None => "done".to_string(),
                    }
                })
            }
            CheckStatus::Pending => {
                let elapsed = step.started_at.as_deref().and_then(elapsed_since_iso)?;
                let cache_key = format!("{}/{}/{}", pr.repo_slug, run.name, step.name);
                let eta = self.check_durations.get(&cache_key).map(|&historical| {
                    let remaining = historical.saturating_sub(elapsed);
                    format!(" (~{})", format_duration(remaining))
                });
                Some(format!(
                    "{}{}",
                    format_duration(elapsed),
                    eta.unwrap_or_default()
                ))
            }
        }
    }

    /// Tick down completed-task display timers; called from the main loop tick.
    pub fn tick_completed_tasks(&mut self) {
        self.status_bar.tick_completed_tasks();
    }

    pub fn add_label_from_picker(&mut self) -> bool {
        let Some(entry) = self.label_picker_entry().cloned() else {
            self.status_bar.message = "No repository selected".to_string();
            return false;
        };
        let Some(issue) = self.selected_issue() else {
            self.status_bar.message = "No issue selected".to_string();
            return false;
        };
        let issue_key = issue.key.clone();
        let labels = issue.labels();
        let target_normalized = repos::normalize_label(&entry.label);
        let already_has = labels
            .iter()
            .any(|l| repos::normalize_label(l) == target_normalized);
        if already_has {
            self.status_bar.message = format!("{issue_key} already labeled with {}", entry.label);
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
        self.status_bar.message = format!("Opened {} in browser", url);
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
        self.status_bar.message = format!("Opened PR #{number} in browser");
        Ok(())
    }
}

/// Returns true if the issue type suggests this issue can contain child issues
/// (i.e. it's a story or epic that can be expanded as a nested story header).
fn is_expandable_type(issue: &Issue) -> bool {
    let type_name = issue
        .issue_type()
        .map(|ty| ty.name)
        .unwrap_or_default()
        .to_lowercase();
    type_name.contains("story") || type_name.contains("epic")
}

/// Numeric rank for sorting issues by status. Lower = higher in the list.
fn status_rank(issue: &Issue) -> u8 {
    const ORDER: &[&str] = &["review", "progress", "rejected", "plan", "proposed"];
    let name = issue
        .status()
        .map(|s| s.name)
        .unwrap_or_default()
        .to_lowercase();
    ORDER
        .iter()
        .position(|&keyword| name.contains(keyword))
        .map(|i| i as u8)
        .unwrap_or(ORDER.len() as u8)
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
