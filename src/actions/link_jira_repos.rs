//! **Link Jira Repos** — discovers PRs via GitHub search and labels unlinked Jira issues.
//!
//! Uses a single GitHub GraphQL search query to find open PRs across the org
//! whose branch names match Jira issue keys. When a PR's repository matches a
//! local directory in `REPOS_DIR`, the repo name is added as a label on the
//! Jira issue so the normal PR-fetching flow picks it up.
//!
//! # Channel messages produced
//! - [`Message::ActionStarted`] / [`Message::ActionFinished`]
//! - [`Message::AutoLabeled`]

use tokio::sync::mpsc;

use super::Message;
use crate::apis::jira::JiraClient;
use crate::repos;

/// An issue that has no repo label match: `(issue_key, current_labels)`.
pub type UnlinkedIssue = (String, Vec<String>);

/// A discovered PR from GitHub search: `(repo_name, branch_name)`.
#[derive(Debug)]
struct DiscoveredPr {
    repo_name: String,
    branch: String,
}

/// Spawn repo linking for unlinked issues.
///
/// Searches GitHub for open PRs across the org, matches branches to issue keys,
/// and labels issues whose PR repo matches a local directory.
pub fn spawn(
    tx: mpsc::UnboundedSender<Message>,
    client: JiraClient,
    unlinked: Vec<UnlinkedIssue>,
    repo_normalized_names: Vec<(String, String)>,
    github_org: String,
) {
    if unlinked.is_empty() || repo_normalized_names.is_empty() || github_org.is_empty() {
        return;
    }

    super::spawn_action(tx, "link_jira_repos", "Linking repos", |tx| async move {
        let discovered = match search_org_prs(&github_org).await {
            Ok(prs) => prs,
            Err(_) => return,
        };

        for (issue_key, current_labels) in &unlinked {
            let key_lower = issue_key.to_lowercase();
            let matched_pr = discovered
                .iter()
                .find(|pr| pr.branch.to_lowercase().starts_with(&key_lower));
            let Some(pr) = matched_pr else {
                continue;
            };

            let pr_normalized = repos::normalize_label(&pr.repo_name);
            let already_labeled = current_labels
                .iter()
                .any(|l| repos::normalize_label(l) == pr_normalized);
            if already_labeled {
                continue;
            }

            let local_match = repo_normalized_names
                .iter()
                .find(|(_, normalized)| *normalized == pr_normalized);
            let Some((original_label, _)) = local_match else {
                continue;
            };

            let mut new_labels = current_labels.clone();
            new_labels.push(original_label.clone());
            let result = client.update_labels(issue_key, &new_labels).await;
            let _ = tx.send(Message::AutoLabeled(issue_key.clone(), result.map(|_| ())));
        }
    });
}

/// Search GitHub for all open PRs in the given org via a single GraphQL query.
async fn search_org_prs(org: &str) -> color_eyre::Result<Vec<DiscoveredPr>> {
    let search_query = format!("type:pr state:open org:{org}");
    let graphql = format!(
        r#"{{
            search(query: "{search_query}", type: ISSUE, first: 100) {{
                nodes {{
                    ... on PullRequest {{
                        headRefName
                        repository {{ name }}
                    }}
                }}
            }}
        }}"#,
    );

    let output = tokio::process::Command::new("gh")
        .args(["api", "graphql", "-f", &format!("query={graphql}")])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(color_eyre::eyre::eyre!("GitHub search failed: {stderr}"));
    }

    let response: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let Some(nodes) = response
        .pointer("/data/search/nodes")
        .and_then(|v| v.as_array())
    else {
        return Ok(Vec::new());
    };

    let mut results = Vec::new();
    for node in nodes {
        let branch = node
            .get("headRefName")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let repo_name = node
            .pointer("/repository/name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if branch.is_empty() || repo_name.is_empty() {
            continue;
        }
        results.push(DiscoveredPr {
            repo_name: repo_name.to_string(),
            branch: branch.to_string(),
        });
    }
    Ok(results)
}
