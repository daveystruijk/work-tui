use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use color_eyre::Result;
use crossterm::event::KeyCode;
use tokio::sync::mpsc;

use crate::{
    actions::{self, Message},
    apis::{
        github::{CheckStatus, GithubStatus, PrInfo, ReviewDecision},
        jira::{build_issue_search_jql, Issue, JiraClient, JiraProject, JiraStatus},
    },
    cache::{self, Cache},
    config::AppConfig,
    repos::{self, RepoEntry},
    ticket::{TicketSources, TicketStore},
    ui::{
        CiLogsView, ConfirmDialogView, FilterPickerView, ImportTasksView, LabelPickerView,
        ListView, SidebarView, StatusBarView, UiAnimationView,
    },
    utils::time::parse_duration_secs,
};

#[derive(Debug, Clone)]
pub struct RunningAction {
    pub id: String,
    pub label: String,
    pub progress: Option<actions::Progress>,
}

/// Which visual section a ticket group belongs to in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ListSection {
    Board,
    Backlog,
}

/// A row in the display list.
#[derive(Debug, Clone)]
pub enum DisplayRow {
    /// Visual section divider.
    SectionHeader { section: ListSection, count: usize },
    /// A ticket row identified by its issue key.
    Ticket { key: String, depth: u8 },
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
    JiraFilterPicker,
    ConfirmDialog,
    HelpOverlay,
}

#[derive(Debug, Clone, Default)]
pub struct JiraFilterState {
    pub selected_project_key: Option<String>,
    pub selected_status_names: Vec<String>,
    pub available_projects: Vec<JiraProject>,
    pub available_statuses: HashMap<String, Vec<JiraStatus>>,
    pub auto_tag_enabled_project_keys: HashSet<String>,
    pub should_auto_open_picker: bool,
    pub loading_status_projects: HashSet<String>,
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
    pub filter_picker: Option<FilterPickerView>,
    pub status_bar: StatusBarView,
    pub loading: bool,
    pub client: JiraClient,
    pub my_account_id: String,
    pub current_branch: String,
    /// Maps issue key -> repo label for issues whose branch is currently checked out
    pub active_branches: HashMap<String, String>,
    /// Repo paths with uncommitted changes
    pub dirty_repos: HashSet<PathBuf>,
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
    pub confirm_dialog: Option<ConfirmDialogView>,
    pub help_overlay: Option<crate::ui::HelpOverlayView>,
    pub pending_selected_issue_key: Option<String>,
    /// When set, a prefetch of the selected PR detail is scheduled after a short delay.
    /// This avoids firing fetches while the user is scrolling quickly through the list.
    pub pending_prefetch_since: Option<std::time::Instant>,
    /// Enriched ticket data combining issues, PRs, repos, and active branches.
    pub ticket_store: TicketStore,
    pub jira_filter: JiraFilterState,
}

pub(crate) const DEFAULT_HIDDEN_JIRA_STATUSES: &[&str] = &["done", "on development", "canceled"];

impl AppView {
    pub fn new(config: AppConfig) -> Result<Self> {
        let client = JiraClient::new(&config.jira)?;
        let (message_tx, message_rx) = mpsc::unbounded_channel();
        let cached = cache::load();
        let should_auto_open_picker = cached.jira_project_key.is_none();
        let mut app = Self {
            should_quit: false,
            input_focus: InputFocus::default(),
            issues: Vec::new(),
            config,
            repo_entries: Vec::new(),
            repo_error: None,
            list: ListView::default(),
            label_picker: None,
            filter_picker: None,
            status_bar: StatusBarView::default(),
            loading: true,
            client,
            my_account_id: String::new(),
            current_branch: String::new(),
            active_branches: HashMap::new(),
            dirty_repos: HashSet::new(),
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
            confirm_dialog: None,
            help_overlay: None,
            pending_selected_issue_key: None,
            pending_prefetch_since: None,
            ticket_store: TicketStore::default(),
            jira_filter: JiraFilterState {
                selected_project_key: cached.jira_project_key.clone(),
                selected_status_names: cached.jira_status_names.clone(),
                available_projects: Vec::new(),
                available_statuses: HashMap::new(),
                auto_tag_enabled_project_keys: cached
                    .jira_auto_tag_enabled_project_keys
                    .into_iter()
                    .collect(),
                should_auto_open_picker,
                loading_status_projects: HashSet::new(),
            },
        };

        app.check_durations = cached.check_durations;
        app.list.collapsed_stories = cached.collapsed_stories;

        app.reload_repo_entries();

        Ok(app)
    }

    /// Kick off all initialization work as background tasks.
    pub fn spawn_initialize(&self) {
        actions::initialize::spawn(self.message_tx.clone(), self.client.clone());
    }

    /// Spawn a full refresh (issues + PRs + statuses).
    pub fn spawn_refresh(&mut self) {
        let Some(jql) = self.current_issue_jql() else {
            self.loading = false;
            return;
        };
        actions::refresh::spawn(self.message_tx.clone(), self.client.clone(), jql);
        self.last_ci_refresh = std::time::Instant::now();
    }

    /// Extract the GitHub org from the first repo entry that has a slug.
    fn github_org(&self) -> Option<String> {
        self.repo_entries
            .iter()
            .filter_map(|entry| entry.github_slug.as_deref())
            .find_map(|slug| slug.split_once('/').map(|(org, _)| org.to_string()))
    }

    /// Extract the Jira project key prefix (e.g. "INI-") from the JQL or issues.
    fn project_key_prefix(&self) -> Option<String> {
        if let Some(project_key) = self.jira_filter.selected_project_key.as_deref() {
            return Some(format!("{project_key}-"));
        }

        // Fallback: derive from first issue key
        self.issues
            .first()
            .and_then(|issue| issue.key.split_once('-'))
            .map(|(prefix, _)| format!("{prefix}-"))
    }

    /// Spawn GitHub PR search for branches matching the project key prefix.
    pub fn spawn_github_prs(&mut self) {
        let Some(org) = self.github_org() else {
            return;
        };
        let Some(head_prefix) = self.project_key_prefix() else {
            return;
        };
        actions::fetch_github_prs::spawn(self.message_tx.clone(), org, head_prefix);
        self.last_ci_refresh = std::time::Instant::now();
    }

    /// Spawn GitHub PR refresh (same query as initial fetch).
    pub fn spawn_github_prs_active(&mut self) {
        self.spawn_github_prs();
    }

    pub fn current_issue_jql(&self) -> Option<String> {
        let project_key = self.jira_filter.selected_project_key.as_deref()?;
        if self.jira_filter.selected_status_names.is_empty() {
            return None;
        }
        Some(build_issue_search_jql(
            project_key,
            &self.jira_filter.selected_status_names,
        ))
    }

    pub fn current_project_key(&self) -> Option<&str> {
        self.jira_filter.selected_project_key.as_deref()
    }

    pub fn default_status_names_for_project(&self, project_key: &str) -> Vec<String> {
        let Some(statuses) = self.jira_filter.available_statuses.get(project_key) else {
            return Vec::new();
        };
        let excluded: HashSet<_> = DEFAULT_HIDDEN_JIRA_STATUSES
            .iter()
            .map(|status_name| status_name.to_string())
            .collect();

        let mut selected: Vec<String> = statuses
            .iter()
            .filter(|status| !excluded.contains(&status.name.to_ascii_lowercase()))
            .map(|status| status.name.clone())
            .collect();

        if selected.is_empty() {
            selected = statuses.iter().map(|status| status.name.clone()).collect();
        }

        selected
    }

    pub fn available_statuses_for_project(&self, project_key: &str) -> &[JiraStatus] {
        self.jira_filter
            .available_statuses
            .get(project_key)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn status_names_for_project(&self, project_key: &str) -> Vec<String> {
        self.available_statuses_for_project(project_key)
            .iter()
            .map(|status| status.name.clone())
            .collect()
    }

    pub fn spawn_project_statuses(&mut self, project_key: &str) {
        if self
            .jira_filter
            .available_statuses
            .contains_key(project_key)
            || !self
                .jira_filter
                .loading_status_projects
                .insert(project_key.to_string())
        {
            return;
        }
        actions::fetch_jira_filters::spawn_project_statuses(
            self.message_tx.clone(),
            self.client.clone(),
            project_key.to_string(),
        );
    }

    pub fn apply_jira_filter(&mut self, project_key: String, status_names: Vec<String>) {
        self.jira_filter.selected_project_key = Some(project_key);
        self.jira_filter.selected_status_names = status_names;
        self.save_cache();
        self.loading = true;
        self.spawn_refresh();
    }

    pub fn is_auto_tagging_enabled_for_project(&self, project_key: &str) -> bool {
        self.jira_filter
            .auto_tag_enabled_project_keys
            .contains(project_key)
    }

    pub fn is_current_project_auto_tagging_enabled(&self) -> bool {
        let Some(project_key) = self.current_project_key() else {
            return false;
        };
        self.is_auto_tagging_enabled_for_project(project_key)
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
        let Some(ticket) = self.list.selected_ticket(&self.ticket_store) else {
            return;
        };
        let issue_key = ticket.issue.key.clone();
        if self.sidebar.detail_loaded.contains(&issue_key)
            || self.sidebar.detail_loading.contains(&issue_key)
        {
            return;
        }
        let Some(pr) = ticket.pr.as_ref() else {
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
        if !self.is_current_project_auto_tagging_enabled() {
            return;
        }
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
        if !self.is_current_project_auto_tagging_enabled() {
            return;
        }
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
        let needs_steps = self.sidebar.handle_message(&msg, &mut self.github_prs);
        self.ci_log_popup.handle_message(&msg, &mut self.github_prs);

        if let Some(issue_key) = needs_steps {
            if let Some(pr) = self.github_prs.get(&issue_key) {
                self.sidebar.steps_loading.insert(issue_key.clone());
                actions::fetch_check_run_steps::spawn(
                    self.message_tx.clone(),
                    issue_key,
                    pr.repo_slug.clone(),
                    pr.check_runs.clone(),
                );
            }
        }

        match msg {
            Message::CurrentBranch(branch) => {
                self.current_branch = branch;
            }
            Message::Myself(result) => {
                if let Ok(account_id) = result {
                    self.my_account_id = account_id;
                }
            }
            Message::ProjectsLoaded(result) => self.handle_projects_loaded_message(result),
            Message::ProjectStatusesLoaded(project_key, result) => {
                self.handle_project_statuses_loaded_message(project_key, result)
            }
            Message::Issues(result) => self.handle_issues_message(result),
            Message::GithubPrs(all_prs, _) => {
                self.handle_github_prs_message(all_prs);
            }
            Message::GithubPrDetail(_, _) => {}
            Message::ActiveBranches(active, dirty) => {
                self.active_branches = active;
                self.dirty_repos = dirty;
                self.rebuild_tickets();
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
                if let Ok(pr_number) = result {
                    self.apply_approve_auto_merged(pr_number);
                }
                self.spawn_github_prs_active();
            }
            Message::Finished(issue_key, result) => {
                if result.is_ok() {
                    self.apply_finished(&issue_key);
                    self.spawn_sync_story_statuses();
                }
            }
            Message::InlineCreated(result) => self.handle_inline_created_message(result),
            Message::AutoLabeled(_key, _result) => {}
            Message::LabelAdded(result) => {
                if let Ok((issue_key, label)) = result {
                    self.apply_label_added(&issue_key, &label);
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
            Message::CheckRunSteps(_, _) => {}
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
            Message::StoryStatusSynced(synced) => {
                self.apply_story_status_synced(&synced);
            }
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

    fn handle_projects_loaded_message(&mut self, result: Result<Vec<JiraProject>>) {
        match result {
            Ok(projects) => {
                self.jira_filter.available_projects = projects;
                if self.jira_filter.available_projects.is_empty() {
                    self.loading = false;
                    return;
                }

                let project_exists = |project_key: &str| {
                    self.jira_filter
                        .available_projects
                        .iter()
                        .any(|project| project.key == project_key)
                };

                let selected_project_key = self
                    .jira_filter
                    .selected_project_key
                    .clone()
                    .filter(|project_key| project_exists(project_key));

                if self.jira_filter.selected_project_key.is_some() && selected_project_key.is_none()
                {
                    self.jira_filter.should_auto_open_picker = true;
                    self.jira_filter.selected_status_names.clear();
                }

                self.jira_filter.selected_project_key = selected_project_key;

                if let Some(project_key) = self.current_project_key() {
                    let project_key = project_key.to_string();
                    self.spawn_project_statuses(&project_key);
                } else {
                    self.loading = false;
                }

                if self.jira_filter.should_auto_open_picker {
                    crate::ui::filter_picker::open(self);
                    self.jira_filter.should_auto_open_picker = false;
                }
            }
            Err(_) => {
                self.loading = false;
            }
        }
    }

    fn handle_project_statuses_loaded_message(
        &mut self,
        project_key: String,
        result: Result<Vec<JiraStatus>>,
    ) {
        self.jira_filter
            .loading_status_projects
            .remove(&project_key);
        let Ok(statuses) = result else {
            if self.current_project_key() == Some(project_key.as_str()) {
                self.loading = false;
            }
            return;
        };

        self.jira_filter
            .available_statuses
            .insert(project_key.clone(), statuses);

        if self.current_project_key() == Some(project_key.as_str()) {
            let available_names: HashSet<_> = self
                .status_names_for_project(&project_key)
                .into_iter()
                .map(|status_name| status_name.to_ascii_lowercase())
                .collect();
            self.jira_filter
                .selected_status_names
                .retain(|status_name| available_names.contains(&status_name.to_ascii_lowercase()));

            if self.jira_filter.selected_status_names.is_empty() {
                self.jira_filter.selected_status_names =
                    self.default_status_names_for_project(&project_key);
            }

            self.save_cache();
            self.loading = true;
            self.spawn_refresh();
        }

        let default_status_names = self.default_status_names_for_project(&project_key);
        if let Some(picker) = self.filter_picker.as_mut() {
            picker.hydrate_status_selection(&project_key, default_status_names);
        }
    }

    fn handle_issues_message(&mut self, result: Result<Vec<Issue>>) {
        match result {
            Ok(issues) => {
                let expanded_story_keys = self.expanded_loaded_story_keys();
                let selection_restore_keys = self.selected_issue_restore_keys();
                self.issues = issues;
                // Retain story_children for stories that still exist in the
                // new issues list. Clearing eagerly causes a visible flicker
                // because the async refetch hasn't completed yet.
                self.retain_valid_story_children();
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
                self.rebuild_tickets();
                self.refetch_story_children(&expanded_story_keys);
                self.spawn_active_branches();
                self.spawn_github_prs();
                self.spawn_tag_jira_repos();
                self.spawn_scan_import_tasks();
                self.spawn_sync_story_statuses();
            }
            Err(_) => {
                self.loading = false;
            }
        }
    }

    /// Remove story_children entries whose parent key no longer appears in the
    /// current issues list (or as a child of another story). Keeps existing
    /// children data so the UI doesn't flicker while the async refetch is
    /// in-flight.
    fn retain_valid_story_children(&mut self) {
        let known_keys: HashSet<String> = self
            .issues
            .iter()
            .map(|issue| issue.key.clone())
            .chain(
                self.story_children
                    .values()
                    .flatten()
                    .map(|issue| issue.key.clone()),
            )
            .collect();
        self.story_children
            .retain(|key, _| known_keys.contains(key));
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
        self.rebuild_tickets();
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
                        .ticket_for_display_row(row, &self.ticket_store)
                        .map(|ticket| ticket.issue.key == key)
                        .unwrap_or(false)
                });
                if let Some(index) = found_index {
                    self.list.selected_index = index;
                } else {
                    self.pending_selected_issue_key = Some(key);
                }
            }
            Err(err) => {
                self.status_bar.set_error(format!("Failed: {err}"));
                self.input_focus = InputFocus::List;
                self.list.cancel_inline_new();
            }
        }
    }

    /// Optimistically update issue status to "Review" after finishing.
    fn apply_finished(&mut self, issue_key: &str) {
        let issue = self
            .issues
            .iter_mut()
            .chain(self.story_children.values_mut().flatten())
            .find(|issue| issue.key == issue_key);
        if let Some(issue) = issue {
            if let Some(status) = issue.fields.get("status").cloned() {
                let mut status_obj = status;
                if let Some(obj) = status_obj.as_object_mut() {
                    obj.insert("name".to_string(), serde_json::json!("Review"));
                    issue
                        .fields
                        .insert("status".to_string(), status_obj.clone());
                }
            }
        }
        // Also refresh PRs since a new PR was created.
        self.spawn_github_prs_active();
    }

    /// Optimistically mark a PR as approved with auto-merge enabled.
    fn apply_approve_auto_merged(&mut self, pr_number: u64) {
        for pr in self.github_prs.values_mut() {
            if pr.number == pr_number {
                pr.auto_merge_enabled = true;
                pr.review_decision = Some(ReviewDecision::Approved);
                break;
            }
        }
        for status in self.github_statuses.values_mut() {
            if let GithubStatus::Found(pr) = status {
                if pr.number == pr_number {
                    pr.auto_merge_enabled = true;
                    pr.review_decision = Some(ReviewDecision::Approved);
                    break;
                }
            }
        }
    }

    /// Optimistically update the labels field on a local issue after a label was added.
    fn apply_label_added(&mut self, issue_key: &str, label: &str) {
        let issue = self
            .issues
            .iter_mut()
            .chain(self.story_children.values_mut().flatten())
            .find(|issue| issue.key == issue_key);
        if let Some(issue) = issue {
            let mut labels = issue.labels();
            if !labels.contains(&label.to_string()) {
                labels.push(label.to_string());
            }
            issue
                .fields
                .insert("labels".to_string(), serde_json::json!(labels));
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
                    .insert((child.key.clone(), ListSection::Board));
                self.list
                    .collapsed_stories
                    .insert((child.key.clone(), ListSection::Backlog));
            }
        }
        self.story_children.insert(parent_key, children);
        self.rebuild_tickets();
        self.list
            .rebuild_display_rows(&self.issues, &self.story_children);
        self.restore_pending_selection();
        self.spawn_sync_story_statuses();
    }

    fn expanded_loaded_story_keys(&self) -> Vec<String> {
        let mut keys = self
            .story_children
            .keys()
            .filter(|key| {
                !self
                    .list
                    .collapsed_stories
                    .contains(&((*key).clone(), ListSection::Board))
                    || !self
                        .list
                        .collapsed_stories
                        .contains(&((*key).clone(), ListSection::Backlog))
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
                .remove(&(key.clone(), ListSection::Board));
            self.list
                .collapsed_stories
                .remove(&(key.clone(), ListSection::Backlog));
            // Only mark as loading if we don't already have children data.
            // Retained stale data is shown until the refetch completes,
            // avoiding a visible flicker.
            if !self.story_children.contains_key(key) {
                self.list.start_loading_children(key);
            }
        }
    }

    fn refetch_story_children(&self, story_keys: &[String]) {
        let Some(jql) = self.current_issue_jql() else {
            return;
        };
        for key in story_keys {
            actions::fetch_children::spawn(
                self.message_tx.clone(),
                self.client.clone(),
                key.clone(),
                jql.clone(),
            );
        }
    }

    fn selected_issue_restore_keys(&self) -> Vec<String> {
        let Some(ticket) = self.list.selected_ticket(&self.ticket_store) else {
            return Vec::new();
        };

        let mut keys = Vec::with_capacity(1);
        keys.push(ticket.issue.key.clone());
        keys.extend(
            crate::issue::ancestors(&ticket.issue)
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
                .ticket_for_display_row(row, &self.ticket_store)
                .map(|ticket| ticket.issue.key == key)
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
        let Some(issue) = self
            .issues
            .iter()
            .chain(self.story_children.values().flatten())
            .find(|issue| issue.key == parent_key)
        else {
            return;
        };
        if !crate::issue::is_expandable(issue) {
            return;
        }
        self.list.start_loading_children(parent_key);
        let Some(jql) = self.current_issue_jql() else {
            self.list.loading_children.remove(parent_key);
            return;
        };
        actions::fetch_children::spawn(
            self.message_tx.clone(),
            self.client.clone(),
            parent_key.to_string(),
            jql,
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

    /// Sync story statuses based on their children's statuses.
    pub fn spawn_sync_story_statuses(&self) {
        if !self.is_current_project_auto_tagging_enabled() {
            return;
        }
        let entries =
            actions::sync_story_statuses::compute_sync_entries(&self.issues, &self.story_children);
        actions::sync_story_statuses::spawn(self.message_tx.clone(), self.client.clone(), entries);
    }

    /// Optimistically update local issue statuses after story sync completes.
    fn apply_story_status_synced(
        &mut self,
        synced: &[(String, actions::sync_story_statuses::DerivedStatus)],
    ) {
        for (story_key, derived_status) in synced {
            let issue = self
                .issues
                .iter_mut()
                .chain(self.story_children.values_mut().flatten())
                .find(|issue| issue.key == *story_key);
            if let Some(issue) = issue {
                if let Some(status) = issue.fields.get("status").cloned() {
                    let mut status_obj = status;
                    if let Some(obj) = status_obj.as_object_mut() {
                        obj.insert(
                            "name".to_string(),
                            serde_json::json!(derived_status.label()),
                        );
                        issue
                            .fields
                            .insert("status".to_string(), status_obj.clone());
                    }
                }
            }
        }
        if !synced.is_empty() {
            self.rebuild_tickets();
            self.list
                .rebuild_display_rows(&self.issues, &self.story_children);
        }
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

    /// Returns the Ticket for the currently selected display row, if any.
    pub fn selected_ticket(&self) -> Option<&crate::ticket::Ticket> {
        self.list.selected_ticket(&self.ticket_store)
    }

    /// Rebuild the ticket store from current app state.
    pub fn rebuild_tickets(&mut self) {
        self.ticket_store = TicketStore::from_sources(&TicketSources {
            issues: &self.issues,
            story_children: &self.story_children,
            github_prs: &self.github_prs,
            active_branches: &self.active_branches,
            dirty_repos: &self.dirty_repos,
            repo_entries: &self.repo_entries,
        });
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
            jira_project_key: self.jira_filter.selected_project_key.clone(),
            jira_status_names: self.jira_filter.selected_status_names.clone(),
            jira_auto_tag_enabled_project_keys: self
                .jira_filter
                .auto_tag_enabled_project_keys
                .iter()
                .cloned()
                .collect(),
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

    use super::{DisplayRow, ListSection};

    #[test]
    fn expanded_loaded_story_keys_only_keeps_open_story_rows() {
        let mut app = test_app();
        app.story_children
            .insert("TEST-1".to_string(), vec![ticket_issue("TEST-2", None)]);
        app.story_children
            .insert("TEST-3".to_string(), vec![ticket_issue("TEST-4", None)]);
        app.list
            .collapsed_stories
            .insert(("TEST-3".to_string(), ListSection::Board));
        app.list
            .collapsed_stories
            .insert(("TEST-3".to_string(), ListSection::Backlog));

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
            .remove(&("TEST-1".to_string(), ListSection::Board));
        app.list
            .collapsed_stories
            .remove(&("TEST-1".to_string(), ListSection::Backlog));

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
                DisplayRow::SectionHeader { section: ListSection::Board, .. },
                DisplayRow::Ticket { key, depth: 0 },
                DisplayRow::Ticket { key: child_key, depth: 1 },
            ] if key == "TEST-1" && child_key == "TEST-3"
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
            .remove(&("TEST-1".to_string(), ListSection::Board));
        app.list
            .collapsed_stories
            .remove(&("TEST-1".to_string(), ListSection::Backlog));
        app.list
            .rebuild_display_rows(&app.issues, &app.story_children);
        app.rebuild_tickets();
        app.list.selected_index = 1;

        let restore_keys = app.selected_issue_restore_keys();

        app.story_children.clear();
        app.restore_expanded_story_loading(&["TEST-1".to_string()]);
        app.list
            .rebuild_display_rows(&app.issues, &app.story_children);
        app.rebuild_tickets();

        assert!(app.restore_selection_for_issue_keys(&restore_keys));
        assert_eq!(
            app.list
                .selected_ticket(&app.ticket_store)
                .map(|ticket| ticket.issue.key.as_str()),
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
                .selected_ticket(&app.ticket_store)
                .map(|ticket| ticket.issue.key.as_str()),
            Some("TEST-2")
        );
        assert_eq!(app.pending_selected_issue_key, None);
    }

    #[test]
    fn label_added_updates_issue_labels_in_place() {
        let mut app = test_app();
        let mut issue = test_issue();
        issue.key = "TEST-1".to_string();
        issue
            .fields
            .insert("labels".to_string(), json!(["existing-label"]));
        app.issues = vec![issue];
        app.list.display_rows.push(DisplayRow::Ticket {
            key: "TEST-1".to_string(),
            depth: 0,
        });

        app.handle_message(Message::LabelAdded(Ok((
            "TEST-1".to_string(),
            "new-label".to_string(),
        ))));

        let labels = app.issues[0].labels();
        assert_eq!(labels, vec!["existing-label", "new-label"]);
    }

    #[test]
    fn label_added_updates_story_child_labels() {
        let mut app = test_app();
        let mut child = test_issue();
        child.key = "TEST-2".to_string();
        child
            .fields
            .insert("labels".to_string(), json!(["old-label"]));
        app.story_children.insert("TEST-1".to_string(), vec![child]);

        app.handle_message(Message::LabelAdded(Ok((
            "TEST-2".to_string(),
            "repo-label".to_string(),
        ))));

        let labels = app.story_children["TEST-1"][0].labels();
        assert_eq!(labels, vec!["old-label", "repo-label"]);
    }

    #[test]
    fn approve_auto_merged_sets_pr_flags() {
        let mut app = test_app();
        let mut pr = crate::fixtures::pr::test_pr();
        pr.number = 99;
        pr.auto_merge_enabled = false;
        pr.review_decision = Some(crate::apis::github::ReviewDecision::ReviewRequired);
        app.github_prs.insert("TEST-1".to_string(), pr.clone());
        app.github_statuses.insert(
            "TEST-1".to_string(),
            crate::apis::github::GithubStatus::Found(pr),
        );

        app.handle_message(Message::ApproveAutoMerged(Ok(99)));

        let pr = &app.github_prs["TEST-1"];
        assert!(pr.auto_merge_enabled);
        assert_eq!(
            pr.review_decision,
            Some(crate::apis::github::ReviewDecision::Approved)
        );
        if let crate::apis::github::GithubStatus::Found(status_pr) = &app.github_statuses["TEST-1"]
        {
            assert!(status_pr.auto_merge_enabled);
            assert_eq!(
                status_pr.review_decision,
                Some(crate::apis::github::ReviewDecision::Approved)
            );
        } else {
            panic!("Expected GithubStatus::Found");
        }
    }

    #[test]
    fn finished_updates_issue_status_to_review() {
        let mut app = test_app();
        let mut issue = test_issue();
        issue.key = "TEST-1".to_string();
        app.issues = vec![issue];

        app.handle_message(Message::Finished(
            "TEST-1".to_string(),
            Ok("https://github.com/example/repo/pull/1".to_string()),
        ));

        let status = app.issues[0].status().expect("status should exist");
        assert_eq!(status.name, "Review");
    }

    #[test]
    fn inline_created_sets_pending_key_when_not_found() {
        let mut app = test_app();
        // No issues in display_rows, so the created key won't be found.

        app.handle_message(Message::InlineCreated(Ok("NEW-1".to_string())));

        assert_eq!(app.pending_selected_issue_key, Some("NEW-1".to_string()));
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

    /// retain_valid_story_children keeps entries for stories still present in
    /// the issues list, preventing a visible flicker during async refetch.
    #[test]
    fn retain_valid_story_children_keeps_existing_stories() {
        let mut app = test_app();
        let story = story_issue("TEST-1", "Story parent");
        app.issues = vec![story.clone(), ticket_issue("TEST-3", Some(&story))];
        app.story_children
            .insert("TEST-1".to_string(), vec![ticket_issue("TEST-2", None)]);

        app.retain_valid_story_children();

        assert!(
            app.story_children.contains_key("TEST-1"),
            "story_children should be retained for stories still in the issues list"
        );
        assert_eq!(app.story_children["TEST-1"][0].key, "TEST-2");
    }

    /// retain_valid_story_children removes entries for stories that no longer
    /// exist in the issues list.
    #[test]
    fn retain_valid_story_children_removes_gone_stories() {
        let mut app = test_app();
        let story = story_issue("TEST-1", "Story parent");
        app.issues = vec![story];
        app.story_children
            .insert("TEST-1".to_string(), vec![ticket_issue("TEST-2", None)]);
        app.story_children
            .insert("GONE-1".to_string(), vec![ticket_issue("GONE-2", None)]);

        app.retain_valid_story_children();

        assert!(app.story_children.contains_key("TEST-1"));
        assert!(
            !app.story_children.contains_key("GONE-1"),
            "story_children for removed stories should be cleaned up"
        );
    }

    /// Expanded stories with retained children don't show a loading spinner
    /// after restore_expanded_story_loading — they keep displaying the stale
    /// children until the refetch completes.
    #[test]
    fn restore_expanded_skips_loading_when_children_retained() {
        let mut app = test_app();
        let story = story_issue("TEST-1", "Story parent");
        app.issues = vec![story.clone(), ticket_issue("TEST-3", Some(&story))];
        app.story_children.insert(
            "TEST-1".to_string(),
            vec![ticket_issue("TEST-2", Some(&story))],
        );
        app.list
            .collapsed_stories
            .remove(&("TEST-1".to_string(), ListSection::Board));
        app.list
            .collapsed_stories
            .remove(&("TEST-1".to_string(), ListSection::Backlog));

        // Simulate the refresh path: retain children, then restore expanded state.
        app.retain_valid_story_children();
        app.restore_expanded_story_loading(&["TEST-1".to_string()]);
        app.list
            .rebuild_display_rows(&app.issues, &app.story_children);

        // No Loading rows should appear — retained children are shown instead.
        let has_loading = app
            .list
            .display_rows
            .iter()
            .any(|row| matches!(row, DisplayRow::Loading { .. }));
        assert!(
            !has_loading,
            "retained children should prevent loading spinners"
        );
        // The retained child should still be visible.
        let has_child = app
            .list
            .display_rows
            .iter()
            .any(|row| matches!(row, DisplayRow::Ticket { key, .. } if key == "TEST-2"));
        assert!(
            has_child,
            "retained child should still appear in display rows"
        );
    }

    /// Search results are preserved when story_children data is retained
    /// across a rebuild (simulating what happens during refresh).
    #[test]
    fn search_results_stable_with_retained_children() {
        let mut app = test_app();
        let story = story_issue("TEST-1", "Story parent");
        let mut child = ticket_issue("TEST-2", Some(&story));
        child
            .fields
            .insert("summary".to_string(), json!("Unique searchable child"));
        app.issues = vec![story];
        app.story_children.insert("TEST-1".to_string(), vec![child]);
        app.list
            .collapsed_stories
            .remove(&("TEST-1".to_string(), ListSection::Board));
        app.list
            .collapsed_stories
            .remove(&("TEST-1".to_string(), ListSection::Backlog));

        // Activate search that matches the child.
        app.list.search_filter = "searchable".to_string();
        app.list
            .rebuild_display_rows(&app.issues, &app.story_children);
        let rows_before = app.list.display_rows.len();
        assert!(rows_before > 0, "search should find matching child");

        // Simulate refresh path: retain children, restore expanded, rebuild.
        app.retain_valid_story_children();
        app.restore_expanded_story_loading(&["TEST-1".to_string()]);
        app.list
            .rebuild_display_rows(&app.issues, &app.story_children);

        assert_eq!(
            app.list.display_rows.len(),
            rows_before,
            "search results should remain stable when children are retained"
        );
    }
}
