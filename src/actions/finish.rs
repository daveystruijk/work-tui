//! **Finish** — pushes changes, creates a PR, and moves the ticket to Review.
//!
//! Performs the full "finish" workflow:
//! 1. Verify the repo working tree is clean
//! 2. Fetch from origin
//! 3. Generate a PR summary via `opencode` (AI-powered)
//! 4. Push the branch to origin
//! 5. Create a pull request via `gh pr create`
//! 6. Transition the Jira issue to "Review" (if available)
//!
//! # Channel messages produced
//! - [`ActionMessage::Progress`] (per-step progress)
//! - [`ActionMessage::Finished`]

use std::path::PathBuf;

use color_eyre::eyre::eyre;
use tokio::process::Command;
use tokio::sync::mpsc;

use super::ActionMessage;
use crate::actions::Progress;
use crate::git;
use crate::apis::github;
use crate::apis::jira::JiraClient;

/// Spawn the finish workflow for a single issue.
pub fn spawn(
    tx: mpsc::UnboundedSender<ActionMessage>,
    client: JiraClient,
    issue_key: String,
    issue_summary: String,
    repo_path: PathBuf,
) {
    tokio::spawn(async move {
        let _ = tx.send(ActionMessage::TaskStarted("Finishing".to_string()));
        let result = run(&tx, &client, &issue_key, &issue_summary, &repo_path).await;
        let _ = tx.send(ActionMessage::TaskFinished("Finishing".to_string()));
        let _ = tx.send(ActionMessage::Finished(result));
    });
}

async fn run(
    tx: &mpsc::UnboundedSender<ActionMessage>,
    client: &JiraClient,
    issue_key: &str,
    issue_summary: &str,
    repo_path: &PathBuf,
) -> color_eyre::Result<String> {
    let branch = git::current_branch_in(repo_path).await?;
    if branch.is_empty() {
        return Err(eyre!("Cannot finish: no branch checked out"));
    }
    if branch == "main" || branch == "master" {
        return Err(eyre!("Cannot finish: on {branch}, not a feature branch"));
    }

    // Step 1: Commit uncommitted changes (if any)
    let _ = tx.send(ActionMessage::Progress(Progress {
        action: "finish",
        message: "Checking working tree...".into(),
        current: 1,
        total: 6,
    }));
    if !git::is_clean(repo_path).await? {
        let commit_message = format!("{issue_key} {issue_summary}");
        git::commit_all(repo_path, &commit_message).await?;
    }

    // Step 2: Fetch origin
    let _ = tx.send(ActionMessage::Progress(Progress {
        action: "finish",
        message: "Fetching origin...".into(),
        current: 2,
        total: 6,
    }));
    git::fetch_origin(repo_path).await?;

    // Step 3: Generate PR summary via opencode
    let _ = tx.send(ActionMessage::Progress(Progress {
        action: "finish",
        message: "Generating PR summary...".into(),
        current: 3,
        total: 6,
    }));
    let pr_title = format!("{issue_key} {issue_summary}");
    let pr_body = generate_pr_summary(repo_path).await?;

    // Step 4: Push branch
    let _ = tx.send(ActionMessage::Progress(Progress {
        action: "finish",
        message: "Pushing branch...".into(),
        current: 4,
        total: 6,
    }));
    git::push_branch(repo_path, &branch).await?;

    // Step 5: Create PR
    let _ = tx.send(ActionMessage::Progress(Progress {
        action: "finish",
        message: "Creating pull request...".into(),
        current: 5,
        total: 6,
    }));
    let pr_url = github::create_pr(repo_path, &pr_title, &pr_body).await?;

    // Step 6: Transition to Review
    let _ = tx.send(ActionMessage::Progress(Progress {
        action: "finish",
        message: "Transitioning to Review...".into(),
        current: 6,
        total: 6,
    }));
    let transitions = client.get_transitions(issue_key).await?;
    let review = transitions
        .into_iter()
        .find(|t| t.name.to_lowercase().contains("review"));
    if let Some(t) = review {
        client.transition_issue(issue_key, &t.id).await?;
    }

    Ok(pr_url)
}

/// Use `opencode` to generate a PR summary from the git diff against main.
async fn generate_pr_summary(repo_path: &PathBuf) -> color_eyre::Result<String> {
    let diff_summary = git::diff_summary(repo_path).await?;
    if diff_summary.is_empty() {
        return Err(eyre!("No commits found between origin/main and HEAD"));
    }

    let prompt = format!(
        "Generate a concise pull request description for these changes. \
         Use markdown with a ## Summary section containing 1-3 bullet points. \
         Only output the markdown, nothing else.\n\n\
         Commits:\n{diff_summary}"
    );

    let output = Command::new("opencode")
        .args(["run", &prompt])
        .current_dir(repo_path)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(eyre!(
            "opencode failed: {}",
            if stderr.is_empty() {
                "unknown error"
            } else {
                &stderr
            }
        ));
    }

    let body = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if body.is_empty() {
        return Err(eyre!("opencode returned empty output"));
    }

    Ok(body)
}
