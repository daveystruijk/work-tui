use std::path::Path;

use color_eyre::{eyre::eyre, Result};
use tokio::process::Command;

pub struct BranchCheckoutResult {
    pub branch_name: String,
    pub reused_existing: bool,
}

pub fn slugify(summary: &str) -> String {
    let mut slug = String::with_capacity(summary.len());
    let mut last_was_dash = false;

    for ch in summary.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
            continue;
        }

        if last_was_dash {
            continue;
        }

        slug.push('-');
        last_was_dash = true;
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.len() > 60 {
        slug.truncate(60);

        while slug.ends_with('-') {
            slug.pop();
        }
    }

    slug
}

pub fn format_branch_name(issue_key: &str, slug: &str) -> String {
    if slug.is_empty() {
        return issue_key.to_string();
    }

    format!("{issue_key}-{slug}")
}

/// Check if a repo's working tree is clean (no uncommitted changes).
pub async fn is_clean(repo_path: &Path) -> Result<bool> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_path)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(eyre!(
            "git status failed: {}",
            if stderr.is_empty() {
                "unknown error"
            } else {
                &stderr
            }
        ));
    }

    Ok(output.stdout.is_empty())
}

/// Stage all changes and create a commit with the given message.
pub async fn commit_all(repo_path: &Path, message: &str) -> Result<()> {
    let add_output = Command::new("git")
        .args(["add", "-A"])
        .current_dir(repo_path)
        .output()
        .await?;

    if !add_output.status.success() {
        let stderr = String::from_utf8_lossy(&add_output.stderr)
            .trim()
            .to_string();
        return Err(eyre!(
            "git add failed: {}",
            if stderr.is_empty() {
                "unknown error"
            } else {
                &stderr
            }
        ));
    }

    let commit_output = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(repo_path)
        .output()
        .await?;

    if !commit_output.status.success() {
        let stderr = String::from_utf8_lossy(&commit_output.stderr)
            .trim()
            .to_string();
        return Err(eyre!(
            "git commit failed: {}",
            if stderr.is_empty() {
                "unknown error"
            } else {
                &stderr
            }
        ));
    }

    Ok(())
}

/// Fetch from origin in the given repo.
pub async fn fetch_origin(repo_path: &Path) -> Result<()> {
    let output = Command::new("git")
        .args(["fetch", "origin"])
        .current_dir(repo_path)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(eyre!(
            "git fetch origin failed: {}",
            if stderr.is_empty() {
                "unknown error"
            } else {
                &stderr
            }
        ));
    }

    Ok(())
}

/// Create a new branch off origin/main in the given repo.
pub async fn create_branch_from_origin_main(
    repo_path: &Path,
    issue_key: &str,
    summary: &str,
) -> Result<BranchCheckoutResult> {
    let slug = slugify(summary);
    let branch_name = format_branch_name(issue_key, &slug);

    let output = Command::new("git")
        .args(["checkout", "-b", &branch_name, "origin/main"])
        .current_dir(repo_path)
        .output()
        .await?;

    if output.status.success() {
        return Ok(BranchCheckoutResult {
            branch_name,
            reused_existing: false,
        });
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stderr_lower = stderr.to_lowercase();

    if stderr_lower.contains("already exists") {
        let branch_ref = format!("refs/heads/{branch_name}");
        let existing_rev = Command::new("git")
            .args(["rev-parse", &branch_ref])
            .current_dir(repo_path)
            .output()
            .await?;

        if !existing_rev.status.success() {
            let rev_stderr = String::from_utf8_lossy(&existing_rev.stderr)
                .trim()
                .to_string();
            return Err(eyre!(
                "{}",
                if rev_stderr.is_empty() {
                    "git rev-parse failed".to_string()
                } else {
                    rev_stderr
                }
            ));
        }

        let origin_rev = Command::new("git")
            .args(["rev-parse", "origin/main"])
            .current_dir(repo_path)
            .output()
            .await?;

        if !origin_rev.status.success() {
            let rev_stderr = String::from_utf8_lossy(&origin_rev.stderr)
                .trim()
                .to_string();
            return Err(eyre!(
                "{}",
                if rev_stderr.is_empty() {
                    "git rev-parse failed".to_string()
                } else {
                    rev_stderr
                }
            ));
        }

        let existing_hash = String::from_utf8_lossy(&existing_rev.stdout)
            .trim()
            .to_string();
        let origin_hash = String::from_utf8_lossy(&origin_rev.stdout)
            .trim()
            .to_string();

        if existing_hash == origin_hash {
            let checkout_existing = Command::new("git")
                .args(["checkout", &branch_name])
                .current_dir(repo_path)
                .output()
                .await?;

            if checkout_existing.status.success() {
                return Ok(BranchCheckoutResult {
                    branch_name,
                    reused_existing: true,
                });
            }

            let checkout_stderr = String::from_utf8_lossy(&checkout_existing.stderr)
                .trim()
                .to_string();
            return Err(eyre!(
                "{}",
                if checkout_stderr.is_empty() {
                    "git checkout failed".to_string()
                } else {
                    checkout_stderr
                }
            ));
        }

        return Err(eyre!(format!(
            "Branch {branch_name} already exists and differs from origin/main"
        )));
    }

    Err(eyre!(
        "{}",
        if stderr.is_empty() {
            "git checkout -b failed".to_string()
        } else {
            stderr
        }
    ))
}

/// Get the current branch in a specific repo directory.
pub async fn current_branch_in(repo_path: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(repo_path)
        .output()
        .await?;

    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(eyre!(
        "{}",
        if stderr.is_empty() {
            "git rev-parse failed".to_string()
        } else {
            stderr
        }
    ))
}

pub async fn current_branch() -> Result<String> {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("HEAD")
        .output()
        .await?;

    if output.status.success() {
        let branch_name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Ok(branch_name);
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let message = if stderr.is_empty() {
        "git rev-parse failed".into()
    } else {
        stderr
    };

    Err(eyre!(message))
}

/// Push the current branch to origin.
pub async fn push_branch(repo_path: &Path, branch: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["push", "-u", "origin", branch])
        .current_dir(repo_path)
        .output()
        .await?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(eyre!(
        "git push failed: {}",
        if stderr.is_empty() {
            "unknown error"
        } else {
            &stderr
        }
    ))
}

