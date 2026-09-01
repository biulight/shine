# 0069 — Preset authoring plans are hypothetical reports, not mutation approvals

- **Status**: Accepted
- **Date**: 2026-09-01
- **Evidence**: `core/src/runtime/authoring.rs`, `cli/src/preset_authoring.rs`,
  `docs/preset-developer-experience-prd.md`

## Context

Roadmap Phase 5 requires authors to inspect a Preset Plan without activating or installing the
Preset. Phase 3 already established `PlanV1` as the exact, snapshot-bound review contract for a real
mutation, and `PlanApprovalV1` authorizes only that exact Plan after fresh state capture. Phase 4
added execution and recovery semantics behind the same boundary.

An authoring preview cannot observe a real target machine consistently across macOS, Linux, and
Windows. It must use synthetic HOME, Shine state, command detection, environment, trust, manifests,
and destinations. Serializing that result as an ordinary approved Plan would blur hypothetical
design feedback with real mutation authorization. Building a separate planner would instead let
authoring behavior drift from runtime lifecycle behavior.

## Decision

`shine preset plan` reuses the existing Core security planners against a deterministic,
observation-only in-memory host and one immutable Preset snapshot. It emits a separate versioned
authoring report. The report may reuse the semantic step and permission-resolution value types, but
it does not expose `PlanApprovalV1`, an apply token, or an authoring fingerprint accepted by any
mutation entry point.

Every report identifies its platform and synthetic assumptions. The initial contract models a
first install with empty manifests, absent destinations, absent environment and secret inputs, no
trust grants, no detected commands, and no administrator state. App and Shell categories use their
install planners. A Sys category uses the managed-resource planner for managed items and the
bootstrap planner for init items. Fixture-backed planning may later replace individual assumptions
with declared observations while remaining non-applicable.

The command validates and plans from the same captured source snapshot. It routes before runtime
configuration initialization and update checks, never runs Preset code or processes, and never uses
real HOME or system mutation ports. Serialized output contains logical targets/resources, stable
diagnostic codes, steps, permissions, and named assumptions, but excludes private checkout paths,
content, argv, environment values, secret plaintext, and raw errors.

The authoring report's readiness describes only whether the hypothetical operation has blockers
under its stated assumptions. It is not evidence that the Preset will apply on another state or that
the user approved its permissions.

## Consequences

- Authoring feedback and real execution share one lifecycle/planner implementation.
- No authoring output can bypass the fresh Plan regeneration and exact approval checks required by
  mutation APIs.
- Missing env, trust, command, or state assumptions remain visible rather than being silently
  invented for a nicer preview.
- Fixtures can grow the state matrix without granting fixture files code execution.
- Public documentation must call the output an authoring preview or report, not a dry-run,
  approval, or executable Plan.
