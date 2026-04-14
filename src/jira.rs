use std::{collections::BTreeMap, env};

use color_eyre::{eyre::eyre, Result};
use futures::StreamExt;
use gouqi::{
    issues::{CreateResponse, EditIssue},
    r#async::Jira as GouqiJira,
    users::UserSearchOptions,
    Board, Credentials, SearchOptions, Sprint, TransitionTriggerOptions, User,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::events::{Event, EventLevel, EventSource};

pub use gouqi::{Issue, IssueType, TransitionOption};

#[derive(Clone, Debug)]
pub struct JiraConfig {
    pub base_url: String,
    pub email: String,
    pub api_token: String,
    pub default_jql: String,
}

impl JiraConfig {
    pub fn from_env() -> Result<Self> {
        let required = |name: &str| -> Result<String> {
            env::var(name).map_err(|_| eyre!("{name} is not set"))
        };

        let base_url = required("JIRA_URL")?;
        let email = required("JIRA_EMAIL")?;
        let api_token = required("JIRA_API_TOKEN")?;
        let default_jql = required("JIRA_JQL")?;
        Ok(Self {
            base_url,
            email,
            api_token,
            default_jql,
        })
    }
}

#[derive(Clone)]
pub struct JiraClient {
    jira: GouqiJira,
    email: String,
}

impl JiraClient {
    pub fn new(config: &JiraConfig) -> Result<Self> {
        let host = config.base_url.trim_end_matches('/').to_string();
        let credentials = Credentials::Basic(config.email.clone(), config.api_token.clone());
        let jira = GouqiJira::new(&host, credentials)?;

        Ok(Self {
            jira,
            email: config.email.clone(),
        })
    }

    pub async fn search(&self, jql: &str) -> Result<Vec<Issue>> {
        let options = SearchOptions::builder()
            .max_results(50)
            .fields(vec![
                "summary",
                "description",
                "status",
                "assignee",
                "priority",
                "issuetype",
                "project",
                "labels",
                "parent",
                "created",
            ])
            .build();

        let issues: Vec<Issue> = self
            .jira
            .search()
            .stream(jql, &options)
            .await?
            .collect()
            .await;
        Ok(issues)
    }

    pub async fn get_myself(&self) -> Result<User> {
        let options = UserSearchOptions::builder()
            .query(self.email.clone())
            .max_results(1)
            .build();
        let users = self.jira.users().search(&options).await?;
        users
            .into_iter()
            .next()
            .ok_or_else(|| eyre!("No Jira user found for configured email"))
    }

    pub async fn assign_issue(&self, issue_key: &str, account_id: &str) -> Result<()> {
        // Bypass gouqi's assign() which sends {"assignee": "..."} instead of
        // the {"accountId": "..."} payload required by Jira Cloud REST API v3.
        let payload = json!({ "accountId": account_id });
        self.jira
            .put::<(), _>("api", &format!("/issue/{issue_key}/assignee"), payload)
            .await?;
        Ok(())
    }

    pub async fn get_transitions(&self, issue_key: &str) -> Result<Vec<TransitionOption>> {
        let transitions = self.jira.transitions(issue_key).list().await?;
        Ok(transitions)
    }

    pub async fn transition_issue(&self, issue_key: &str, transition_id: &str) -> Result<()> {
        let options = TransitionTriggerOptions::builder(transition_id).build();
        self.jira.transitions(issue_key).trigger(options).await?;
        Ok(())
    }

    pub async fn move_issue_to_active_board(&self, issue_key: &str) -> Result<()> {
        let Some((project_part, _)) = issue_key.split_once('-') else {
            return Err(eyre!("Issue key {issue_key} is missing a project prefix"));
        };
        let project_key = project_part.to_uppercase();

        let boards = self.collect_project_boards(&project_key).await?;
        if boards.is_empty() {
            return Err(eyre!("No Jira boards found for project {project_key}"));
        }

        let exact_matches: Vec<Board> = boards
            .iter()
            .filter(|board| {
                board
                    .location
                    .as_ref()
                    .and_then(|loc| loc.project_key.as_deref())
                    == Some(project_key.as_str())
            })
            .cloned()
            .collect();
        let matching_boards = if exact_matches.is_empty() {
            boards
        } else {
            exact_matches
        };

        let mut scrum_boards = Vec::new();
        let mut flow_boards = Vec::new();
        for board in matching_boards {
            if board.type_name.eq_ignore_ascii_case("scrum") {
                scrum_boards.push(board);
            } else {
                flow_boards.push(board);
            }
        }

        let mut scrum_with_active = Vec::new();
        for board in &scrum_boards {
            if let Some(sprint) = self.find_active_sprint(board).await? {
                scrum_with_active.push((board.clone(), sprint));
            }
        }

        if scrum_with_active.is_empty() {
            if !flow_boards.is_empty() {
                return Ok(());
            }
            let names = scrum_boards
                .into_iter()
                .map(|board| board.name)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(eyre!("No active sprint found for scrum boards: {names}"));
        }

        if scrum_with_active.len() > 1 {
            let names = scrum_with_active
                .iter()
                .map(|(board, _)| board.name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(eyre!("Multiple scrum boards have active sprints: {names}"));
        }

        let (_board, sprint) = scrum_with_active.into_iter().next().unwrap();
        self.jira
            .sprints()
            .move_issues(sprint.id, vec![issue_key.to_string()])
            .await?;
        Ok(())
    }

    async fn collect_project_boards(&self, project_key: &str) -> Result<Vec<Board>> {
        let mut collected = Vec::new();
        let mut start_at = 0;
        loop {
            let options = SearchOptions::builder()
                .project_key_or_id(project_key)
                .max_results(50)
                .start_at(start_at)
                .build();
            let page = self.jira.boards().list(&options).await?;
            let is_last = page.is_last;
            let max_results = page.max_results;
            collected.extend(page.values);
            start_at += max_results;
            if is_last {
                break;
            }
        }
        Ok(collected)
    }

    async fn find_active_sprint(&self, board: &Board) -> Result<Option<Sprint>> {
        let mut start_at = 0;
        loop {
            let options = SearchOptions::builder()
                .state("active")
                .max_results(50)
                .start_at(start_at)
                .build();
            let page = self.jira.sprints().list(board, &options).await?;
            let is_last = page.is_last;
            let max_results = page.max_results;
            if let Some(active) = page.values.into_iter().find(|sprint| {
                sprint
                    .state
                    .as_deref()
                    .map(|state| state.eq_ignore_ascii_case("active"))
                    .unwrap_or(false)
            }) {
                return Ok(Some(active));
            }
            if is_last {
                break;
            }
            start_at += max_results;
        }
        Ok(None)
    }

    pub async fn update_labels(&self, issue_key: &str, labels: &[String]) -> Result<()> {
        let values = labels.iter().cloned().map(Value::String).collect();
        let mut fields = BTreeMap::new();
        fields.insert("labels".to_string(), Value::Array(values));
        let edit = EditIssue { fields };
        self.jira.issues().update(issue_key, edit).await?;
        Ok(())
    }

    pub async fn get_issue_events(&self, issue_key: &str) -> Result<Vec<Event>> {
        let path = format!("/issue/{issue_key}?expand=changelog&fields=status,assignee,comment");
        let value = self.jira.get::<Value>("api", &path).await?;
        let response: ChangelogResponse = serde_json::from_value(value)?;

        let mut events = Vec::new();
        let Some(wrapper) = response.changelog else {
            return Ok(events);
        };

        for history in wrapper.histories {
            for item in history.items {
                let field = item.field.to_lowercase();
                let title;
                let level;
                let detail;

                if field == "status" {
                    let to_value = item.to_string.clone().unwrap_or_default();
                    let to_lower = to_value.to_lowercase();
                    level = if to_lower.contains("done") {
                        EventLevel::Success
                    } else if to_lower.contains("progress") {
                        EventLevel::Warning
                    } else if to_lower.contains("review") {
                        EventLevel::Info
                    } else if to_lower.contains("blocked") {
                        EventLevel::Error
                    } else {
                        EventLevel::Neutral
                    };
                    title = format!("Status → {to_value}");
                    detail = item.from_string.clone();
                } else if field == "assignee" {
                    level = EventLevel::Info;
                    let to_value = item
                        .to_string
                        .clone()
                        .unwrap_or_else(|| "Unassigned".into());
                    title = format!("Assigned to {to_value}");
                    detail = item.from_string.clone();
                } else {
                    level = EventLevel::Neutral;
                    let to_value = item.to_string.clone().unwrap_or_default();
                    title = format!("{field} updated");
                    detail = if to_value.is_empty() {
                        item.from_string.clone()
                    } else {
                        Some(to_value)
                    };
                }

                events.push(Event {
                    at: history.created.clone(),
                    source: EventSource::Jira,
                    level,
                    title,
                    detail,
                });
            }
        }

        events.sort_by(|a, b| b.at.cmp(&a.at));
        Ok(events)
    }

    pub async fn create_issue(
        &self,
        project_key: &str,
        issue_type_id: &str,
        summary: &str,
        description: Option<&str>,
        parent_key: Option<&str>,
    ) -> Result<String> {
        let mut fields = serde_json::Map::new();
        fields.insert("project".into(), json!({ "key": project_key }));
        fields.insert("issuetype".into(), json!({ "id": issue_type_id }));
        fields.insert("summary".into(), json!(summary));

        if let Some(text) = description {
            fields.insert("description".into(), json!(text));
        }

        if let Some(parent) = parent_key {
            fields.insert("parent".into(), json!({ "key": parent }));
        }

        let payload = json!({ "fields": Value::Object(fields) });
        let created: CreateResponse = self.jira.post("api", "/issue", payload).await?;
        Ok(created.key)
    }

    pub async fn get_issue_types(&self, project_key: &str) -> Result<Vec<IssueType>> {
        let path = format!("/project/{project_key}");
        let value: Value = self.jira.get("api", &path).await?;
        let types = value
            .get("issueTypes")
            .and_then(|v| serde_json::from_value::<Vec<IssueType>>(v.clone()).ok())
            .unwrap_or_default();
        if types.is_empty() {
            return Err(eyre!("No issue types found for project {project_key}"));
        }
        Ok(types)
    }
}

#[derive(Deserialize)]
struct ChangelogResponse {
    changelog: Option<ChangelogWrapper>,
}

#[derive(Deserialize)]
struct ChangelogWrapper {
    histories: Vec<ChangelogHistory>,
}

#[derive(Deserialize)]
struct ChangelogHistory {
    created: String,
    items: Vec<ChangelogItem>,
}

#[derive(Deserialize)]
struct ChangelogItem {
    field: String,
    #[serde(rename = "fromString")]
    from_string: Option<String>,
    #[serde(rename = "toString")]
    to_string: Option<String>,
}
