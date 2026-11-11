use std::collections::HashSet;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use crate::config::Config;
use crate::model::{Issue, Parent, Person};

const FIELDS: &[&str] = &[
    "summary",
    "issuetype",
    "description",
    "status",
    "assignee",
    "labels",
    "parent",
    "updated",
];
const MAX_ISSUES: usize = 500;

/// A column of the board with the ids of the statuses shown in it.
pub struct Column {
    pub name: String,
    pub status_ids: Vec<String>,
}

pub struct Board {
    pub id: u64,
    pub columns: Vec<Column>,
}

#[derive(Clone)]
pub struct Jira {
    base: String,
    authorization: String,
}

impl Jira {
    pub fn new(config: &Config) -> Self {
        let credentials = format!("{}:{}", config.jira_email, config.jira_token);
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials);
        Self {
            base: config.jira_url.clone(),
            authorization: format!("Basic {encoded}"),
        }
    }

    fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        let request = ureq::request(method, &format!("{}{path}", self.base))
            .set("Authorization", &self.authorization)
            .set("Accept", "application/json");
        let response = match body {
            Some(body) => request.send_json(body),
            None => request.call(),
        };
        let text = match response {
            Ok(response) => response.into_string()?,
            Err(ureq::Error::Status(code, response)) => {
                let detail = response.into_string().unwrap_or_default();
                bail!("Jira {method} {path} returned {code}: {detail}")
            }
            Err(error) => return Err(anyhow!(error).context(format!("Jira {method} {path}"))),
        };
        if text.trim().is_empty() {
            return Ok(Value::Null); // 204 No Content
        }
        serde_json::from_str(&text).with_context(|| format!("parsing Jira response of {path}"))
    }

    pub fn search(&self, jql: &str) -> Result<Vec<Issue>> {
        let mut issues = Vec::new();
        let mut next_page: Option<String> = None;
        loop {
            let mut body = json!({ "jql": jql, "fields": FIELDS, "maxResults": 100 });
            if let Some(token) = &next_page {
                body["nextPageToken"] = json!(token);
            }
            let page = self.request("POST", "/rest/api/3/search/jql", Some(body))?;
            let batch = page["issues"]
                .as_array()
                .context("Jira search returned no issues")?;
            issues.extend(batch.iter().map(parse_issue));
            next_page = page["nextPageToken"].as_str().map(str::to_owned);
            if next_page.is_none() || issues.len() >= MAX_ISSUES {
                return Ok(issues);
            }
        }
    }

    pub fn myself(&self) -> Result<Person> {
        let me = self.request("GET", "/rest/api/3/myself", None)?;
        parse_person(&me).context("Jira returned no account for these credentials")
    }

    /// Assign the issue, move it to In Progress and onto the board.
    pub fn claim(&self, key: &str, account_id: &str) -> Result<()> {
        let assignee = json!({ "accountId": account_id });
        self.request(
            "PUT",
            &format!("/rest/api/3/issue/{key}/assignee"),
            Some(assignee),
        )?;
        self.transition(key, "progress")?;
        self.move_to_board(key)
    }

    /// Apply the first available transition whose name contains `name`.
    pub fn transition(&self, key: &str, name: &str) -> Result<()> {
        let path = format!("/rest/api/3/issue/{key}/transitions");
        let available = self.request("GET", &path, None)?;
        let transitions = available["transitions"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let name_of = |transition: &Value| transition["name"].as_str().unwrap_or("").to_owned();
        let Some(transition) = transitions
            .iter()
            .find(|transition| name_of(transition).to_lowercase().contains(name))
        else {
            let names: Vec<String> = transitions.iter().map(name_of).collect();
            bail!(
                "{key} has no '{name}' transition; available: {}",
                names.join(", ")
            );
        };
        self.request(
            "POST",
            &path,
            Some(json!({ "transition": { "id": transition["id"] } })),
        )?;
        Ok(())
    }

    /// The single board of a project, with its columns from left to right.
    pub fn board(&self, project: &str) -> Result<Board> {
        let path = format!("/rest/agile/1.0/board?projectKeyOrId={project}");
        let boards = self.request("GET", &path, None)?;
        let boards = boards["values"].as_array().cloned().unwrap_or_default();
        let [board] = boards.as_slice() else {
            bail!("expected one board for {project}, found {}", boards.len());
        };
        let id = board["id"].as_u64().context("board without an id")?;
        let configuration = self.request(
            "GET",
            &format!("/rest/agile/1.0/board/{id}/configuration"),
            None,
        )?;
        let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
        let columns = configuration["columnConfig"]["columns"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|column| Column {
                name: text(&column["name"]),
                status_ids: column["statuses"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|status| text(&status["id"]))
                    .collect(),
            })
            .collect();
        Ok(Board { id, columns })
    }

    /// The keys of the issues in the board's backlog.
    pub fn backlog(&self, board: u64) -> Result<HashSet<String>> {
        let mut keys = HashSet::new();
        loop {
            let path = format!(
                "/rest/agile/1.0/board/{board}/backlog?fields=summary&maxResults=100&startAt={}",
                keys.len()
            );
            let page = self.request("GET", &path, None)?;
            let issues = page["issues"].as_array().cloned().unwrap_or_default();
            keys.extend(
                issues
                    .iter()
                    .filter_map(|issue| issue["key"].as_str())
                    .map(str::to_owned),
            );
            if issues.is_empty() || keys.len() as u64 >= page["total"].as_u64().unwrap_or(0) {
                return Ok(keys);
            }
        }
    }

    /// Take the issue out of the backlog and put it on the board.
    pub fn move_to_board(&self, key: &str) -> Result<()> {
        self.move_issue(key, "board")
    }

    /// Take the issue off the board and put it in the backlog.
    pub fn move_to_backlog(&self, key: &str) -> Result<()> {
        self.move_issue(key, "backlog")
    }

    fn move_issue(&self, key: &str, destination: &str) -> Result<()> {
        let board = self.board(key.split('-').next().unwrap_or(key))?;
        let path = format!("/rest/agile/1.0/{destination}/{}/issue", board.id);
        self.request("POST", &path, Some(json!({ "issues": [key] })))?;
        Ok(())
    }

    pub fn add_label(&self, key: &str, label: &str) -> Result<()> {
        self.change_labels(key, Some(label), &[])
    }

    /// Add one label and remove others in a single edit; all other labels stay as they are.
    pub fn change_labels(&self, key: &str, add: Option<&str>, remove: &[String]) -> Result<()> {
        let mut changes: Vec<Value> = remove
            .iter()
            .map(|label| json!({ "remove": label }))
            .collect();
        changes.extend(add.map(|label| json!({ "add": label })));
        let update = json!({ "update": { "labels": changes } });
        self.request("PUT", &format!("/rest/api/3/issue/{key}"), Some(update))?;
        Ok(())
    }

    /// Create a task and return its key.
    pub fn create_task(
        &self,
        project: &str,
        summary: &str,
        parent: Option<&str>,
    ) -> Result<String> {
        let mut fields = json!({
            "project": { "key": project },
            "issuetype": { "name": "Task" },
            "summary": summary,
        });
        if let Some(parent) = parent {
            fields["parent"] = json!({ "key": parent });
        }
        let created = self.request(
            "POST",
            "/rest/api/3/issue",
            Some(json!({ "fields": fields })),
        )?;
        created["key"]
            .as_str()
            .map(str::to_owned)
            .context("Jira did not return the new issue key")
    }
}

fn parse_issue(value: &Value) -> Issue {
    let fields = &value["fields"];
    let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    let mut description = String::new();
    push_document_text(&fields["description"], &mut description);
    Issue {
        key: text(&value["key"]),
        summary: text(&fields["summary"]),
        status: text(&fields["status"]["name"]),
        status_id: text(&fields["status"]["id"]),
        is_subtask: fields["issuetype"]["subtask"].as_bool().unwrap_or_default(),
        is_epic: fields["issuetype"]["hierarchyLevel"]
            .as_i64()
            .unwrap_or_default()
            > 0,
        listed: true, // refined, like the next two, once all issues are known
        epic: None,
        section: String::new(), // assigned once the board is known
        assignee: parse_person(&fields["assignee"]),
        labels: fields["labels"]
            .as_array()
            .into_iter()
            .flatten()
            .map(text)
            .collect(),
        parent: fields["parent"]["key"].as_str().map(|key| Parent {
            key: key.to_owned(),
            summary: text(&fields["parent"]["fields"]["summary"]),
        }),
        updated: fields["updated"].as_str().and_then(parse_time),
        description: description.trim().to_owned(),
    }
}

fn parse_person(value: &Value) -> Option<Person> {
    Some(Person {
        account_id: value["accountId"].as_str()?.to_owned(),
        name: value["displayName"].as_str().unwrap_or_default().to_owned(),
    })
}

fn parse_time(timestamp: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_str(timestamp, "%Y-%m-%dT%H:%M:%S%.f%z") // Jira writes the offset without a colon
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

/// Flatten an Atlassian Document Format node into plain text, one blank line between blocks.
fn push_document_text(node: &Value, text: &mut String) {
    match node["type"].as_str() {
        Some("text") => text.push_str(node["text"].as_str().unwrap_or_default()),
        Some("hardBreak") => text.push('\n'),
        Some(kind) => {
            let start = text.len();
            for child in node["content"].as_array().into_iter().flatten() {
                push_document_text(child, text);
            }
            let is_block = matches!(
                kind,
                "paragraph"
                    | "heading"
                    | "blockquote"
                    | "panel"
                    | "codeBlock"
                    | "listItem"
                    | "tableRow"
            );
            if is_block && text.len() > start && !text.ends_with("\n\n") {
                text.push_str(if text.ends_with('\n') { "\n" } else { "\n\n" });
            }
        }
        None => {}
    }
}
