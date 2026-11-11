# work — requirements

Working document for a lean rewrite of `~/code/work-tui`. Status: **first version built. Reading (list, pull requests, CI logs, search) is verified against real data; the actions that write are not yet exercised, and neither is `momo worktree` on the real stack**.

## Purpose

One place to see what there is to work on, start on it, and see how it is doing afterwards.

The core loop, and the only thing every feature has to justify itself against:

1. **See** — my project's open issues, with whoever is on them and the state of their PR.
2. **Switch** — one key on an issue puts me in its worktree, in an agent session, with the issue claimed in Jira. No dialogs, no manual steps, whether the issue is new to me or one I was on yesterday.
3. **Follow** — PR review state, CI and unresolved comments, without opening GitHub.
4. **Hand over** — get the work into review; review other people's PRs.
5. **Capture** — create a ticket inline the moment it comes up.

## What work-tui is today

About 18k lines of Rust (ratatui, tokio, gouqi) across 143 commits in six months, 23 background actions, 7 popups, 16 keybindings.
The size sits in three places: the list (`ui/list/mod.rs`, 2.9k), the app state machine (`app.rs`, 2.2k) and the GitHub client (`apis/github.rs`, 1.3k).

## Decisions log

| # | Decision | Consequence |
|---|---|---|
| 1 | Lean full-screen TUI | Live PR/CI overview stays; no separate CLI |
| 2 | Rust + ratatui | Leanness has to come from scope and structure, not from the language |
| 3 | Jira labels remain the issue-to-repo link | Labels are read as-is. The tool sets the label itself wherever it creates or first touches an issue, so there is never a manual labelling step |
| 4 | Switching must be seamless | Pick-up becomes "switch": no confirmation dialog, no option matrix |
| 5 | Openspec is cut | Propose, tasks.json import, pending-import scan and the fs watcher go (~1.3k lines) |
| 6 | Reviewing stays | Branch diff, approve & auto-merge, reviewable-PR notifications |
| 7 | Inline ticket creation stays | Used a lot |
| 8 | CI logs and fix-CI stay, but rebuilt | The current versions are not good enough to be used; they need a simpler design, not a port |
| 9 | A worktree per issue, always | No single-working-tree mode, no stash or carry logic, no dirty-tree handling anywhere. Main checkouts stay on main |
| 10 | Finish stays as is | Commit all as `KEY summary`, push, create PR, enable auto-merge, transition to Review |
| 11 | No stories, no subtasks | The hierarchy is epic > task. Tasks are grouped under their epic, collapsible. The story/task toggle and story status sync have nothing left to do |
| 12 | Sessions open in tmux | One window per issue, named after its key and the start of its title, focused if it already exists |
| 13 | Moving to In Progress always moves to the board | One operation, not two |
| 14 | `momo` owns worktrees and the dev stack | `work` asks `momo` for a worktree and gets a path back; it knows nothing about ports, nginx or processes |

## Principles

- **Reading never writes.** Opening or refreshing must not change Jira.
- **Writes happen only as part of something I pressed a key for.**
- **Don't rebuild what the agent or the browser does better.**
- **Shell out to `gh`, `git` and `momo`**; keep the Jira surface to a handful of REST calls.
- **Config in a file, not in a UI**, for anything changed less than weekly.
- **One way to do each thing.**

## Requirements

### See

- Issue list of the project's board, refreshed every 60s. While CI runs, only the pull requests it runs on are re-fetched every 10s, by id; the full GitHub search is too slow and heavy to repeat that often.
- One section per board column, from Review back to the first column, then the backlog. `<` moves an issue from the backlog onto the board, `>` back. Issues past Review are not loaded.
- Issues appear as soon as Jira answers (about a second); pull requests fill in when GitHub does. The bare GitHub search takes about 1.3s; the per-pull-request details (reviews, review threads, CI checks) take several seconds more, so they are fetched in three parallel requests and must never hold up the list.
- PRs from a single GitHub search on `head:<PROJECT>-`, matched to issues by branch prefix.
- Board columns are flat lists; an issue's epic is shown in the sidebar and matched by search. The backlog is grouped by epic. Epics themselves are not listed as rows.
- Columns: issue, status, assignee, age, PR number + unresolved comments, CI, repo (the repositories its labels name).
- A pull request is greyed out unless I can move it forward: mine with conflicts, failing CI, requested changes or unresolved comments; someone else's that is not a draft and that I have not reviewed yet (or where a new review was requested from me).
- Age fades in four steps of white: last hour, last day, last week, older.
- `a` tags an issue with a repository: the chosen one replaces any other repository labels on the issue. Labels that do not name a repository are never touched.
- A coloured badge between the issue key and the title says what stands out (no column is reserved for it): `DIFF` (green) while its diff is open in a pane, `WORK` (blue) when it has a worktree, `NEW` (yellow) on a PR that is mine to review and that I have not looked at yet (in that order of precedence). Looking means opening its diff or the PR; the ids of looked-at PRs are the one thing the tool keeps on disk (`~/.local/state/work/seen-pull-requests`), so `NEW` survives restarts. There is deliberately no badge for every reviewable PR; that was too much. `DIFF` and `WORK` also tint the row. Both are read from the outside world (the worktree list, tmux), not remembered by the tool.
- The workspace URL of an issue with a worktree is in the sidebar.
- Fuzzy search over visible rows.
- Open PR / ticket in browser.

### Switch (replaces pick up)

1. Repo comes from the issue's label. If there is none, fuzzy-pick a repo once; the tool writes the label.
2. Branch name is the existing branch for the issue key if there is one (local or remote), otherwise `KEY-slugged-summary`.
3. Run the worktree command (see below) with repo and branch. It creates the worktree if needed, starts its services and returns the path.
4. (The agent's tab is marked in tmux, via the terminal bell, whenever the agent finishes a turn or needs an answer.) Focus the tmux window for the issue, or open one in the worktree: a live diff of the uncommitted changes (`hunk diff --watch`) in one pane, the agent with the issue (plus its epic) as prompt in the other.
5. If the issue is not mine and In Progress yet: assign to me, transition to In Progress, move to board. This is the one step that asks first (`y` to confirm), and it asks before anything else happens, so cancelling leaves no worktree behind.

Steps 1–4 are the same for a new issue, one of mine from yesterday, and someone else's PR I want to look at; only step 5 is conditional.

### Follow

- Sidebar: description, PR status, checks. Built from data the list already fetched where possible.
- CI logs: one key shows the failing output of the selected PR's checks. Thin wrapper around `gh run view --log-failed`, in a scrollable pane.
- Fix CI: one key runs switch on that issue with "CI is failing on this branch, fix it" as the prompt. The agent fetches the logs itself.

### Hand over

- Finish, run in the issue's worktree: commit everything as `KEY summary` if dirty, push, have the agent write the description (a few bullet points of what changed, from the branch's diff; `prompts/pr-description.md`), create the PR or mark the agent's draft ready for review, enable auto-merge, transition to Review.
- Prompt templates in `prompts/`, one file per kind of session, with the issue and its parent appended. `implement.md` (on `enter`) has the agent do the work and end by opening a draft PR for me to review; `fix-ci.md` (on `C`) has it fix the failing checks. Choosing between templates on `enter` comes with the second one.
- Approve & auto-merge the selected PR.
- Branch diff: `hunk diff origin/main...<branch>` in a pane beside the list (not a window of its own), read straight from the repository. It creates no worktree and starts no services, so glancing at someone's PR is instant and leaves nothing behind.
- A terminal bell (which marks the tab in tmux until visited) and a line in the status bar when a PR becomes reviewable by me: someone else's, not a draft, not reviewed by me yet or with a new review requested from me. The same rule that un-greys a PR in the list. Only for PRs that appear while the tool runs, not for what is there at startup.
- Removing a workspace is `momo worktree rm`, not a key in the TUI.

### Capture

- Inline new task under the cursor's epic. It gets no repo label; tag it with `a` or let switch ask.

### Config

- Environment variables for the Jira connection, the same ones work-tui used: `JIRA_URL`, `JIRA_EMAIL`, `JIRA_API_TOKEN`, `REPOS_DIR`.
- The project (`INI`), the worktree commands (`momo worktree add/ls`) and the agent (`claude`) are constants in `src/config.rs`, so there is nothing else to set up. `work` still only knows `momo` as three command templates.

## What was built

About 2,300 lines of Rust including tests, against 18k before. No async runtime: each action is a thread that shells out and reports back over a channel.

Where it differs from the requirements above:

- **Backlog grouping is by direct parent, whatever its type.** The project still holds stories and subtasks; they group under their own parent.
- **Collapsed sections are not remembered** between runs. Nothing is cached on disk at all.
- **A new task shows up on the next refresh**, not instantly: Jira's search lags a moment behind creation, and there is no optimistic insert.
- **Move to board** posts the issue to the project's single board. Projects with several boards or sprints are not handled.
- **Refresh fetches at most 500 issues.**
- **An issue with subtasks is flattened into its subtasks.** The parent is not listed; each subtask is, in the column of its own status, filed under the parent's epic. Jira reports every subtask as backlog, so a subtask only counts as backlog when its parent is there.

Not yet run for real, because they write to Jira, GitHub, git or tmux: switch, fix CI, finish, diff, approve, new task.

## Worktrees and the dev stack

Built in `~/momo/momolite` as `momo worktree` (see its README for usage). The design:

- **Workspace.** An issue's worktrees sit next to the main checkouts as `~/worktrees/<issue-key>-<repo>` (for example `~/worktrees/ini-1234-admin-panel-backend`), outside `~/momo` so nothing that scans the repositories trips over them. A `momo-local-stack` symlink in `~/worktrees` keeps the relative paths in `process-compose.yaml` resolving from inside a worktree.
- **Services.** Each process in `process-compose.yaml` that lives in a worktree'd repo gets a second instance, on its port + 10 × the workspace number. They are added to and removed from the running stack with `process-compose project update`; nothing is restarted that did not change.
- **Routing.** Each workspace gets its own gateway, `http://localhost:<9000+N>` and `http://<issue-key>.localhost` (not `127.0.0.1`: the Next.js dev servers refuse to serve their scripts to pages loaded from an IP address), with the same paths as the main one (`/beheer`, `/ap/api`, `/postfab`, …). Requests for the workspace's own services go to its instances, everything else to the main checkouts. So a frontend-only issue runs one extra Next.js server against the shared backends.
- **Derived, not configured.** Both the extra processes and the nginx server blocks are generated from the hand-written `process-compose.yaml` and `nginx/conf.d/local.conf`. Adding a service to momolite makes it worktree-capable with no extra work, provided it has an explicit `PORT`.
- **Dependencies.** A new worktree gets the main checkout's `node_modules`, yarn cache and install state as copy-on-write clones (APFS). No extra disk until files change, nothing shared, and `yarn install` only applies the lockfile difference. Measured on admin-panel-backend: about 30s to clone, 3s for the install that follows.
- **Status.** `momo status` shows each workspace with its URL and the branch and state of its worktrees.
- **Any repo.** `momo worktree add` works for every repository under `~/momo`; ones without services simply get a worktree.

What was checked: the commands and the generated configuration against a throwaway copy of the stack, and adding/removing processes on a live throwaway process-compose project.

What was not: real services running from a worktree. See the first open question.

### Known limits

- **Databases are shared.** A migration on a branch changes the schema for every workspace and the main checkouts. Migrations and seeds run from the main checkouts only.
- **Every worktree pays for a build** on first start, and a running workspace costs a Node process per service. The limit is 20 workspaces; the practical limit is memory.
- **process-compose must be v1.124 or newer.** The installed v1.120 restarts every process on the first update after `momo up`, and can shut the project down while doing so. Both are fixed upstream.
- **Main checkouts must be on main.** Git refuses to check out a branch in two places, and several are currently on issue branches.

## Cut

| Feature | Why |
|---|---|
| Openspec (propose, import, scan, watcher) | Decision 5 |
| Story status sync, story/task toggle | Decision 11 |
| Pick-up dialog (branch base, carry/abort, open modes) | Decisions 4 and 9 |
| Dirty-tree detection, active-branch detection | Decision 9: a worktree existing is the signal |
| Bulk auto-label, tag-jira-repos, `normalize-labels` binary, auto-tag toggle | Background writes; replaced by labelling at switch/create time |
| Label picker popup | Folded into switch: it only appears when a label is missing |
| Project / status filter picker | 820 lines; the list now simply follows the board |
| Lazy child fetching, board/backlog sections | Decision 11 |
| CI ETA and the duration cache | Nice, not needed |
| CI log tabs, sanitising, error-context extraction | Replaced by the thin version |
| Optimistic UI updates | Refetch after an action instead |
| Step-by-step progress messages | A spinner and the final error are enough |
| Help overlay | A one-line footer once there are fewer keys |
| `debug-action` binary, `debug.log`, git hooks | Tooling around complexity that should no longer exist |
| Hardcoded `~/momo` fallback | — |
| `gouqi` | A few REST calls replace it |

## Open questions

1. **First real run of a workspace.** Needs process-compose upgraded, the main checkouts on main, and `cargo install --path .` in momolite. Things to watch: do the frontends call the API with relative URLs (they appear to), does login survive a different port, and how long does a cold worktree take to come up.
2. **Should a workspace's services start on `momo worktree add`, or only on demand?** They start immediately now. With many open issues that is a lot of Node; the alternative is `momo worktree start/stop <name>`, called by switch.
3. **Issues spanning two repos.** A workspace can hold several worktrees, but an issue has one tmux window and the label picks one repo. Is a second repo added by hand (`momo worktree add`), or does switch need to handle it?
