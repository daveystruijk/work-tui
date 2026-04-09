# Jira <-> GitHub Workflow

This document describes the typical workflow when using `work-tui` to manage
Jira issues alongside local Git repositories and GitHub pull requests.

## High-level flow

```mermaid
flowchart TD
    A[Browse Jira issues in TUI] --> B{Issue has repo label?}
    B -- No --> C[Attach repo label via label picker]
    C --> B
    B -- Yes --> D["Pick up issue (p)"]
    D --> D1[Ensure clean working tree]
    D1 --> D2["Create branch from origin/main\n(ISSUE-key-slug)"]
    D2 --> D3[Assign issue to yourself in Jira]
    D3 --> D4["Transition issue to 'In Progress'"]
    D4 --> E[Work on code locally]
    E --> F["Push branch & create PR on GitHub\n(branch name contains issue key)"]
    F --> G["work-tui detects PR via gh CLI\n(matches branch prefix to issue key)"]
    G --> H[TUI shows PR status & CI checks]
    H --> I{CI passing?}
    I -- No --> E
    I -- Yes --> J[PR reviewed & merged on GitHub]
    J --> K[TUI activity column updates\nto 'PR merged']
```

## Step-by-step breakdown

### 1. Issue discovery

```mermaid
flowchart LR
    ENV["JIRA_URL\nJIRA_EMAIL\nJIRA_API_TOKEN\nJIRA_JQL"] --> JQL[Run JQL search]
    JQL --> LIST[Populate issue list in TUI]
    LIST --> GH["Query GitHub for each\nrepo-labeled issue"]
    GH --> STATUS[Show activity/CI status\nper issue in list view]
```

On launch (or pressing `r`), the TUI fetches issues from Jira using the
configured JQL query. It then scans for matching GitHub PRs by checking if a
branch with the issue key exists in the linked repository.

### 2. Repo linkage via labels

```mermaid
flowchart TD
    ISSUE[Select issue in TUI] --> HAS{Has repo label?}
    HAS -- Yes --> READY[Ready for pickup]
    HAS -- No --> PICKER["Open label picker (a)"]
    PICKER --> SCAN["Scan ~/momo/* for\nlocal repositories"]
    SCAN --> SELECT[Select matching repo]
    SELECT --> UPDATE["Add label to Jira issue\n(normalized repo name)"]
    UPDATE --> READY
```

Jira issues are linked to local Git repositories through **labels**. Each label
corresponds to a directory under `~/momo/`. The label picker normalizes names
(lowercase, alphanumeric + dashes) so Jira labels match directory names.

### 3. Picking up an issue

```mermaid
flowchart TD
    PICK["Press 'p' on issue"] --> CHECK_LABEL{Repo label\npresent?}
    CHECK_LABEL -- No --> ABORT1[Abort: add label first]
    CHECK_LABEL -- Yes --> CHECK_GIT{Working tree\nclean?}
    CHECK_GIT -- No --> ABORT2[Abort: commit or stash changes]
    CHECK_GIT -- Yes --> FETCH[git fetch origin]
    FETCH --> BRANCH["git checkout -b\nISSUE-KEY-slug origin/main"]
    BRANCH --> ASSIGN[Assign issue to current\nuser via Jira API]
    ASSIGN --> TRANSITION["Transition issue to\n'In Progress'"]
    TRANSITION --> DONE[Branch created, issue\nassigned and in progress]
```

This is the core automation step. A single keypress creates the branch, assigns
the ticket, and moves it to "In Progress" in Jira.

### 4. Development & PR creation

```mermaid
flowchart TD
    DEV[Write code on feature branch] --> PUSH[Push branch to origin]
    PUSH --> PR["Create PR on GitHub\n(manually or via gh CLI)"]
    PR --> NOTE["Branch name must contain\nthe Jira issue key\nfor auto-detection"]
```

PR creation happens **outside** the TUI (e.g. via `gh pr create` or the GitHub
UI). The only requirement is that the branch name starts with the Jira issue
key, which is guaranteed when using the pickup flow.

### 5. Status syncing

```mermaid
flowchart TD
    REFRESH["TUI refresh (r)"] --> REPOS[Identify repos from\nissue labels]
    REPOS --> GH_QUERY["gh pr list --head ISSUE-KEY\nper repo"]
    GH_QUERY --> FOUND{PR found?}
    FOUND -- No --> JIRA_STATUS[Show Jira status\nas activity]
    FOUND -- Yes --> CHECKS["gh pr checks\nfor CI status"]
    CHECKS --> EVENTS["gh pr view --json\nfor timeline events"]
    EVENTS --> MERGE[Merge Jira changelog\n+ GitHub events]
    MERGE --> DISPLAY["Display combined activity\nin detail view"]
```

The TUI continuously merges information from both systems:

| Source | Data |
|--------|------|
| Jira | Issue status, assignee, transitions, description changes |
| GitHub | PR state (open/merged/closed), reviews, CI check results |

GitHub events take priority in the activity column when a PR exists; otherwise
the TUI falls back to Jira status heuristics.

## Complete lifecycle

```mermaid
flowchart TD
    subgraph Jira
        J1[Issue created] --> J2[Label attached]
        J2 --> J3[Assigned + In Progress]
        J3 --> J4[Done / Closed]
    end

    subgraph work-tui
        T1["Browse issues"] --> T2["Attach repo label (a)"]
        T2 --> T3["Pick up issue (p)"]
        T3 --> T4["View PR status & activity"]
        T4 --> T5["See 'PR merged' in activity"]
    end

    subgraph "Local Git + GitHub"
        G1["Branch created\nfrom origin/main"] --> G2[Code committed & pushed]
        G2 --> G3[PR created on GitHub]
        G3 --> G4[CI runs, review happens]
        G4 --> G5[PR merged]
    end

    T2 -.-> J2
    T3 --> G1
    T3 -.-> J3
    G3 -.-> T4
    G5 -.-> T5
    G5 -.-> J4
```

## Keyboard shortcuts reference

| Key | Screen | Action |
|-----|--------|--------|
| `r` | List / Detail | Refresh issues + GitHub statuses |
| `p` | List / Detail | Pick up issue (branch + assign + transition) |
| `a` | Detail | Open repo label picker |
| `e` | List / Detail | Edit issue description |
| `o` | List / Detail | Open issue in browser |
| `n` | List | Create new Jira issue |
| `Enter` | List | View issue detail |
| `Esc` | Detail / Picker / Edit | Go back |
