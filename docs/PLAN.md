# Planning Guide for Managing Ideas and Plans

This document defines how to manage ideas, plans, tasks, bugs, and release work in the `shine` repository.

## Purpose

The goal of this workflow is to keep planning lightweight, visible, and easy to maintain for a solo-maintained or small-team open-source project.

This process helps you:

- collect ideas without losing them
- separate rough ideas from actionable plans
- track work in progress
- review priorities regularly
- keep release planning visible

## Recommended GitHub Workflow

Use GitHub's built-in features as follows:

- **Issues** for ideas, plans, tasks, and bugs
- **Labels** for type and status
- **Projects** for visual workflow tracking
- **Milestones** for version or phase planning

## Issue Lifecycle

### 1. Inbox
A new idea or task is captured here first.

Typical label:
- `status: inbox`

### 2. Idea / Discussion
The item is reviewed, discussed, or clarified.

Typical labels:
- `idea`
- `discussion`
- `status: inbox`

### 3. Plan
The idea becomes actionable and is broken down into steps.

Typical labels:
- `plan`
- `status: next`

### 4. In Progress
Work is actively being done.

Typical labels:
- `task`
- `status: in-progress`

### 5. Blocked
Work cannot continue yet because of a dependency, unanswered question, or missing decision.

Typical labels:
- `status: blocked`

### 6. Review
The implementation is ready for verification.

Typical labels:
- `status: review`

### 7. Done
The work is completed and the issue can be closed.

Typical labels:
- `status: done`

## Labels

Keep labels simple and consistent.

### Type labels
Use one or more of these labels to classify the issue:

- `idea`
- `plan`
- `task`
- `bug`
- `docs`
- `enhancement`

### Status labels
Use exactly one of these labels to represent current state:

- `status: inbox`
- `status: next`
- `status: in-progress`
- `status: blocked`
- `status: review`
- `status: done`

## Milestones

Use milestones to group work by version or release phase.

### Version-based examples
- `v0.1`
- `v0.2`
- `v1.0`

### Phase-based examples
- `foundation`
- `core-features`
- `polish`
- `stabilization`

### Milestone rules
- Every meaningful feature or release-worthy change should belong to a milestone.
- Small internal fixes may stay unassigned if they are not release-sensitive.
- Milestones should be used to answer: “What is planned for this release?”

## Projects

Use a GitHub Project board to make work visible.

Recommended columns:

- `Inbox`
- `Next`
- `In Progress`
- `Blocked`
- `Review`
- `Done`

### Project usage rules
- New ideas enter `Inbox`
- Ready items move to `Next`
- Active work moves to `In Progress`
- Waiting items move to `Blocked`
- Completed items move to `Done`

## Weekly Review

Once per week, spend a short time reviewing the board.

### Review checklist
- Review all open issues
- Decide whether each idea is still relevant
- Promote valuable ideas into plans
- Move ready work into `Next`
- Move active work into `In Progress`
- Close completed items
- Remove stale items that no longer matter

## How to Write a Good Issue

Every idea or plan issue should answer these questions:

- What is it?
- Why does it matter?
- What outcome do we want?
- What is the smallest useful next step?
- Are there any risks or dependencies?

## Recommended Issue Template

Use this format for idea and plan issues:

```md
## Background
Why is this needed? What problem does it solve?

## Goal
What result do we want?

## Proposal
What is the proposed solution or direction?

## Risks
What could block or complicate this work?

## Next Steps
What is the smallest actionable step?
