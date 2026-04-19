use tokio::sync::mpsc;

use crate::{
    apis::jira::{JiraClient, JiraConfig},
    app::{App, DisplayRow},
    config::AppConfig,
    ui::{CiLogPopupState, ListViewState, SidebarState, StatusBarState, UiAnimationState},
};

use super::{issue::test_issue, pr::test_pr};

pub fn test_app() -> App {
    let config = JiraConfig {
        jira_url: "http://localhost".to_string(),
        jira_email: "tester@example.com".to_string(),
        jira_api_token: "token".to_string(),
        jira_jql: "project = TEST".to_string(),
    };
    let app_config = AppConfig {
        jira: config.clone(),
        repos_dir: std::path::PathBuf::from("/tmp/test-repos"),
    };
    let client = JiraClient::new(&config).expect("jira client");
    let (bg_tx, bg_rx) = mpsc::unbounded_channel();

    App {
        should_quit: false,
        screen: crate::app::Screen::List,
        input_mode: crate::app::InputMode::Normal,
        issues: Vec::new(),
        selected_index: 0,
        config: app_config,
        repo_entries: Vec::new(),
        repo_error: None,
        list_view: ListViewState::default(),
        status_bar: StatusBarState::default(),
        loading: false,
        client,
        my_account_id: String::new(),
        current_branch: String::new(),
        pending_g: false,
        active_branches: Default::default(),
        github_statuses: Default::default(),
        github_loading: false,
        animation: UiAnimationState::default(),
        latest_activity: Default::default(),
        display_rows: Vec::new(),
        inline_new: None,
        search_filter: String::new(),
        collapsed_stories: Default::default(),
        story_children: Default::default(),
        sidebar: SidebarState::default(),
        github_prs: Default::default(),
        check_durations: Default::default(),
        running_tasks: Default::default(),
        bg_tx,
        bg_rx,
        last_ci_refresh: std::time::Instant::now(),
        ci_log_popup: CiLogPopupState::default(),
        import_tasks_popup: None,
        pending_import_keys: Default::default(),
    }
}

pub fn selected_issue_app() -> App {
    let mut app = test_app();
    app.issues.push(test_issue());
    app.display_rows.push(DisplayRow::Issue {
        index: 0,
        depth: 0,
        child_of: None,
    });
    app
}

pub fn sidebar_app() -> App {
    let mut app = selected_issue_app();
    let issue_key = app.issues[0].key.clone();
    app.animation.spinner_tick = 2;
    app.github_prs.insert(issue_key.clone(), test_pr());
    app.sidebar.detail_loading.insert(issue_key);
    app
}
