use std::{collections::BTreeMap, env};

use color_eyre::{eyre::eyre, Result};
use futures::StreamExt;
use gouqi::{
    issues::{CreateResponse, EditIssue},
    r#async::Jira as GouqiJira,
    users::UserSearchOptions,
    Credentials, SearchOptions, TransitionTriggerOptions, User,
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
