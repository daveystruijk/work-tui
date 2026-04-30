use std::collections::{HashMap, HashSet};

use color_eyre::Result;
use crossterm::event::KeyCode;
use tokio::sync::mpsc;

use crate::{
    actions::{self, Message},
    apis::{
        github::{CheckStatus, GithubStatus, PrInfo},
        jira::{Issue, JiraClient},
    },
    cache::{self, Cache},
    config::AppConfig,
    repos::{self, RepoEntry},
    ui::{
        CiLogsView, ImportTasksView, LabelPickerView, ListView, SidebarView, StatusBarView,
        UiAnimationView,
    },
    utils::time::parse_duration_secs,
};

#[derive(Debug, Clone)]
pub struct RunningAction {
    pub id: String,
    pub label: String,
    pub progress: Option<actions::Progress>,
}

/// A row in the display list — either a story header, an issue, or an inline-new placeholder.
#[derive(Debug, Clone)]
pub enum DisplayRow {
    /// Visual section divider (e.g. "BOARD", "BACKLOG").
    SectionHeader { label: String, count: usize },
    /// A parent story header (not necessarily in the fetched issues list).
    StoryHeader {
        key: String,
        summary: String,
        depth: u8,
        /// Which section this header belongs to (`Some(true)` = board, `Some(false)` = backlog, `None` = unsectioned).
        section: Option<bool>,
    },
    /// An actual issue row.
    Issue {
        index: usize,
        depth: u8,
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
pub struct InlineNewView {
    pub summary: String,
    pub parent_key: Option<String>,
    pub project_key: String,
    pub row_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputFocus {
    #[default]
    List,
    Search,
    InlineNew,
    ImportTasksPopup,
    CiLogPopup,
    LabelPicker,
}

pub struct AppView {
    pub should_quit: bool,
    pub input_focus: InputFocus,
    pub issues: Vec<Issue>,
    pub config: AppConfig,
    pub repo_entries: Vec<RepoEntry>,
    pub repo_error: Option<String>,
    pub list: ListView,
    pub label_picker: Option<LabelPickerView>,
    pub status_bar: StatusBarView,
    pub loading: bool,
    pub client: JiraClient,
    pub my_account_id: String,
    pub current_branch: String,
    /// Maps issue key -> repo label for issues whose branch is currently checked out
    pub active_branches: HashMap<String, String>,
    /// Maps issue key -> GitHub PR status
    pub github_statuses: HashMap<String, GithubStatus>,
    /// Whether GitHub statuses are currently being loaded
    pub github_loading: bool,
    pub animation: UiAnimationView,
    /// Dynamically loaded child issues for expanded stories, keyed by parent key.
    pub story_children: HashMap<String, Vec<Issue>>,
    pub sidebar: SidebarView,
    /// Maps issue key -> matched PR info from GitHub
    pub github_prs: HashMap<String, PrInfo>,
    /// Historical CI check durations in seconds, keyed by "repo_slug/check_name".
    pub check_durations: HashMap<String, u64>,
    /// Currently running background tasks
    pub running_tasks: Vec<RunningAction>,
    /// Sender for background tasks to deliver results
    pub message_tx: mpsc::UnboundedSender<Message>,
    /// Receiver polled in the event loop
    pub message_rx: mpsc::UnboundedReceiver<Message>,
    /// Last time a CI/PR refresh was triggered (for auto-refresh throttling)
    pub last_ci_refresh: std::time::Instant,
    pub ci_log_popup: CiLogsView,
    pub previous_key: Option<KeyCode>,
    pub import_tasks_popup: Option<ImportTasksView>,
    pub pending_selected_issue_key: Option<String>,
    /// When set, a prefetch of the selected PR detail is scheduled after a short delay.
    /// This avoids firing fetches while the user is scrolling quickly through the list.
    pub pending_prefetch_since: Option<std::time::Instant>,
}

impl AppView {
    pub fn new(config: AppConfig) -> Result<Self> {
        let client = JiraClient::new(&config.jira)?;
        let (message_tx, message_rx) = mpsc::unbounded_channel();
        let mut app = Self {
            should_quit: false,
            input_focus: InputFocus::default(),
            issues: Vec::new(),
            config,
            repo_entries: Vec::new(),
            repo_error: None,
            list: ListView::default(),
            label_picker: None,
            status_bar: StatusBarView::default(),
            loading: true,
            client,
            my_account_id: String::new(),
            current_branch: String::new(),
            active_branches: HashMap::new(),
            github_statuses: HashMap::new(),
            github_loading: false,
            animation: UiAnimationView::default(),
            story_children: HashMap::new(),
            sidebar: SidebarView::default(),
            github_prs: HashMap::new(),
            check_durations: HashMap::new(),
            running_tasks: Vec::new(),
            message_tx,
            message_rx,
            last_ci_refresh: std::time::Instant::now(),
            ci_log_popup: CiLogsView::default(),
            previous_key: None,
            import_tasks_popup: None,
            pending_selected_issue_key: None,
            pending_prefetch_since: None,
        };

        let cached = cache::load();
        app.check_durations = cached.check_durations;
        app.list.collapsed_stories = cached.collapsed_stories;

        app.reload_repo_entries();

        Ok(app)
    }

    /// Kick off all initialization work as background tasks.
    pub fn spawn_initialize(&self) {
        actions::initialize::spawn(
            self.message_tx.clone(),
            self.client.clone(),
            self.config.jira.jira_jql.clone(),
        );
    }

    /// Spawn a full refresh (issues + PRs + statuses).
    pub fn spawn_refresh(&mut self) {
        actions::refresh::spawn(
            self.message_tx.clone(),
            self.client.clone(),
            self.config.jira.jira_jql.clone(),
        );
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
        actions::fetch_github_prs::spawn(self.message_tx.clone(), self.matched_repo_slugs());
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
        actions::fetch_github_prs::spawn(self.message_tx.clone(), active_repos);
        self.last_ci_refresh = std::time::Instant::now();
    }

    /// Schedule a debounced prefetch of the selected PR detail.
    /// The actual fetch fires after the cursor has been idle for 150ms.
    pub fn schedule_prefetch(&mut self) {
        self.pending_prefetch_since = Some(std::time::Instant::now());
    }

    /// Called every event-loop tick. Fires the prefetch if the debounce delay has elapsed.
    pub fn tick_prefetch(&mut self) {
        const PREFETCH_DELAY: std::time::Duration = std::time::Duration::from_millis(150);
        let Some(since) = self.pending_prefetch_since else {
            return;
        };
        if since.elapsed() < PREFETCH_DELAY {
            return;
        }
        self.pending_prefetch_since = None;
        self.prefetch_selected_pr_detail();
    }

    pub fn prefetch_selected_pr_detail(&mut self) {
        let Some(issue) = self.list.selected_issue(&self.issues, &self.story_children) else {
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
            self.message_tx.clone(),
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

        actions::detect_active_branches::spawn(self.message_tx.clone(), issue_data);
    }

    /// Spawn repo tagging for issues that have no repo label match.
    fn spawn_tag_jira_repos(&self) {
        let untagged: Vec<_> = self
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

        actions::tag_jira_repos::spawn(
            self.message_tx.clone(),
            self.client.clone(),
            untagged,
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

        actions::auto_label::spawn(self.message_tx.clone(), self.client.clone(), to_label);
    }

    /// Process a background message. Called from the event loop.
    pub fn handle_message(&mut self, msg: Message) {
        self.list.handle_message(&msg);
        self.status_bar.handle_message(&msg);
        self.sidebar.handle_message(&msg, &mut self.github_prs);
        self.ci_log_popup.handle_message(&msg, &mut self.github_prs);

        match msg {
            Message::CurrentBranch(branch) => {
                self.current_branch = branch;
            }
            Message::Myself(result) => {
                if let Ok(account_id) = result {
                    self.my_account_id = account_id;
                }
            }
            Message::Issues(result) => self.handle_issues_message(result),
            Message::GithubPrs(all_prs, _) => {
                self.handle_github_prs_message(all_prs);
            }
            Message::GithubPrDetail(_, _) => {}
            Message::ActiveBranches(active) => {
                self.active_branches = active;
            }
            Message::PickedUp(result) => {
                if let Ok(pickup) = result {
                    if let Some(branch) = pickup.branch {
                        self.current_branch = branch;
                        self.spawn_active_branches();
                    }
                }
            }
            Message::BranchDiffOpened(_) => {}
            Message::ApproveAutoMerged(result) => {
                if result.is_ok() {
                    self.spawn_github_prs_active();
                }
            }
            Message::Finished(result) => {
                if result.is_ok() {
                    self.spawn_refresh();
                }
            }
            Message::InlineCreated(result) => self.handle_inline_created_message(result),
            Message::AutoLabeled(_key, _result) => {}
            Message::LabelAdded(result) => {
                if result.is_ok() {
                    self.spawn_refresh();
                }
            }
            Message::ChildrenLoaded(parent_key, result) => {
                self.handle_children_loaded_message(parent_key, result);
            }
            Message::ConvertedToStory(_, result) => {
                if result.is_ok() {
                    self.spawn_refresh();
                }
            }
            Message::CiLogsFetched(_, _) => {}
            Message::FixCiOpened(result) => {
                if let Ok(branch) = result {
                    self.current_branch = branch;
                    self.spawn_active_branches();
                }
            }
            Message::OpenspecProposeOpened(_) => {}
            Message::TasksImported(_issue_key, result) => match result {
                Ok(()) => {
                    self.spawn_refresh();
                    self.spawn_scan_import_tasks();
                }
                Err(err) => self.status_bar.set_error(format!("Import failed: {err}")),
            },
            Message::PendingImportKeys(_) => {}
            Message::ActionStarted { id, label } => self.start_running_action(id, label),
            Message::ActionFinished(id) => self.finish_running_action(&id),
            Message::Progress(progress) => self.update_running_action(progress),
        }
    }

    fn start_running_action(&mut self, id: String, label: String) {
        if self.running_tasks.iter().any(|task| task.id == id) {
            return;
        }
        self.running_tasks.push(RunningAction {
            id,
            label,
            progress: None,
        });
    }

    fn finish_running_action(&mut self, id: &str) {
        self.running_tasks.retain(|task| task.id != id);
    }

    fn update_running_action(&mut self, progress: actions::Progress) {
        if let Some(task) = self
            .running_tasks
            .iter_mut()
            .find(|task| task.id == progress.task_id)
        {
            task.progress = Some(progress);
        }
    }

    fn handle_issues_message(&mut self, result: Result<Vec<Issue>>) {
        match result {
            Ok(issues) => {
                let expanded_story_keys = self.expanded_loaded_story_keys();
                let selection_restore_keys = self.selected_issue_restore_keys();
                self.issues = issues;
                self.story_children.clear();
                self.restore_expanded_story_loading(&expanded_story_keys);
                self.list
                    .rebuild_display_rows(&self.issues, &self.story_children);
                let previous_scroll_offset = self.list.scroll_offset;
                let restored_exact_selection =
                    self.restore_selection_for_issue_keys(&selection_restore_keys);
                self.list.scroll_offset = previous_scroll_offset;
                self.list.adjust_scroll_offset();
                self.loading = false;
                self.pending_selected_issue_key = if restored_exact_selection {
                    None
                } else {
                    selection_restore_keys.first().cloned()
                };
                self.refetch_story_children(&expanded_story_keys);
                self.spawn_active_branches();
                self.spawn_github_prs();
                self.spawn_tag_jira_repos();
                self.spawn_scan_import_tasks();
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
        self.prefetch_selected_pr_detail();
    }

    fn handle_inline_created_message(&mut self, result: Result<String>) {
        match result {
            Ok(key) => {
                self.input_focus = InputFocus::List;
                let found_index = self.list.display_rows.iter().position(|row| {
                    self.list
                        .issue_for_display_row(row, &self.issues, &self.story_children)
                        .map(|issue| issue.key == key)
                        .unwrap_or(false)
                });
                if let Some(index) = found_index {
                    self.list.selected_index = index;
                }
            }
            Err(err) => {
                self.status_bar.set_error(format!("Failed: {err}"));
                self.input_focus = InputFocus::List;
                self.list.cancel_inline_new();
            }
        }
    }

    fn handle_children_loaded_message(&mut self, parent_key: String, result: Result<Vec<Issue>>) {
        let Ok(children) = result else {
            return;
        };

        for child in &children {
            if crate::issue::is_expandable(child) {
                self.list
                    .collapsed_stories
                    .insert((child.key.clone(), Some(true)));
                self.list
                    .collapsed_stories
                    .insert((child.key.clone(), Some(false)));
            }
        }
        self.story_children.insert(parent_key, children);
        self.list
            .rebuild_display_rows(&self.issues, &self.story_children);
        self.restore_pending_selection();
    }

    fn expanded_loaded_story_keys(&self) -> Vec<String> {
        let mut keys = self
            .story_children
            .keys()
            .filter(|key| {
                !self
                    .list
                    .collapsed_stories
                    .contains(&((*key).clone(), Some(true)))
                    || !self
                        .list
                        .collapsed_stories
                        .contains(&((*key).clone(), Some(false)))
            })
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        keys
    }

    fn restore_expanded_story_loading(&mut self, story_keys: &[String]) {
        for key in story_keys {
            self.list
                .collapsed_stories
                .remove(&(key.clone(), Some(true)));
            self.list
                .collapsed_stories
                .remove(&(key.clone(), Some(false)));
            self.list.start_loading_children(key);
        }
    }

    fn refetch_story_children(&self, story_keys: &[String]) {
        for key in story_keys {
            actions::fetch_children::spawn(
                self.message_tx.clone(),
                self.client.clone(),
                key.clone(),
                self.config.jira.jira_jql.clone(),
            );
        }
    }

    fn selected_issue_restore_keys(&self) -> Vec<String> {
        let Some(issue) = self.list.selected_issue(&self.issues, &self.story_children) else {
            return Vec::new();
        };

        let mut keys = Vec::with_capacity(1);
        keys.push(issue.key.clone());
        keys.extend(
            crate::issue::ancestors(issue)
                .into_iter()
                .map(|ancestor| ancestor.key),
        );
        keys
    }

    fn restore_selection_for_issue_keys(&mut self, issue_keys: &[String]) -> bool {
        if self.list.display_rows.is_empty() {
            self.list.selected_index = 0;
            return false;
        }

        for (index, key) in issue_keys.iter().enumerate() {
            if let Some(position) = self.display_row_index_for_issue_key(key) {
                self.list.selected_index = position;
                return index == 0;
            }
        }

        self.list.selected_index = self
            .list
            .selected_index
            .min(self.list.display_rows.len() - 1);
        false
    }

    fn restore_pending_selection(&mut self) {
        let Some(key) = self.pending_selected_issue_key.clone() else {
            return;
        };
        if self.select_issue_key_if_visible(&key) {
            self.pending_selected_issue_key = None;
        }
    }

    fn select_issue_key_if_visible(&mut self, key: &str) -> bool {
        let Some(position) = self.display_row_index_for_issue_key(key) else {
            return false;
        };
        self.list.selected_index = position;
        self.list.adjust_scroll_offset();
        true
    }

    fn display_row_index_for_issue_key(&self, key: &str) -> Option<usize> {
        self.list.display_rows.iter().position(|row| {
            self.list
                .issue_for_display_row(row, &self.issues, &self.story_children)
                .map(|issue| issue.key == key)
                .unwrap_or(false)
        })
    }

    /// Spawn children fetch for a story key.
    pub fn spawn_fetch_children(&mut self, parent_key: &str) {
        if self.list.loading_children.contains(parent_key)
            || self.story_children.contains_key(parent_key)
        {
            return;
        }
        self.list.start_loading_children(parent_key);
        actions::fetch_children::spawn(
            self.message_tx.clone(),
            self.client.clone(),
            parent_key.to_string(),
            self.config.jira.jira_jql.clone(),
        );
    }

    pub fn reload_repo_entries(&mut self) {
        match repos::scan_repos(&self.config.repos_dir) {
            Ok(entries) => {
                self.repo_entries = entries;
                self.repo_error = None;
            }
            Err(err) => {
                let message = format!("Failed to scan repos: {err}");
                self.repo_entries.clear();
                self.repo_error = Some(message.clone());
                self.status_bar.set_error(message);
            }
        }
    }

    /// Scan openspec changes for pending import tasks.
    pub fn spawn_scan_import_tasks(&self) {
        actions::scan_import_tasks::spawn(self.message_tx.clone(), self.config.repos_dir.clone());
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

    pub fn tick_spinner(&mut self) {
        self.animation.tick_spinner();
    }

    /// Returns `true` when any background work is in progress.
    pub fn is_busy(&self) -> bool {
        self.loading || self.github_loading || !self.running_tasks.is_empty()
    }

    /// Returns `true` when any tracked PR has pending CI checks.
    pub fn has_pending_checks(&self) -> bool {
        self.github_prs
            .values()
            .any(|pr| pr.checks == CheckStatus::Pending)
    }

    /// Build a Cache from current app state and save to disk.
    pub fn save_cache(&self) {
        cache::save(&Cache {
            check_durations: self.check_durations.clone(),
            collapsed_stories: self.list.collapsed_stories.clone(),
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
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{
        actions::Message,
        apis::jira::Issue,
        fixtures::{test_app, test_issue},
    };

    use super::DisplayRow;

    #[test]
    fn expanded_loaded_story_keys_only_keeps_open_story_rows() {
        let mut app = test_app();
        app.story_children
            .insert("TEST-1".to_string(), vec![ticket_issue("TEST-2", None)]);
        app.story_children
            .insert("TEST-3".to_string(), vec![ticket_issue("TEST-4", None)]);
        app.list
            .collapsed_stories
            .insert(("TEST-3".to_string(), Some(true)));
        app.list
            .collapsed_stories
            .insert(("TEST-3".to_string(), Some(false)));

        assert_eq!(app.expanded_loaded_story_keys(), vec!["TEST-1".to_string()]);
    }

    #[test]
    fn restore_expanded_story_loading_keeps_story_open_during_reload() {
        let mut app = test_app();
        let story = story_issue("TEST-1", "Story parent");
        app.issues = vec![story.clone(), ticket_issue("TEST-3", Some(&story))];
        app.list
            .rebuild_display_rows(&app.issues, &app.story_children);
        app.story_children
            .insert("TEST-1".to_string(), vec![ticket_issue("TEST-2", None)]);
        app.list
            .collapsed_stories
            .remove(&("TEST-1".to_string(), Some(true)));
        app.list
            .collapsed_stories
            .remove(&("TEST-1".to_string(), Some(false)));

        let expanded_story_keys = app.expanded_loaded_story_keys();

        app.story_children.clear();
        app.restore_expanded_story_loading(&expanded_story_keys);
        app.list
            .rebuild_display_rows(&app.issues, &app.story_children);

        assert!(app.list.loading_children.contains("TEST-1"));
        // Story appears in BOARD (expanded) with its child issue from parent_groups
        assert!(matches!(
            app.list.display_rows.as_slice(),
            [
                DisplayRow::SectionHeader { label, .. },
                DisplayRow::StoryHeader {
                    key,
                    summary,
                    depth: 0,
                    section,
                },
                DisplayRow::Issue { index: 1, depth: 1, .. },
            ] if label == "BOARD" && key == "TEST-1" && summary == "Story parent" && section == &Some(true)
        ));
    }

    #[test]
    fn picked_up_without_branch_does_not_change_current_branch() {
        let mut app = test_app();
        app.current_branch = "existing-branch".to_string();

        app.handle_message(Message::PickedUp(Ok(crate::actions::PickUpResult {
            branch: None,
        })));

        assert_eq!(app.current_branch, "existing-branch");
    }

    #[test]
    fn restore_selection_falls_back_to_parent_story_during_reload() {
        let mut app = test_app();
        let story = story_issue("TEST-1", "Story parent");
        // Include a child in issues so the story appears via parent_groups after reload.
        app.issues = vec![story.clone(), ticket_issue("TEST-3", Some(&story))];
        app.story_children.insert(
            "TEST-1".to_string(),
            vec![ticket_issue("TEST-2", Some(&story))],
        );
        app.list
            .collapsed_stories
            .remove(&("TEST-1".to_string(), Some(true)));
        app.list
            .collapsed_stories
            .remove(&("TEST-1".to_string(), Some(false)));
        app.list
            .rebuild_display_rows(&app.issues, &app.story_children);
        app.list.selected_index = 1;

        let restore_keys = app.selected_issue_restore_keys();

        app.story_children.clear();
        app.restore_expanded_story_loading(&["TEST-1".to_string()]);
        app.list
            .rebuild_display_rows(&app.issues, &app.story_children);

        assert!(app.restore_selection_for_issue_keys(&restore_keys));
        assert_eq!(
            app.list
                .selected_issue(&app.issues, &app.story_children)
                .map(|issue| issue.key.as_str()),
            Some("TEST-1")
        );
    }

    #[test]
    fn children_loaded_restores_pending_child_selection() {
        let mut app = test_app();
        let story = story_issue("TEST-1", "Story parent");
        app.issues = vec![story.clone()];
        app.restore_expanded_story_loading(&["TEST-1".to_string()]);
        app.list
            .rebuild_display_rows(&app.issues, &app.story_children);
        app.pending_selected_issue_key = Some("TEST-2".to_string());

        app.handle_message(Message::ChildrenLoaded(
            "TEST-1".to_string(),
            Ok(vec![ticket_issue("TEST-2", Some(&story))]),
        ));

        assert_eq!(
            app.list
                .selected_issue(&app.issues, &app.story_children)
                .map(|issue| issue.key.as_str()),
            Some("TEST-2")
        );
        assert_eq!(app.pending_selected_issue_key, None);
    }

    fn story_issue(key: &str, summary: &str) -> Issue {
        let mut issue = test_issue();
        issue.key = key.to_string();
        issue
            .fields
            .insert("summary".to_string(), json!(summary.to_string()));
        issue.fields.insert(
            "issuetype".to_string(),
            json!({
                "description": "",
                "iconUrl": "",
                "id": "10000",
                "name": "Story",
                "self": "http://localhost/issuetype/10000",
                "subtask": false
            }),
        );
        issue
    }

    fn ticket_issue(key: &str, parent: Option<&Issue>) -> Issue {
        let mut issue = test_issue();
        issue.key = key.to_string();
        if let Some(parent) = parent {
            issue.fields.insert(
                "parent".to_string(),
                json!({
                    "key": parent.key.clone(),
                    "id": parent.id.clone(),
                    "self": parent.self_link.clone(),
                    "fields": {
                        "summary": parent.summary().unwrap_or_default(),
                        "issuetype": {
                            "description": "",
                            "iconUrl": "",
                            "id": "10000",
                            "name": "Story",
                            "self": "http://localhost/issuetype/10000",
                            "subtask": false
                        }
                    }
                }),
            );
        }
        issue
    }
}
