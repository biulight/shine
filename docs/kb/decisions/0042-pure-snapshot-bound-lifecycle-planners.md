# 0042 — Lifecycle Plans are pure, snapshot-bound assessments

- **Status**: Accepted
- **Date**: 2026-08-29
- **Evidence**: `core/src/runtime/planner.rs`, `core/src/runtime/host.rs`

## Context

ADR 0040 defined the security Plan contract and ADR 0041 added target-local permission
declarations. App, Shell, and managed Sys still needed domain planners that could inspect the same
state as execution without inheriting write, process, privilege, or system-mutation capabilities.
Planning also has to remain deterministic and payload-free when live files, manifests, environment
inputs, generators, hooks, or Presets that disappeared after installation affect the result.

## Decision

Filesystem and split-DNS host ports are split into observation-only traits and mutation traits that
inherit them. The workspace-internal App, Shell, and managed Sys planner APIs are implemented only
for observation traits. This makes process execution, writes, removals, privilege escalation, and
split-DNS application unavailable to planner code at compile time.

Each planner selects and validates its target from immutable request/Preset input before observing
host state. It then produces `PlanV1` from the effective trust-layer-aware Preset digest and a
state digest containing only canonical target/logical-resource labels and hashes of relevant
manifests, receipts, live resources, launcher/profile/platform/mode decisions, and input
identities. Plain environment values contribute only hashes. Secret values require a caller-
supplied opaque handle or version; missing identity blocks the Plan and plaintext never enters the
digest or output.

Typed metadata and receipt ownership implicitly declare bounded Core effects. Explicit target-
local `[permissions]` declarations are merged with those effects and with known generator, hook,
administrator, and environment requirements. Missing declarations, unavailable secret identity,
or an opaque capability that cannot be determined fail closed.

Generators and hooks are not executed during planning. Their lifecycle trigger produces a
conservative `execute` step and any potentially affected resource step. Existing external-code
gates can still produce `blocked`. User modification and foreign ownership produce `preserve` or
`blocked`; `force` produces a distinct step/diagnostic and therefore a distinct fingerprint.
Uninstall may use a supported manifest or receipt after its original Preset disappears, but it
does not invent or execute teardown code that can no longer be resolved.

## Consequences

- Tests can run all three planner families with a host that cannot implement mutation.
- Plan review is bound to the exact observations that determined steps and permissions, while
  serialization remains free of content, argv, secret values, raw errors, and private checkout
  paths.
- Opaque generator output is deliberately conservative and may request review more often than a
  planner that executed code.
- CLI rendering, confirmation, re-planning, approval validation, and mutation enforcement remain a
  separate delivery slice; existing dry-run, status, and lifecycle execution behavior is unchanged.
