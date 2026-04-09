# Workflow Entrypoints

A developer doesn't always start from the same place. This document maps out
the different entrypoints into a typical Jira + GitHub workflow and how they
converge into a shared development loop.

## Overview

```mermaid
flowchart TD
    %% ── Entrypoints ──
    EP1["🔍 Browse Jira backlog"]
    EP2["📋 Assigned to me"]
    EP3["💡 New idea / bug spotted"]
    EP4["🌿 Already have a local branch"]
    EP5["👀 PR review request"]
    EP6["🔴 CI failed on my PR"]
    EP7["🔄 Returning to WIP"]

    %% ── Converge into states ──
    EP1 --> TRIAGE
    EP2 --> HAS_ISSUE
    EP3 --> CREATE
    EP4 --> RECONCILE
    EP5 --> REVIEW
    EP6 --> FIX_CI
    EP7 --> FIND_WIP

    %% ── Triage / grooming ──
    TRIAGE["Triage & groom issues\n(edit, label, prioritize)"]
    TRIAGE --> PICK_OR_SKIP{Pick up\nan issue?}
    PICK_OR_SKIP -- No --> DONE_TRIAGE["Done triaging"]
    PICK_OR_SKIP -- Yes --> HAS_ISSUE

    %% ── Create new issue ──
    CREATE["Create Jira issue"] --> HAS_ISSUE

    %% ── Central: have an issue, need a branch ──
    HAS_ISSUE["Issue exists in Jira"] --> LABEL{Repo label\nattached?}
    LABEL -- No --> ADD_LABEL["Attach repo label"]
    ADD_LABEL --> LABEL
    LABEL -- Yes --> PICKUP["Pick up issue\n(branch + assign + transition)"]
    PICKUP --> DEV_LOOP

    %% ── Reconcile existing branch ──
    RECONCILE["Local branch exists\nwithout formal pickup"] --> LINK{Branch name\ncontains issue key?}
    LINK -- Yes --> SYNC_JIRA["Sync Jira status\n(assign + transition)"]
    LINK -- No --> RENAME["Rename branch or\ncreate issue to match"]
    RENAME --> SYNC_JIRA
    SYNC_JIRA --> DEV_LOOP

    %% ── Returning to WIP ──
    FIND_WIP["Find issue with\nexisting branch/PR"] --> CHECK_STATE{PR exists?}
    CHECK_STATE -- No --> DEV_LOOP
    CHECK_STATE -- Yes --> CHECK_CI{CI status?}
    CHECK_CI -- Passing --> WAIT_REVIEW["Awaiting review\nor ready to merge"]
    CHECK_CI -- Failing --> FIX_CI
    CHECK_CI -- No runs yet --> DEV_LOOP

    %% ── Fix CI ──
    FIX_CI["Identify failing checks"] --> DEV_LOOP

    %% ── Core development loop ──
    DEV_LOOP["Write / fix code locally"]
    DEV_LOOP --> PUSH["Push & create/update PR"]
    PUSH --> CI{CI passing?}
    CI -- No --> DEV_LOOP
    CI -- Yes --> REVIEW

    %% ── Review ──
    REVIEW["Review PR\n(give or receive)"]
    REVIEW --> APPROVED{Approved?}
    APPROVED -- Changes requested --> DEV_LOOP
    APPROVED -- Yes --> MERGE["Merge PR"]
    MERGE --> CLOSE["Transition Jira\nissue to Done"]
```

## Entrypoints explained

### 1. Browse Jira backlog

> *"Let me see what needs doing."*

You open the TUI with no specific task in mind. You're scanning the backlog,
reading descriptions, maybe grooming (editing descriptions, attaching labels,
reprioritizing). You might pick something up, or just organize.

### 2. Assigned to me

> *"Someone assigned me a ticket."*

An issue already has your name on it. You open the TUI, find it in your list,
and pick it up — creating a branch and transitioning it to "In Progress" in one
step.

### 3. New idea / bug spotted

> *"I just found a bug"* or *"I have an idea for a feature."*

No Jira issue exists yet. You create one from the TUI (or Jira directly), then
immediately pick it up and start working.

### 4. Already have a local branch

> *"I started hacking on something before opening the TUI."*

You already have code on a branch. The workflow needs to **reconcile** — link
the branch to a Jira issue (by key in the branch name), sync the Jira status,
and continue into the normal development loop.

### 5. PR review request

> *"Someone asked me to review their PR."*

You're entering the workflow from the GitHub side. You need to find the related
Jira issue for context, review the code, and leave feedback. You're not the
author — you're a participant.

### 6. CI failed on my PR

> *"My build is red."*

You need to quickly find the issue, see which checks failed, jump back into the
code, fix it, and push again. The entrypoint is the failure notification — the
TUI should surface this prominently.

### 7. Returning to work-in-progress

> *"I context-switched yesterday, where was I?"*

You need to find your in-flight work. The TUI should make it obvious which
issues have active branches, open PRs, and what state they're in (waiting for
CI, waiting for review, changes requested, etc.).

## How entrypoints converge

All paths eventually feed into the same **development loop**:

```mermaid
flowchart LR
    CODE["Write code"] --> PUSH["Push"] --> CI["CI"] --> REVIEW["Review"] --> MERGE["Merge"]
    CI -- fail --> CODE
    REVIEW -- changes --> CODE
```

The value of mapping entrypoints is understanding that the "pick up from Jira"
path is just one of many. A good work TUI should make **every** entrypoint
feel natural — not just the happy path.
