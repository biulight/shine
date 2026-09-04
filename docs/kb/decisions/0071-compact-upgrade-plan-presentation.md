# 0071 — Upgrade Plans use compact scope-aware presentation

- **Status**: Accepted
- **Date**: 2026-09-02
- **Evidence**: `cli/src/lifecycle_plan.rs`, `cli/src/list.rs`

## Context

The snapshot-bound Plan contract deliberately records every ordered semantic step, exact permission
identity, input digest, and fingerprint. Untargeted `shine upgrade` reviews Shell, App, and enabled
managed Sys Plans before its first mutation. Rendering all three contracts verbatim made ordinary
reviews dominated by unchanged steps, per-file embedded Preset-cache convergence, repeated labels,
and long digests. Meanwhile `shine update` could identify one user-facing target but still recommend
the broader aggregate command, making unrelated maintenance look like additional updates.

The complete Plan remains valuable for auditing and debugging, but its storage and approval
contract does not require every terminal view to give every field equal visual weight.

## Decision

Default `upgrade` presentation renders one batch heading with scope sections for Shell, App, and
managed System configuration. It keeps mutation, preserve, blocked, missing-declaration, and
uncomputable-permission state explicit; groups exact required permission identities by capability;
counts ordinary no-op steps; summarizes consecutive per-category Preset-cache steps by action; and
uses shortened display forms for snapshot and Plan identities. Lifecycle Plan renderers use a shared
semantic tone map for action prefixes and grouped action counts: create is green, update and
preserve are yellow, remove and blocked are red, execute is cyan, and unchanged is dim. Targets,
resources, permission identities, and diagnostic codes remain unstyled. Output automatically falls
back to the same plain text when stdout does not support color or color is disabled. The underlying
`PlanV1`, approval fingerprint, permission validation, fresh re-planning, and all-or-nothing
preflight ordering are unchanged.

`upgrade --verbose` retains the unabridged rendering with every ordered step and full identity.
Other lifecycle operations keep their existing full presentation. A blocked aggregate review
collects all Plans before reporting and adds actionable guidance for missing Preset declarations and
untrusted external App code without exposing private paths or command arguments.

When untargeted `shine update` finds exactly one canonical category or managed Sys target, its hint
recommends `shine upgrade <TARGET>`. Multiple targets retain the aggregate command.

## Consequences

- The common one-update path does not unexpectedly traverse unrelated installed targets.
- Global reconciliation and its single approval remain available and retain the same safety gates.
- Default output is intentionally a compact projection, not a serialized Plan API; automation that
  needs complete audit text uses `--verbose`.
- Color is presentation-only and never enters Plan serialization, fingerprints, approvals, or
  hypothetical Preset authoring reports.
- Renderer tests must cover scope grouping, no-op/cache summaries, exact permission identities,
  shortened display identities, and actionable blocker messages.
