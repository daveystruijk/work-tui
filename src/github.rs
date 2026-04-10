#![allow(dead_code)]

use std::path::Path;

use color_eyre::{eyre::eyre, Result};
use serde::Deserialize;
use tokio::process::Command;

use crate::events::{Event, EventLevel, EventSource};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckStatus {
    Pending,
    Pass,
    Fail,
}

#[derive(Debug, Clone)]
pub struct PrInfo {
    pub number: u64,
    pub state: String,
    pub checks: CheckStatus,
    pub url: String,
    pub head_branch: String,
    /// The repo slug (owner/repo) this PR belongs to
    pub repo_slug: String,
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
    state: String,
    url: String,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "statusCheckRollup", default)]
    status_check_rollup: Option<Vec<GhCheckRollup>>,
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
            "number,state,url,headRefName,statusCheckRollup",
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
            PrInfo {
                number: pr.number,
                state: pr.state,
                checks,
                url: pr.url,
                head_branch: pr.head_ref_name,
                repo_slug: slug.clone(),
            }
        })
        .collect())
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
    name: String,
    conclusion: Option<String>,
    status: String,
    #[serde(rename = "completedAt")]
    completed_at: Option<String>,
}
