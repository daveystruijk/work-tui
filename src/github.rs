#![allow(dead_code)]

use std::path::Path;

use color_eyre::{eyre::eyre, Result};
use serde::Deserialize;
use serde_json::Value;
use tokio::process::Command;

use crate::events::{Event, EventLevel, EventSource};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CheckStatus {
    Pending,
    Pass,
    Fail,
}

/// A single CI check run with timing info for ETA estimation.
#[derive(Debug, Clone)]
pub struct CheckRun {
    pub name: String,
    pub status: CheckStatus,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PrInfo {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub checks: CheckStatus,
    pub check_runs: Vec<CheckRun>,
    pub url: String,
    pub head_branch: String,
    /// The repo slug (owner/repo) this PR belongs to
    pub repo_slug: String,
    pub body: String,
}

#[derive(Debug, Clone)]
pub enum GithubStatus {
    Loading,
    NoPr,
    Found(PrInfo),
    Error(String),
}

#[derive(Deserialize)]
struct GhPrWithBranch {
    number: u64,
    #[serde(default)]
    title: String,
    state: String,
    url: String,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "statusCheckRollup", default)]
    status_check_rollup: Option<Vec<GhCheckRollup>>,
    #[serde(default)]
    body: String,
}

/// Fetch all PRs for a given `owner/repo` in a single `gh` call.
/// Returns PR number, state, URL, head branch name, and aggregated CI status.
pub async fn list_repo_prs(repo_slug: &str) -> Result<Vec<PrInfo>> {
    let output = Command::new("gh")
        .args([
            "pr",
            "list",
            "--repo",
            repo_slug,
            "--state",
            "open",
            "--json",
            "number,title,state,url,headRefName,statusCheckRollup,body",
            "--limit",
            "100",
        ])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(eyre!(
            "gh pr list --repo {repo_slug} failed: {}",
            if stderr.is_empty() {
                "unknown error"
            } else {
                &stderr
            }
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let prs: Vec<GhPrWithBranch> = serde_json::from_str(&stdout)?;

    let slug = repo_slug.to_string();
    Ok(prs
        .into_iter()
        .map(|pr| {
            let checks = aggregate_check_status(&pr.status_check_rollup);
            let check_runs = pr
                .status_check_rollup
                .as_deref()
                .unwrap_or_default()
                .iter()
                .filter(|c| !c.name.is_empty())
                .map(|c| CheckRun {
                    name: c.name.clone(),
                    status: check_run_status(c),
                    started_at: c.started_at.clone(),
                    completed_at: c.completed_at.clone(),
                })
                .collect();
            PrInfo {
                number: pr.number,
                title: pr.title,
                state: pr.state,
                checks,
                check_runs,
                url: pr.url,
                head_branch: pr.head_ref_name,
                repo_slug: slug.clone(),
                body: pr.body,
            }
        })
        .collect())
}

pub async fn list_all_repo_prs(repo_slugs: &[String]) -> (Vec<PrInfo>, Vec<String>) {
    let mut errors = Vec::new();
    let mut repo_queries: Vec<(String, String, String, String)> = Vec::new();

    for (idx, slug) in repo_slugs.iter().enumerate() {
        let Some((owner, name)) = slug.split_once('/') else {
            errors.push(format!("{slug}: invalid repo slug"));
            continue;
        };
        repo_queries.push((format!("repo_{idx}"), owner.to_string(), name.to_string(), slug.clone()));
    }

    if repo_queries.is_empty() {
        return (Vec::new(), errors);
    }

    let mut query = String::from("{");
    for (alias, owner, name, _) in &repo_queries {
        query.push_str(&format!(
            r#"
            {alias}: repository(owner: "{owner}", name: "{name}") {{
                nameWithOwner
                pullRequests(states: OPEN, first: 100, orderBy: {{field: UPDATED_AT, direction: DESC}}) {{
                    nodes {{
                        number
                        title
                        state
                        url
                        headRefName
                        body
                        statusCheckRollup: commits(last: 1) {{
                            nodes {{
                                commit {{
                                    statusCheckRollup {{
                                        contexts(first: 100) {{
                                            nodes {{
                                                __typename
                                                ... on CheckRun {{
                                                    name
                                                    status
                                                    conclusion
                                                    startedAt
                                                    completedAt
                                                }}
                                            }}
                                        }}
                                    }}
                                }}
                            }}
                        }}
                    }}
                }}
            }}
            "#,
            alias = alias,
            owner = owner,
            name = name,
        ));
    }
    query.push_str("\n}");

    let output = match Command::new("gh")
        .args(["api", "graphql", "-f", &format!("query={query}")])
        .output()
        .await
    {
        Ok(output) => output,
        Err(err) => {
            errors.push(format!("gh api graphql failed: {err}"));
            return (Vec::new(), errors);
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        errors.push(format!(
            "gh api graphql failed: {}",
            if stderr.is_empty() {
                "unknown error".to_string()
            } else {
                stderr
            }
        ));
        return (Vec::new(), errors);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let response: Value = match serde_json::from_str(&stdout) {
        Ok(value) => value,
        Err(err) => {
            errors.push(format!("Failed to parse GraphQL response: {err}"));
            return (Vec::new(), errors);
        }
    };

    if let Some(graph_errors) = response
        .get("errors")
        .and_then(|errs| errs.as_array())
    {
        for err in graph_errors {
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            errors.push(format!("GraphQL error: {message}"));
        }
    }

    let Some(data_obj) = response.get("data").and_then(|d| d.as_object()) else {
        return (Vec::new(), errors);
    };

    let mut all_prs = Vec::new();

    for (alias, _owner, _name, slug) in repo_queries {
        let Some(repo_value) = data_obj.get(&alias) else {
            errors.push(format!("{slug}: missing repository data"));
            continue;
        };

        if repo_value.is_null() {
            errors.push(format!("{slug}: repository not found"));
            continue;
        }

        let repo_slug = repo_value
            .get("nameWithOwner")
            .and_then(|v| v.as_str())
            .unwrap_or(&slug)
            .to_string();

        let Some(pr_nodes) = repo_value
            .get("pullRequests")
            .and_then(|prs| prs.get("nodes"))
            .and_then(|nodes| nodes.as_array())
        else {
            continue;
        };

        for pr in pr_nodes {
            let Some(number) = pr.get("number").and_then(|n| n.as_u64()) else {
                errors.push(format!("{repo_slug}: missing PR number"));
                continue;
            };

            let title = pr
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let state = pr
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let url = pr
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let head_branch = pr
                .get("headRefName")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let body = pr
                .get("body")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            let mut rollup_option = {
                let rollups = extract_check_rollups(pr);
                if rollups.is_empty() {
                    None
                } else {
                    Some(rollups)
                }
            };

            let checks = aggregate_check_status(&rollup_option);
            let check_rollup_vec = rollup_option.take().unwrap_or_default();
            let check_runs = check_rollup_vec
                .iter()
                .filter(|c| !c.name.is_empty())
                .map(|c| CheckRun {
                    name: c.name.clone(),
                    status: check_run_status(c),
                    started_at: c.started_at.clone(),
                    completed_at: c.completed_at.clone(),
                })
                .collect();

            all_prs.push(PrInfo {
                number,
                title,
                state,
                checks,
                check_runs,
                url,
                head_branch,
                repo_slug: repo_slug.clone(),
                body,
            });
        }
    }

    (all_prs, errors)
}

/// Create a pull request using `gh pr create` and return the PR URL.
pub async fn create_pr(repo_path: &Path, title: &str, body: &str) -> Result<String> {
    let output = Command::new("gh")
        .args(["pr", "create", "--title", title, "--body", body])
        .current_dir(repo_path)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(eyre!(
            "gh pr create failed: {}",
            if stderr.is_empty() {
                "unknown error"
            } else {
                &stderr
            }
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn extract_check_rollups(pr_node: &Value) -> Vec<GhCheckRollup> {
    let Some(contexts) = pr_node
        .get("statusCheckRollup")
        .and_then(|rollup| rollup.get("nodes"))
        .and_then(|nodes| nodes.as_array())
        .and_then(|nodes| nodes.first())
        .and_then(|node| {
            node.get("commit").and_then(|commit| {
                commit.get("statusCheckRollup").and_then(|status| {
                    status.get("contexts").and_then(|ctx| ctx.get("nodes"))
                })
            })
        })
        .and_then(|nodes| nodes.as_array())
    else {
        return Vec::new();
    };

    let mut rollups = Vec::new();
    for context in contexts {
        let typename = context
            .get("__typename")
            .and_then(|t| t.as_str())
            .unwrap_or_default();
        if typename != "CheckRun" {
            continue;
        }
        let name = context
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if name.is_empty() {
            continue;
        }
        rollups.push(GhCheckRollup {
            name,
            status: context
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            conclusion: context
                .get("conclusion")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            started_at: context
                .get("startedAt")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            completed_at: context
                .get("completedAt")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
        });
    }

    rollups
}

fn aggregate_check_status(rollup: &Option<Vec<GhCheckRollup>>) -> CheckStatus {
    let Some(checks) = rollup else {
        return CheckStatus::Pass;
    };
    if checks.is_empty() {
        return CheckStatus::Pass;
    }
    let any_fail = checks.iter().any(|c| {
        c.conclusion
            .as_deref()
            .map(|s| s.eq_ignore_ascii_case("failure") || s.eq_ignore_ascii_case("cancelled"))
            .unwrap_or(false)
    });
    if any_fail {
        return CheckStatus::Fail;
    }
    let any_pending = checks
        .iter()
        .any(|c| !c.status.eq_ignore_ascii_case("completed"));
    if any_pending {
        return CheckStatus::Pending;
    }
    CheckStatus::Pass
}

pub async fn get_pr_events(repo_path: &Path, pr_number: u64) -> Result<Vec<Event>> {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            &pr_number.to_string(),
            "--json",
            "createdAt,mergedAt,closedAt,state,reviews,statusCheckRollup",
        ])
        .current_dir(repo_path)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(eyre!(
            "gh pr view failed: {}",
            if stderr.is_empty() {
                "unknown error"
            } else {
                &stderr
            }
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail: GhPrDetail = serde_json::from_str(&stdout)?;

    let mut events = Vec::new();

    events.push(Event {
        at: detail.created_at.clone(),
        source: EventSource::GitHub,
        level: EventLevel::Info,
        title: "PR opened".to_string(),
        detail: Some(format!("#{pr_number}")),
    });

    if let Some(merged_at) = detail.merged_at.clone() {
        events.push(Event {
            at: merged_at,
            source: EventSource::GitHub,
            level: EventLevel::Success,
            title: "PR merged".to_string(),
            detail: Some(format!("#{pr_number}")),
        });
    } else if let Some(closed_at) = detail.closed_at.clone() {
        events.push(Event {
            at: closed_at,
            source: EventSource::GitHub,
            level: EventLevel::Warning,
            title: "PR closed".to_string(),
            detail: Some(format!("#{pr_number}")),
        });
    }

    if let Some(reviews) = detail.reviews {
        for review in reviews {
            let Some(at) = review.submitted_at else {
                continue;
            };
            let (level, title) = match review.state.to_uppercase().as_str() {
                "APPROVED" => (EventLevel::Success, "Review approved".to_string()),
                "CHANGES_REQUESTED" => (EventLevel::Error, "Changes requested".to_string()),
                "COMMENTED" => (EventLevel::Info, "Review commented".to_string()),
                _ => (EventLevel::Neutral, format!("Review {}", review.state)),
            };
            let detail = review
                .author
                .as_ref()
                .map(|author| format!("by {}", author.login.clone()));
            events.push(Event {
                at,
                source: EventSource::GitHub,
                level,
                title,
                detail,
            });
        }
    }

    if let Some(checks) = detail.status_check_rollup {
        for check in checks {
            let Some(at) = check.completed_at else {
                continue;
            };
            let conclusion = check.conclusion.unwrap_or_else(|| check.status.clone());
            let upper = conclusion.to_uppercase();
            let level = match upper.as_str() {
                "SUCCESS" | "COMPLETED" => EventLevel::Success,
                "FAILURE" | "FAILED" | "TIMED_OUT" | "CANCELLED" => EventLevel::Error,
                _ => EventLevel::Warning,
            };
            events.push(Event {
                at,
                source: EventSource::GitHub,
                level,
                title: format!("CI: {}", check.name),
                detail: Some(conclusion),
            });
        }
    }

    events.sort_by(|a, b| b.at.cmp(&a.at));
    Ok(events)
}

#[derive(Deserialize)]
struct GhPrDetail {
    #[serde(rename = "createdAt")]
    created_at: String,
    #[serde(rename = "mergedAt")]
    merged_at: Option<String>,
    #[serde(rename = "closedAt")]
    closed_at: Option<String>,
    state: String,
    reviews: Option<Vec<GhReview>>,
    #[serde(rename = "statusCheckRollup")]
    status_check_rollup: Option<Vec<GhCheckRollup>>,
}

#[derive(Deserialize)]
struct GhReview {
    #[serde(rename = "submittedAt")]
    submitted_at: Option<String>,
    state: String,
    author: Option<GhAuthor>,
}

#[derive(Deserialize)]
struct GhAuthor {
    login: String,
}

#[derive(Deserialize)]
struct GhCheckRollup {
    #[serde(default)]
    name: String,
    conclusion: Option<String>,
    #[serde(default)]
    status: String,
    #[serde(rename = "startedAt")]
    started_at: Option<String>,
    #[serde(rename = "completedAt")]
    completed_at: Option<String>,
}

fn check_run_status(c: &GhCheckRollup) -> CheckStatus {
    let is_fail = c
        .conclusion
        .as_deref()
        .map(|s| s.eq_ignore_ascii_case("failure") || s.eq_ignore_ascii_case("cancelled"))
        .unwrap_or(false);
    if is_fail {
        return CheckStatus::Fail;
    }
    if !c.status.eq_ignore_ascii_case("completed") {
        return CheckStatus::Pending;
    }
    CheckStatus::Pass
}
