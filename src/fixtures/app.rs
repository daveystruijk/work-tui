use tokio::sync::mpsc;

use crate::{
    apis::jira::{JiraClient, JiraConfig},
    app::{AppView, DisplayRow},
    config::AppConfig,
    ui::{CiLogsView, ListView, SidebarView, StatusBarView, UiAnimationView},
};

use super::{issue::test_issue, pr::test_pr};

pub fn test_app() -> AppView {
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
    let (message_tx, message_rx) = mpsc::unbounded_channel();

    AppView {
        should_quit: false,
        input_focus: crate::app::InputFocus::default(),
        issues: Vec::new(),
        config: app_config,
        repo_entries: Vec::new(),
        repo_error: None,
        list: ListView::default(),
        label_picker: None,
        status_bar: StatusBarView::default(),
        loading: false,
        client,
        my_account_id: String::new(),
        current_branch: String::new(),
        active_branches: Default::default(),
        github_statuses: Default::default(),
        github_loading: false,
        animation: UiAnimationView::default(),
        story_children: Default::default(),
        sidebar: SidebarView::default(),
        github_prs: Default::default(),
        check_durations: Default::default(),
        running_tasks: Default::default(),
        message_tx,
        message_rx,
        last_ci_refresh: std::time::Instant::now(),
        ci_log_popup: CiLogsView::default(),
        previous_key: None,
        import_tasks_popup: None,
    }
}

pub fn selected_issue_app() -> AppView {
    let mut app = test_app();
    app.issues.push(test_issue());
    app.list.display_rows.push(DisplayRow::Issue {
        index: 0,
        depth: 0,
        child_of: None,
    });
    app
}

pub fn sidebar_app() -> AppView {
    let mut app = selected_issue_app();
    let issue_key = app.issues[0].key.clone();
    app.animation.spinner_tick = 2;
    app.github_prs.insert(issue_key.clone(), test_pr());
    app.sidebar.detail_loading.insert(issue_key);
    app
}
