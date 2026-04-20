#![allow(dead_code)]

use std::path::Path;

use color_eyre::{eyre::eyre, Result};
use serde::Deserialize;
use serde_json::Value;
use tokio::process::Command;

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
    pub details_url: String,
    pub summary: String,
    pub text: String,
    pub log_excerpt: String,
    pub steps: Vec<CheckStep>,
    pub annotations: Vec<CheckAnnotation>,
}

/// A single step within a CI check run.
#[derive(Debug, Clone)]
pub struct CheckStep {
    pub name: String,
    pub status: CheckStatus,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

/// An annotation on a CI check run (e.g. error/warning from GitHub Actions).
#[derive(Debug, Clone)]
pub struct CheckAnnotation {
    pub message: String,
    pub title: String,
    pub path: String,
    pub annotation_level: String,
}

#[derive(Debug, Clone)]
pub struct PrComment {
    pub author: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct ReviewComment {
    pub author: String,
    pub body: String,
    pub created_at: String,
    pub path: Option<String>,
    pub line: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ReviewThread {
    pub is_resolved: bool,
    pub resolved_by: Option<String>,
    pub comments: Vec<ReviewComment>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MergeableState {
    Mergeable,
    Conflicting,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct PrInfo {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub is_draft: bool,
    pub checks: CheckStatus,
    pub check_runs: Vec<CheckRun>,
    pub url: String,
    pub head_branch: String,
    /// The repo slug (owner/repo) this PR belongs to
    pub repo_slug: String,
    pub comments: Vec<PrComment>,
    pub review_threads: Vec<ReviewThread>,
    pub changed_files: Option<u64>,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub mergeable: Option<MergeableState>,
}

#[derive(Debug, Clone)]
pub struct PrDetail {
    pub checks: CheckStatus,
    pub check_runs: Vec<CheckRun>,
    pub comments: Vec<PrComment>,
    pub review_threads: Vec<ReviewThread>,
    pub changed_files: Option<u64>,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub mergeable: Option<MergeableState>,
}

impl PrInfo {
    pub fn apply_detail(&mut self, detail: PrDetail) {
        self.checks = detail.checks;
        self.check_runs = detail.check_runs;
        self.comments = detail.comments;
        self.review_threads = detail.review_threads;
        self.changed_files = detail.changed_files;
        self.additions = detail.additions;
        self.deletions = detail.deletions;
        self.mergeable = detail.mergeable;
    }

    /// Copy detail-enriched fields from a previously loaded PR.
    /// Used to carry forward cached CI logs across auto-refresh cycles
    /// when the check run set hasn't changed.
    pub fn apply_detail_from(&mut self, old: &PrInfo) {
        self.check_runs = old.check_runs.clone();
        self.comments = old.comments.clone();
        self.review_threads = old.review_threads.clone();
        self.changed_files = old.changed_files;
        self.additions = old.additions;
        self.deletions = old.deletions;
        self.mergeable = old.mergeable.clone();
    }

    pub fn latest_failed_check(&self) -> Option<&CheckRun> {
        self.check_runs
            .iter()
            .filter(|run| run.status == CheckStatus::Fail)
            .max_by_key(|run| {
                run.completed_at
                    .as_deref()
                    .or(run.started_at.as_deref())
                    .unwrap_or_default()
            })
    }
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
    #[serde(rename = "isDraft", default)]
    is_draft: bool,
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
            "number,title,state,url,headRefName,isDraft,statusCheckRollup",
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
                    details_url: c.details_url.clone(),
                    summary: String::new(),
                    text: String::new(),
                    log_excerpt: String::new(),
                    steps: Vec::new(),
                    annotations: Vec::new(),
                })
                .collect();
            PrInfo {
                number: pr.number,
                title: pr.title,
                state: pr.state,
                is_draft: pr.is_draft,
                checks,
                check_runs,
                url: pr.url,
                head_branch: pr.head_ref_name,
                repo_slug: slug.clone(),
                comments: Vec::new(),
                review_threads: Vec::new(),
                changed_files: None,
                additions: None,
                deletions: None,
                mergeable: None,
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
        repo_queries.push((
            format!("repo_{idx}"),
            owner.to_string(),
            name.to_string(),
            slug.clone(),
        ));
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
                        isDraft
                        mergeable
                        changedFiles
                        additions
                        deletions
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
                                                    detailsUrl
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

    if let Some(graph_errors) = response.get("errors").and_then(|errs| errs.as_array()) {
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
            let is_draft = pr.get("isDraft").and_then(|v| v.as_bool()).unwrap_or(false);
            let changed_files = pr.get("changedFiles").and_then(|v| v.as_u64());
            let additions = pr.get("additions").and_then(|v| v.as_u64());
            let deletions = pr.get("deletions").and_then(|v| v.as_u64());
            let mergeable = parse_mergeable_state(pr);
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
                    details_url: c.details_url.clone(),
                    summary: String::new(),
                    text: String::new(),
                    log_excerpt: String::new(),
                    steps: Vec::new(),
                    annotations: Vec::new(),
                })
                .collect();

            all_prs.push(PrInfo {
                number,
                title,
                state,
                is_draft,
                checks,
                check_runs,
                url,
                head_branch,
                repo_slug: repo_slug.clone(),
                comments: Vec::new(),
                review_threads: Vec::new(),
                changed_files,
                additions,
                deletions,
                mergeable,
            });
        }
    }

    (all_prs, errors)
}

pub async fn fetch_pr_detail(repo_slug: &str, pr_number: u64) -> Result<PrDetail> {
    let Some((owner, name)) = repo_slug.split_once('/') else {
        return Err(eyre!("invalid repo slug: {repo_slug}"));
    };

    let query = format!(
        r#"query {{
            repository(owner: "{owner}", name: "{name}") {{
                pullRequest(number: {pr_number}) {{
                    mergeable
                    changedFiles
                    additions
                    deletions
                    comments(first: 20, orderBy: {{field: UPDATED_AT, direction: DESC}}) {{
                        nodes {{
                            body
                            createdAt
                            updatedAt
                            url
                            author {{
                                login
                            }}
                        }}
                    }}
                    reviewThreads(first: 20) {{
                        nodes {{
                            isResolved
                            resolvedBy {{
                                login
                            }}
                            comments(first: 20) {{
                                nodes {{
                                    body
                                    createdAt
                                    path
                                    line
                                    author {{
                                        login
                                    }}
                                }}
                            }}
                        }}
                    }}
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
                                                detailsUrl
                                                steps(first: 50) {{
                                                    nodes {{
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
            }}
        }}"#,
    );

    let output = Command::new("gh")
        .args(["api", "graphql", "-f", &format!("query={query}")])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(eyre!(
            "gh api graphql failed: {}",
            if stderr.is_empty() {
                "unknown error"
            } else {
                &stderr
            }
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let response: Value = serde_json::from_str(&stdout)?;

    if let Some(graph_errors) = response.get("errors").and_then(|value| value.as_array()) {
        let messages = graph_errors
            .iter()
            .filter_map(|error| error.get("message").and_then(|value| value.as_str()))
            .collect::<Vec<_>>();
        if !messages.is_empty() {
            return Err(eyre!(messages.join("; ")));
        }
    }

    let pr_node = response
        .get("data")
        .and_then(|value| value.get("repository"))
        .and_then(|value| value.get("pullRequest"))
        .ok_or_else(|| eyre!("missing pull request detail for {repo_slug}#{pr_number}"))?;

    let mut rollup_option = {
        let rollups = extract_check_rollups(pr_node);
        if rollups.is_empty() {
            None
        } else {
            Some(rollups)
        }
    };

    let checks = aggregate_check_status(&rollup_option);
    let check_runs: Vec<CheckRun> = rollup_option
        .take()
        .unwrap_or_default()
        .into_iter()
        .filter(|check| !check.name.is_empty())
        .map(|check| {
            let status = check_run_status(&check);
            let steps = check
                .steps
                .into_iter()
                .map(|step| CheckStep {
                    name: step.name,
                    status: check_step_status(&step.status, step.conclusion.as_deref()),
                    started_at: step.started_at,
                    completed_at: step.completed_at,
                })
                .collect();
            CheckRun {
                name: check.name,
                status,
                started_at: check.started_at,
                completed_at: check.completed_at,
                details_url: check.details_url,
                summary: String::new(),
                text: String::new(),
                log_excerpt: String::new(),
                steps,
                annotations: Vec::new(),
            }
        })
        .collect();

    let changed_files = pr_node.get("changedFiles").and_then(|v| v.as_u64());
    let additions = pr_node.get("additions").and_then(|v| v.as_u64());
    let deletions = pr_node.get("deletions").and_then(|v| v.as_u64());
    let mergeable = parse_mergeable_state(pr_node);

    Ok(PrDetail {
        checks,
        check_runs,
        comments: extract_issue_comments(pr_node),
        review_threads: extract_review_threads(pr_node),
        changed_files,
        additions,
        deletions,
        mergeable,
    })
}

fn parse_job_id_from_details_url(details_url: &str) -> Option<u64> {
    let marker = "/actions/runs/";
    let run_index = details_url.find(marker)? + marker.len();
    let after_run = &details_url[run_index..];
    let job_marker = "/job/";
    let job_index = after_run.find(job_marker)? + job_marker.len();
    let job_part = &after_run[job_index..];
    let digits: String = job_part
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

async fn fetch_job_log_text(repo_slug: &str, job_id: u64) -> Option<String> {
    let output = Command::new("gh")
        .args([
            "run",
            "view",
            "--repo",
            repo_slug,
            "--job",
            &job_id.to_string(),
            "--log",
        ])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    sanitize_log_excerpt(&stdout)
}

/// Strip ANSI sequences, `gh` log prefixes and timestamps, then extract
/// `##[error]` lines with surrounding context.
fn sanitize_log_excerpt(output: &str) -> Option<String> {
    let without_ansi = strip_ansi_sequences(output);
    let lines: Vec<&str> = without_ansi
        .lines()
        .map(str::trim_end)
        .filter(|line| line.contains('\t'))
        .map(strip_gh_log_prefix)
        .filter(|line| !line.trim().is_empty())
        .collect();

    if lines.is_empty() {
        return None;
    }

    Some(extract_error_context(&lines).join("\n"))
}

/// Extract `##[error]` lines and surrounding context from a job log.
///
/// Returns up to 5 lines of context before each error line, the error line
/// itself, and up to 2 lines after. Falls back to the last 50 lines if no
/// `##[error]` markers are found.
fn extract_error_context<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    let error_indices: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains("##[error]"))
        .map(|(i, _)| i)
        .collect();

    if error_indices.is_empty() {
        let start = lines.len().saturating_sub(50);
        return lines[start..].to_vec();
    }

    let mut included = vec![false; lines.len()];
    for &idx in &error_indices {
        let context_start = idx.saturating_sub(5);
        let context_end = (idx + 3).min(lines.len());
        for flag in &mut included[context_start..context_end] {
            *flag = true;
        }
    }

    let mut result = Vec::new();
    let mut in_block = false;
    for (i, line) in lines.iter().enumerate() {
        if included[i] {
            if !in_block && !result.is_empty() {
                result.push("...");
            }
            result.push(line);
            in_block = true;
        } else {
            in_block = false;
        }
    }
    result
}

/// Strip the `<job>\t<step>\t` prefix that `gh run view --log` adds.
/// Format: "Lint and Test / test\tUNKNOWN STEP\t<content>"
fn strip_gh_log_prefix(line: &str) -> &str {
    let mut remainder = line;
    for _ in 0..2 {
        match remainder.find('\t') {
            Some(pos) => remainder = &remainder[pos + 1..],
            None => return line,
        }
    }
    strip_gh_log_timestamp(remainder)
}

/// Strip the ISO 8601 timestamp prefix that GitHub Actions adds to each log line.
/// Format: "2024-01-15T10:30:45.1234567Z <content>"
fn strip_gh_log_timestamp(line: &str) -> &str {
    // Timestamps are exactly 28 chars: "YYYY-MM-DDTHH:MM:SS.fffffffZ"
    if line.len() >= 29
        && line.as_bytes()[4] == b'-'
        && line.as_bytes()[10] == b'T'
        && line.as_bytes()[27] == b'Z'
        && line.as_bytes()[28] == b' '
    {
        return &line[29..];
    }
    line
}

fn strip_ansi_sequences(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && matches!(chars.peek(), Some('[')) {
            chars.next();
            while let Some(next) = chars.next() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        output.push(ch);
    }
    output
}

pub async fn fetch_check_run_logs(repo_slug: &str, check_runs: &[CheckRun]) -> Result<Vec<String>> {
    let mut logs = Vec::with_capacity(check_runs.len());
    for run in check_runs {
        let Some(job_id) = parse_job_id_from_details_url(&run.details_url) else {
            logs.push(String::new());
            continue;
        };

        logs.push(
            fetch_job_log_text(repo_slug, job_id)
                .await
                .unwrap_or_default(),
        );
    }
    Ok(logs)
}

/// Create a pull request using `gh pr create`, enable auto-merge, and return the PR URL.
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

    let pr_url = String::from_utf8_lossy(&output.stdout).trim().to_string();

    let auto_merge = Command::new("gh")
        .args(["pr", "merge", &pr_url, "--auto", "--merge"])
        .current_dir(repo_path)
        .output()
        .await?;
    if !auto_merge.status.success() {
        let stderr = String::from_utf8_lossy(&auto_merge.stderr).trim().to_string();
        tracing::warn!("Failed to enable auto-merge: {stderr}");
    }

    Ok(pr_url)
}

fn parse_mergeable_state(pr_node: &Value) -> Option<MergeableState> {
    pr_node
        .get("mergeable")
        .and_then(|v| v.as_str())
        .map(|s| match s {
            "MERGEABLE" => MergeableState::Mergeable,
            "CONFLICTING" => MergeableState::Conflicting,
            _ => MergeableState::Unknown,
        })
}

fn extract_check_rollups(pr_node: &Value) -> Vec<GhCheckRollup> {
    let Some(contexts) = pr_node
        .get("statusCheckRollup")
        .and_then(|rollup| rollup.get("nodes"))
        .and_then(|nodes| nodes.as_array())
        .and_then(|nodes| nodes.first())
        .and_then(|node| {
            node.get("commit").and_then(|commit| {
                commit
                    .get("statusCheckRollup")
                    .and_then(|status| status.get("contexts").and_then(|ctx| ctx.get("nodes")))
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
            details_url: context
                .get("detailsUrl")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            summary: context
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            text: context
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            steps: context
                .get("steps")
                .and_then(|v| v.get("nodes"))
                .and_then(|v| serde_json::from_value::<Vec<GhCheckStep>>(v.clone()).ok())
                .unwrap_or_default(),
            annotations: context
                .get("annotations")
                .and_then(|v| v.get("nodes"))
                .and_then(|v| serde_json::from_value::<Vec<GhCheckAnnotation>>(v.clone()).ok())
                .unwrap_or_default(),
        });
    }

    rollups
}

fn extract_issue_comments(pr_node: &Value) -> Vec<PrComment> {
    let Some(comment_nodes) = pr_node
        .get("comments")
        .and_then(|comments| comments.get("nodes"))
        .and_then(|nodes| nodes.as_array())
    else {
        return Vec::new();
    };

    comment_nodes
        .iter()
        .map(|comment| PrComment {
            author: extract_author_login(comment.get("author")),
            body: comment
                .get("body")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            created_at: comment
                .get("createdAt")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            updated_at: comment
                .get("updatedAt")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            url: comment
                .get("url")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
        })
        .collect()
}

fn extract_review_threads(pr_node: &Value) -> Vec<ReviewThread> {
    let Some(thread_nodes) = pr_node
        .get("reviewThreads")
        .and_then(|threads| threads.get("nodes"))
        .and_then(|nodes| nodes.as_array())
    else {
        return Vec::new();
    };

    thread_nodes
        .iter()
        .map(|thread| ReviewThread {
            is_resolved: thread
                .get("isResolved")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            resolved_by: thread
                .get("resolvedBy")
                .map(|value| extract_author_login(Some(value)))
                .filter(|login| !login.is_empty()),
            comments: thread
                .get("comments")
                .and_then(|comments| comments.get("nodes"))
                .and_then(|nodes| nodes.as_array())
                .into_iter()
                .flatten()
                .map(|comment| ReviewComment {
                    author: extract_author_login(comment.get("author")),
                    body: comment
                        .get("body")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    created_at: comment
                        .get("createdAt")
                        .and_then(|value| value.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    path: comment
                        .get("path")
                        .and_then(|value| value.as_str())
                        .map(ToString::to_string),
                    line: comment.get("line").and_then(|value| value.as_i64()),
                })
                .collect(),
        })
        .filter(|thread| !thread.comments.is_empty())
        .collect()
}

fn extract_author_login(author: Option<&Value>) -> String {
    author
        .and_then(|value| value.get("login"))
        .and_then(|value| value.as_str())
        .unwrap_or("github")
        .to_string()
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

fn check_step_status(status: &str, conclusion: Option<&str>) -> CheckStatus {
    let is_fail = conclusion
        .map(|s| s.eq_ignore_ascii_case("failure") || s.eq_ignore_ascii_case("cancelled"))
        .unwrap_or(false);
    if is_fail {
        return CheckStatus::Fail;
    }
    if !status.eq_ignore_ascii_case("completed") {
        return CheckStatus::Pending;
    }
    CheckStatus::Pass
}

#[derive(Deserialize)]
struct GhCheckStep {
    #[serde(default)]
    name: String,
    #[serde(default)]
    status: String,
    conclusion: Option<String>,
    #[serde(rename = "startedAt")]
    started_at: Option<String>,
    #[serde(rename = "completedAt")]
    completed_at: Option<String>,
}

#[derive(Deserialize)]
struct GhCheckAnnotation {
    #[serde(rename = "annotationLevel", default)]
    annotation_level: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    path: String,
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
    #[serde(rename = "detailsUrl", default)]
    details_url: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    steps: Vec<GhCheckStep>,
    #[serde(default)]
    annotations: Vec<GhCheckAnnotation>,
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
