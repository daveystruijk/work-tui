//! The things a key press sets in motion. Each runs on its own thread and blocks until done.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::thread;

use anyhow::{Context as _, Result, bail};

use crate::config::{
    AGENT, AGENT_SETTINGS, Config, DIFF_VIEWER, DIFF_WATCH, PROJECT, WORKTREE_ADD, WORKTREE_LIST,
};
use crate::github;
use crate::jira::Jira;
use crate::model::{BACKLOG, Issue, Person, Pr, issue_key_of};
use crate::shell::{quote, run};

/// What the agent is asked to do with an issue. The files in `prompts/` hold the instructions;
/// the issue and its parent are appended when a session is opened.
#[derive(Clone, Copy)]
pub struct Template {
    instructions: &'static str,
    /// Whether an issue that already has a session gets a new agent for this, next to it.
    starts_new_agent: bool,
}

pub const IMPLEMENT: Template = Template {
    instructions: include_str!("../prompts/implement.md"),
    starts_new_agent: false,
};
/// Instructions for writing a pull request description from a diff on standard input.
const PR_DESCRIPTION: &str = include_str!("../prompts/pr-description.md");
pub const FIX_CI: Template = Template {
    instructions: include_str!("../prompts/fix-ci.md"),
    starts_new_agent: true,
};
const MAX_BRANCH_LENGTH: usize = 60;
const MAX_WINDOW_NAME_LENGTH: usize = 30; // in characters; tabs of several issues have to fit next to each other
/// tmux pane option that holds the issue key of the diff shown in the pane.
const DIFF_PANE_OPTION: &str = "@work_diff";

pub struct Context {
    pub config: Config,
    pub jira: Jira,
}

/// An issue together with everything needed to work on it.
pub struct Target {
    pub issue: Issue,
    /// The issue's parent, when it is among the loaded issues.
    pub parent: Option<Issue>,
    pub pr: Option<Pr>,
    pub repo: String,
    /// The repository was just chosen by hand and has to be recorded on the issue.
    pub add_label: bool,
}

/// Everything that comes from Jira and the local machine; fast enough to show before GitHub answers.
pub struct Issues {
    pub issues: Vec<Issue>,
    pub sections: Vec<String>,
    pub workspaces: HashMap<String, String>,
    pub me: Person,
}

/// The issues on the board up to and including the Review column, plus the backlog.
/// Sections run from Review back to the first column, with the backlog last.
pub fn load_issues(context: &Context) -> Result<Issues> {
    let jira = &context.jira;
    let board = jira.board(PROJECT)?;
    let review = board
        .columns
        .iter()
        .position(|column| column.name.to_lowercase().contains("review"));
    let columns = &board.columns[..review.map_or(board.columns.len(), |index| index + 1)];
    let status_ids: Vec<&str> = columns
        .iter()
        .flat_map(|column| &column.status_ids)
        .map(String::as_str)
        .collect();
    let jql = format!(
        "project = {PROJECT} AND status in ({}) ORDER BY updated DESC",
        status_ids.join(", ")
    );
    thread::scope(|scope| {
        let me = scope.spawn(|| jira.myself());
        let workspaces = scope.spawn(|| workspaces(&context.config));
        let backlog = scope.spawn(|| jira.backlog(board.id));
        let mut issues = jira.search(&jql)?;
        let backlog = backlog.join().expect("backlog lookup panicked")?;
        let column_of = |issue: &Issue| {
            columns
                .iter()
                .find(|column| column.status_ids.contains(&issue.status_id))
                .map_or_else(String::new, |column| column.name.clone())
        };
        let by_key: HashMap<String, Issue> = issues
            .iter()
            .map(|issue| (issue.key.clone(), issue.clone()))
            .collect();
        let with_subtasks: HashSet<String> = issues
            .iter()
            .filter(|issue| issue.is_subtask)
            .filter_map(|issue| issue.parent.as_ref())
            .map(|parent| parent.key.clone())
            .collect();
        for issue in &mut issues {
            let parent = issue
                .parent
                .as_ref()
                .and_then(|parent| by_key.get(&parent.key));
            // Jira reports every subtask as backlog; it is only there when its parent is.
            let in_backlog = match parent.filter(|_| issue.is_subtask) {
                Some(parent) => backlog.contains(&parent.key),
                None => backlog.contains(&issue.key) && !issue.is_subtask,
            };
            issue.section = if in_backlog {
                BACKLOG.to_owned()
            } else {
                column_of(issue)
            };
            issue.listed = !issue.is_epic && !with_subtasks.contains(&issue.key);
            issue.epic = if issue.is_subtask {
                parent.and_then(|parent| parent.parent.clone())
            } else {
                issue.parent.clone()
            };
        }
        let mut sections: Vec<String> = columns
            .iter()
            .rev()
            .map(|column| column.name.clone())
            .collect(); // closest to done first
        sections.push(BACKLOG.to_owned());
        Ok(Issues {
            issues,
            sections,
            workspaces: workspaces.join().expect("workspace listing panicked")?,
            me: me.join().expect("user lookup panicked")?,
        })
    })
}

/// The current GitHub login and the open pull requests of the project, by issue key.
pub fn load_prs(context: &Context, repos: &[String]) -> Result<(String, HashMap<String, Pr>)> {
    let organisation = github::organisation(&context.config.repos_dir, repos)
        .context("no repository with a GitHub origin found in REPOS_DIR")?;
    github::pull_requests(&organisation, PROJECT)
}

fn workspaces(config: &Config) -> Result<HashMap<String, String>> {
    let listing = run("sh", &["-c", WORKTREE_LIST], Some(&config.repos_dir))?;
    Ok(listing
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let (name, url) = (words.next()?, words.next()?);
            url.starts_with("http")
                .then(|| (name.to_uppercase(), url.to_owned())) // skips a header line
        })
        .collect())
}

/// Bring up the issue's worktree, open or focus its tmux window and, when `claim_as` is given,
/// make the issue mine and In Progress.
pub fn switch(
    context: &Context,
    target: &Target,
    claim_as: Option<&Person>,
    template: Template,
) -> Result<String> {
    let path = worktree(context, target)?;
    let issue = &target.issue;
    let prompt = issue_prompt(template, issue, target.parent.as_ref());
    open_session(issue, &path, &prompt, template.starts_new_agent)?;
    if let Some(me) = claim_as
        && !(issue.in_progress() && issue.assignee.as_ref() == Some(me))
    {
        context.jira.claim(&issue.key, &me.account_id)?;
    }
    Ok(format!("{} is open in {}", issue.key, path.display()))
}

/// Commit what is left, push, have the agent write the description, open the pull request (or
/// mark the existing draft ready), enable auto-merge and move the issue to Review.
pub fn finish(context: &Context, target: &Target) -> Result<String> {
    let path = worktree(context, target)?;
    let dir = Some(path.as_path());
    let title = format!("{} {}", target.issue.key, target.issue.summary);
    if !run("git", &["status", "--porcelain"], dir)?.is_empty() {
        run("git", &["add", "--all"], dir)?;
        run("git", &["commit", "--message", &title], dir)?;
    }
    let branch = run("git", &["rev-parse", "--abbrev-ref", "HEAD"], dir)?;
    run("git", &["push", "--set-upstream", "origin", &branch], dir)?;
    // The agent summarises what is actually on the branch now, after my own review and edits.
    let summarise = format!("git diff --no-ext-diff origin/main...HEAD | {AGENT} -p \"$0\"");
    let description = run("sh", &["-c", &summarise, PR_DESCRIPTION.trim()], dir)?;
    let url = match &target.pr {
        Some(pr) => {
            run("gh", &["pr", "edit", &pr.url, "--body", &description], None)?;
            pr.url.clone()
        }
        None => {
            let args = [
                "pr",
                "create",
                "--title",
                &title,
                "--body",
                &description,
                "--head",
                &branch,
            ];
            let created = run("gh", &args, dir)?;
            created.lines().last().unwrap_or_default().to_owned()
        }
    };
    if target.pr.as_ref().is_some_and(|pr| pr.is_draft) {
        run("gh", &["pr", "ready", &url], None)?; // a draft the agent opened, reviewed and now promoted
    }
    context.jira.transition(&target.issue.key, "review")?;
    github::enable_auto_merge(&url)?;
    Ok(format!("{} is in review: {url}", target.issue.key))
}

/// Show the issue's changes against main in a pane beside this tool. This reads the branch straight
/// from the repository, so looking at a diff never creates a worktree or starts services.
pub fn diff(context: &Context, target: &Target) -> Result<String> {
    require_tmux()?;
    let key = &target.issue.key;
    let repo = context.config.repos_dir.join(&target.repo);
    run("git", &["fetch", "--quiet", "origin"], Some(&repo))?;
    let branch = match &target.pr {
        Some(pr) => pr.branch.clone(),
        None => existing_branch(&repo, key)?.with_context(|| format!("{key} has no branch yet"))?,
    };
    let local = format!("refs/heads/{branch}");
    let has_local = run(
        "git",
        &["rev-parse", "--verify", "--quiet", &local],
        Some(&repo),
    )
    .is_ok();
    let revision = if has_local {
        branch
    } else {
        format!("origin/{branch}")
    }; // a local branch may hold commits that are not pushed yet
    let command = format!("{DIFF_VIEWER} origin/main...{}", quote(&revision));
    let dir = repo.to_string_lossy();
    // Next to this tool rather than in a window of its own; the pane closes with the viewer.
    let own_pane = std::env::var("TMUX_PANE").unwrap_or_default();
    let split = [
        "split-window",
        "-h",
        "-P",
        "-F",
        "#{pane_id}",
        "-t",
        &own_pane,
        "-c",
        &dir,
        &command,
    ];
    let pane = run("tmux", &split, None)?;
    run(
        "tmux",
        &["set-option", "-p", "-t", &pane, DIFF_PANE_OPTION, key],
        None,
    )?; // see `open_diffs`
    Ok(format!("Showing the diff of {key}"))
}

/// The keys of the issues whose diff is open in some pane right now. The diff panes carry
/// the issue key as a tmux pane option, so tmux is the only place this is kept.
pub fn open_diffs() -> HashSet<String> {
    let format = format!("#{{{DIFF_PANE_OPTION}}}");
    let panes = run("tmux", &["list-panes", "-a", "-F", &format], None).unwrap_or_default();
    panes
        .lines()
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The path of the issue's worktree, created on the right branch if it does not exist yet.
fn worktree(context: &Context, target: &Target) -> Result<PathBuf> {
    let config = &context.config;
    if target.add_label {
        context.jira.add_label(&target.issue.key, &target.repo)?;
    }
    let branch = match &target.pr {
        Some(pr) => pr.branch.clone(),
        None => existing_branch(&config.repos_dir.join(&target.repo), &target.issue.key)?
            .unwrap_or_else(|| new_branch_name(&target.issue)),
    };
    let command = WORKTREE_ADD
        .replace("{repo}", &quote(&target.repo))
        .replace("{branch}", &quote(&branch));
    let output = run("sh", &["-c", &command], Some(&config.repos_dir))?;
    let path = PathBuf::from(output.lines().last().unwrap_or_default());
    if !path.is_dir() {
        bail!("`{command}` did not print a worktree path");
    }
    Ok(path)
}

fn existing_branch(repo: &Path, key: &str) -> Result<Option<String>> {
    let args = [
        "for-each-ref",
        "--format=%(refname:short)",
        "refs/heads",
        "refs/remotes/origin",
    ];
    let branches = run("git", &args, Some(repo))?;
    Ok(branches
        .lines()
        .map(|branch| branch.strip_prefix("origin/").unwrap_or(branch))
        .find(|branch| issue_key_of(branch).as_deref() == Some(key))
        .map(str::to_owned))
}

/// `INI-12 Fix the thing!` becomes `INI-12-fix-the-thing`, cut off at a word boundary.
fn new_branch_name(issue: &Issue) -> String {
    let mut name = issue.key.clone();
    let summary = issue.summary.to_lowercase();
    let words = summary
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty());
    for word in words {
        if name.len() + 1 + word.len() > MAX_BRANCH_LENGTH {
            break;
        }
        name.push('-');
        name.push_str(word);
    }
    name
}

fn require_tmux() -> Result<()> {
    if std::env::var_os("TMUX").is_none() {
        bail!("not running inside tmux");
    }
    Ok(())
}

/// `INI-12 Fix the thing in the toolbar` becomes `INI-12 Fix the thing in…`: the key, then as
/// many whole words of the summary as fit.
fn window_name(issue: &Issue) -> String {
    let mut name = issue.key.clone();
    let mut length = name.chars().count();
    let mut words = issue.summary.split_whitespace().peekable();
    while let Some(word) = words.next() {
        let word_length = word.chars().count();
        let ellipsis = usize::from(words.peek().is_some()); // room to mark the cut after this word
        if length + 1 + word_length + ellipsis > MAX_WINDOW_NAME_LENGTH {
            name.push('…');
            break;
        }
        name.push(' ');
        name.push_str(word);
        length += 1 + word_length;
    }
    name
}

/// Focus the issue's tmux window (the one whose name starts with the issue key), or open one
/// with a live diff of the worktree and the agent side by side.
/// With `always_start_agent`, an existing window gets a new agent pane for the given prompt.
fn open_session(issue: &Issue, path: &Path, prompt: &str, always_start_agent: bool) -> Result<()> {
    require_tmux()?;
    let key = issue.key.as_str();
    let dir = path.to_string_lossy();
    let windows = run(
        "tmux",
        &["list-windows", "-a", "-F", "#{window_name}\t#{window_id}"],
        None,
    )?;
    let existing = windows
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .find(|(name, _)| name.split_whitespace().next() == Some(key)) // the summary after the key may have changed
        .map(|(_, id)| id.to_owned());
    let window = match &existing {
        Some(window) => window.clone(),
        None => run(
            "tmux",
            &[
                "new-window",
                "-P",
                "-F",
                "#{window_id}",
                "-n",
                &window_name(issue),
                "-c",
                &dir,
            ],
            None,
        )?,
    };
    if existing.is_none() {
        // Typed into the shell rather than run as the pane's command, so the pane survives quitting it.
        run(
            "tmux",
            &["send-keys", "-t", &window, DIFF_WATCH, "Enter"],
            None,
        )?;
    }
    if existing.is_none() || always_start_agent {
        let agent = format!(
            "{AGENT} --settings {} {}",
            quote(AGENT_SETTINGS.trim()),
            quote(prompt)
        );
        run(
            "tmux",
            &["split-window", "-h", "-t", &window, "-c", &dir, &agent],
            None,
        )?;
    }
    run("tmux", &["select-window", "-t", &window], None)?;
    run("tmux", &["switch-client", "-t", &window], None)?; // the window may live in another session
    Ok(())
}

pub fn issue_prompt(template: Template, issue: &Issue, parent: Option<&Issue>) -> String {
    let ticket = |role: &str, issue: &Issue| {
        format!(
            "<<< {role} TICKET START: {key} >>>\nKey: {key}\nSummary: {}\nDescription:\n{}\n<<< {role} TICKET END: {key} >>>",
            issue.summary,
            issue.description,
            key = issue.key,
        )
    };
    let mut prompt = format!(
        "{}\n\n{}",
        template.instructions.trim(),
        ticket("PRIMARY", issue)
    );
    if let Some(parent) = parent {
        prompt.push_str("\n\n");
        prompt.push_str(&ticket("PARENT", parent));
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(key: &str, summary: &str, description: &str) -> Issue {
        Issue {
            key: key.to_owned(),
            summary: summary.to_owned(),
            description: description.to_owned(),
            ..Issue::default()
        }
    }

    #[test]
    fn prompt_contains_issue_and_parent() {
        let task = issue("INI-2", "Add the export button", "It goes in the toolbar.");
        let epic = issue("INI-1", "Exports", "Everything around exporting data.");
        insta::assert_snapshot!(issue_prompt(IMPLEMENT, &task, Some(&epic)));
    }

    #[test]
    fn window_name_shows_the_start_of_the_summary() {
        let long = issue("INI-7089", "Device coupling tool: stop deriving a room", "");
        assert_eq!(window_name(&long), "INI-7089 Device coupling…");
        let short = issue("INI-2", "Add the export button", "");
        assert_eq!(window_name(&short), "INI-2 Add the export button");
        let one_long_word = issue("INI-3", "Supercalifragilisticexpialidocious", "");
        assert_eq!(window_name(&one_long_word), "INI-3…");
    }

    #[test]
    fn branch_name_is_cut_at_a_word_boundary() {
        let task = issue(
            "INI-7089",
            "Device coupling tool: stop deriving a room (stoel/chair) from the room name",
            "",
        );
        assert_eq!(
            new_branch_name(&task),
            "INI-7089-device-coupling-tool-stop-deriving-a-room-stoel"
        );
    }
}
