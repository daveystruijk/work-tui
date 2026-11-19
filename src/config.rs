use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

#[derive(Clone)]
pub struct Config {
    pub jira_url: String,
    pub jira_email: String,
    pub jira_token: String,
    pub repos_dir: PathBuf,
    /// Where the little this tool remembers between runs is kept.
    pub state_dir: PathBuf,
}

/// The Jira project whose board is shown.
pub const PROJECT: &str = "INI";
/// Command that shows the changes of a revision range. Not `git difftool`, which opens the
/// viewer once per changed file instead of once for the whole range.
pub const DIFF_VIEWER: &str = "hunk diff";
/// Shell command that keeps showing everything the branch of the directory it runs in changes
/// against main: commits, staged, unstaged and untracked files alike. Against the merge-base
/// rather than `origin/main`, so what lands on main since does not show up as reverted.
pub const DIFF_WATCH: &str = "hunk diff --watch \"$(git merge-base origin/main HEAD)\"";
/// Command that receives the prompt as its last argument.
pub const AGENT: &str = "claude";
/// Settings for the agent sessions this tool starts, on top of the user's own: hooks that ring
/// the terminal bell when the agent finishes a turn or needs an answer, so tmux marks the tab,
/// and that keep a pane option set while the agent is working (see `actions::panes`).
pub const AGENT_SETTINGS: &str = include_str!("../agent-settings.json");
/// Shell command with `{repo}` and `{branch}`; prints the worktree path as its last line.
pub const WORKTREE_ADD: &str = "momo worktree add {repo} {branch}";
/// Shell command with `{workspace}`; removes the workspace, refusing when a worktree in it has
/// uncommitted changes.
pub const WORKTREE_REMOVE: &str = "momo worktree rm {workspace}";
/// Shell command printing one `<workspace> <url>` per line.
pub const WORKTREE_LIST: &str = "momo worktree ls";

impl Config {
    pub fn from_env() -> Result<Self> {
        let required = |name: &str| env::var(name).with_context(|| format!("{name} is not set"));
        let repos_dir = PathBuf::from(required("REPOS_DIR")?);
        if !repos_dir.is_dir() {
            bail!("REPOS_DIR ({}) is not a directory", repos_dir.display());
        }
        Ok(Self {
            jira_url: required("JIRA_URL")?.trim_end_matches('/').to_owned(),
            jira_email: required("JIRA_EMAIL")?,
            jira_token: required("JIRA_API_TOKEN")?,
            repos_dir,
            state_dir: PathBuf::from(required("HOME")?).join(".local/state/work"),
        })
    }
}
