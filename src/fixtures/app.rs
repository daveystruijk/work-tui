use tokio::sync::mpsc;

use crate::{
    apis::jira::{JiraClient, JiraConfig},
    app::{App, DisplayRow},
};

use super::{issue::test_issue, pr::test_pr};

pub fn test_app() -> App {
    let config = JiraConfig {
        base_url: "http://localhost".to_string(),
        email: "tester@example.com".to_string(),
        api_token: "token".to_string(),
        default_jql: "project = TEST".to_string(),
    };
    let client = JiraClient::new(&config).expect("jira client");
    let (bg_tx, bg_rx) = mpsc::unbounded_channel();

    App {
        should_quit: false,
        screen: crate::app::Screen::List,
        input_mode: crate::app::InputMode::Normal,
        issues: Vec::new(),
        selected_index: 0,
        jql: config.default_jql,
        repo_entries: Vec::new(),
        repo_error: None,
        label_picker: None,
        status_message: String::new(),
        loading: false,
        client,
        jira_base_url: config.base_url,
        my_account_id: String::new(),
        current_branch: String::new(),
        pending_g: false,
        list_area_height: 0,
        list_scroll_offset: 0,
        active_branches: Default::default(),
        github_statuses: Default::default(),
        github_loading: false,
        spinner_tick: 0,
        latest_activity: Default::default(),
        display_rows: Vec::new(),
        inline_new: None,
        search_filter: String::new(),
        collapsed_stories: Default::default(),
        story_children: Default::default(),
        loading_children: Default::default(),
        github_prs: Default::default(),
        github_pr_detail_loading: Default::default(),
        github_pr_detail_loaded: Default::default(),
        github_pr_detail_errors: Default::default(),
        check_durations: Default::default(),
        running_tasks: Default::default(),
        completed_tasks: Default::default(),
        bg_tx,
        bg_rx,
        last_ci_refresh: std::time::Instant::now(),
        last_updated: None,
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
    app.spinner_tick = 2;
    app.github_prs.insert(issue_key.clone(), test_pr());
    app.github_pr_detail_loading.insert(issue_key);
    app
}
