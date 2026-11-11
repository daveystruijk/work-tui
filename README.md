# work

A terminal UI for picking up work: Jira issues with their pull requests and CI, and one key to land in an issue's worktree with an agent session.

Direction and decisions are in [REQUIREMENTS.md](REQUIREMENTS.md).

## Setup

Needs `gh` (authenticated), `git`, `tmux`, `momo`, `claude` and `hunk`; run it inside tmux.

```sh
export JIRA_URL="https://yourteam.atlassian.net"
export JIRA_EMAIL="you@example.com"
export JIRA_API_TOKEN="..." # https://id.atlassian.com/manage-profile/security/api-tokens
export REPOS_DIR="$HOME/momo" # the directory holding all repositories
```

The Jira project (`INI`), the worktree commands (`momo worktree`) and the agent (`claude`) are constants in `src/config.rs`.

```sh
cargo install --path .
work
```

## Keys

| Key | Action |
|---|---|
| `j` `k` `g` `G` `ctrl-d` `ctrl-u` | Move |
| `h` `l` `space` | Collapse, expand, toggle the section |
| `/` | Search; `esc` clears |
| `enter` | Switch: worktree and tmux window with a live diff (`hunk diff --watch`) next to the agent. If the issue is not yours and In Progress yet, asks before assigning it to you and moving it to In Progress and the board |
| `f` | Finish: commit, push, have the agent write a short description, open the pull request or mark the agent's draft ready, enable auto-merge, move to Review |
| `n` | New task in the same epic as the selected issue |
| `a` | Tag the issue with a repository: sets that Jira label and removes other repository labels; other labels are left alone |
| `<` `>` | Move the issue from the backlog onto the board, or back |
| `c` | Failed CI output of the pull request |
| `C` | Open the issue with an agent told to fix CI |
| `b` | Diff against main in hunk, in a pane beside the list; reads the branch directly, no worktree is created |
| `V` | Approve the pull request and enable auto-merge |
| `o` `t` | Open pull request, ticket in the browser |
| `r` | Refresh |
| `q` | Quit |

The list follows the project's board: one section per column from Review back to the first, then the backlog (collapsed at start). The board columns are flat, with an issue's epic in the sidebar; the backlog is grouped by epic. An issue with subtasks is not listed itself: its subtasks are, each in the column of its own status. Issues past Review are not loaded.

A badge between an issue's key and its title says what stands out about it: `DIFF` (green) while its diff is open in a pane, `WORK` (blue) when it has a worktree, `NEW` (yellow) on a pull request that is yours to review and that you have not looked at yet; opening its diff (`b`) or the pull request (`o`) clears it, and it is remembered across restarts in `~/.local/state/work/`. The first two also tint the row.

An issue's repository is the one its Jira label names, shown in the Repo column. `a` tags an issue by hand; when no label matches a repository, switch asks once and adds the label.

## Prompts

What the agent is told lives in `prompts/`: `implement.md` for `enter`, `fix-ci.md` for `C`, and `pr-description.md` for the description `f` has the agent write. The issue and its parent are appended automatically. The files are compiled in, so rebuild after editing one.

## Agent sessions

Sessions started with `enter` or `C` get the hooks in `agent-settings.json`: the terminal bell rings when the agent finishes a turn or needs an answer, which marks the ticket's tab in tmux until you visit it. Your other Claude sessions are not affected.

## Development

```sh
cargo test                    # snapshot tests of the screens and the agent prompt
cargo insta review            # after an intended change
```
