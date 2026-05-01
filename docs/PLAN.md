# Planning Guide for Managing Ideas and Plans

This document defines the default GitHub workflow for ideas, plans, tasks, bugs, and release work in the `shine` repository.

## Purpose

The workflow is intentionally lightweight. It is designed for solo maintenance or a small team and should answer three questions quickly:

- What ideas have been captured?
- What is ready to work on next?
- What is planned for the next release?

## Default Workflow

Use GitHub features with one clear responsibility each:

- **Issues** hold the actual content and discussion
- **Labels** classify the issue type and current state
- **Projects** visualize the workflow
- **Milestones** group work into releases or phases

`docs/PLAN.md` is the rulebook. It is not the live task list.

## Core Rules

- Every new idea starts as an issue.
- Prefer one issue per problem or deliverable.
- Each issue should have one primary type label.
- Each issue should have exactly one `status:` label.
- Detailed discussion belongs in the issue, not in the project card.
- Only release-relevant work needs a milestone.

## Issue Types

Use these labels as the primary classification:

- `idea` for rough concepts or possible improvements
- `plan` for scoped work that is understood but not yet being implemented
- `task` for implementation-ready work
- `bug` for user-facing defects or regressions
- `docs` for documentation changes
- `enhancement` for a user-visible improvement that is already accepted

## Status Flow

Use exactly one of these labels at a time:

- `status: inbox` for newly captured work
- `status: next` for approved and ready work
- `status: in-progress` for active implementation
- `status: blocked` for work waiting on a dependency or decision
- `status: review` for work ready to verify
- `status: done` for completed work that is about to be closed

Recommended lifecycle:

1. `idea` + `status: inbox`
2. `plan` + `status: next`
3. `task` + `status: in-progress`
4. `status: review`
5. `status: done`, then close the issue

## Issue Entry Points

Use the repository issue templates:

- `Idea / Plan` for new directions, proposals, and rough planning
- `Task` for implementation-ready work
- `Bug` for regressions and broken behavior

Every planning issue should answer:

- What problem are we solving?
- Why does it matter now?
- What is the proposed direction?
- What is the smallest next step?
- How do we know it is done?

## Projects

Create one GitHub Project for the repository and mirror the status flow with these columns:

- `Inbox`
- `Next`
- `In Progress`
- `Blocked`
- `Review`
- `Done`

Usage rules:

- New issues start in `Inbox`
- Move an item to `Next` only when it is ready to be worked on
- Use `Blocked` only when an external dependency or decision is truly stopping progress
- Close issues after they have reached `Done`

The project board should answer "where is this work now?" and nothing more.

## Milestones

Use milestones only when the issue matters to a release or phase.

Examples:

- `v0.5`
- `v0.6`
- `v1.0`
- `stabilization`

Rules:

- Assign a milestone when the issue should ship in a specific release or phase
- Leave small internal chores unassigned unless timing matters
- Use milestones to answer "what is in this release?"

## Weekly Review

Run a short review once per week:

1. Review all open `idea` and `plan` issues
2. Close stale items that no longer matter
3. Promote useful ideas into `plan` or `task`
4. Move ready work into `status: next`
5. Check that active work is still in the correct milestone

## Recommended Repo Setup

After merging this workflow, configure GitHub to match it:

1. Create the labels listed in this document
2. Create a project board with the recommended columns
3. Create milestones only when a release or phase is real enough to plan against

GitHub configuration is partly manual. The repository stores the templates and the process, while labels, projects, and milestones are maintained in the GitHub UI.
