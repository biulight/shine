# 0046 — External Preset code trust is snapshot-scoped and target-local

- **Status**: Accepted
- **Date**: 2026-08-30
- **Evidence**: `core/src/trust.rs`, `cli/src/trust.rs`, `core/src/runtime/planner.rs`

## Context

Security Plans now bind every supported mutation to exact source, state, steps, and permissions,
but external App and Sys code is still controlled by the global `allow_app_hooks` and
`allow_sys_code` booleans. Those switches trust unrelated targets and future code changes at once.
Preset permission declarations cannot replace that decision: they are author claims, while Plan
approval is a single-operation authorization rather than durable trust in opaque code.

## Decision

Shine stores versioned external-code grants in the global Shine state. A grant is target-local and
binds a capability kind, the effective logical code inputs and their trust layers through a digest,
and the exact declared permission set. A changed target, capability, code input, source layer, or
permission set does not match the grant.

Preset and project configuration cannot create grants. The CLI may persist a grant only after it
derives the current requirement from the immutable Preset snapshot, renders the scope, and receives
explicit confirmation. Non-interactive enrollment requires an explicit acknowledgement flag.

Grant matching is Core-owned and frontend-neutral. A matching grant only permits opaque external
code to remain in a ready Plan. Every mutation still requires a freshly regenerated,
snapshot-bound `PlanApprovalV1`; permission declarations remain descriptions and do not grant
execution. Embedded code continues to rely on the installed Shine distribution provenance.

Legacy coarse booleans are read only for migration diagnostics. They are never converted into
grants automatically and cannot authorize new execution.

## Consequences

- Trusting one target cannot authorize another target or later code.
- Permission expansion and source-layer changes require a new grant.
- The trust store contains identities and digests, never code, argv, environment values, secrets,
  or physical checkout paths.
- Opaque code is not sandboxed by the declaration; the UI must describe the grant as trust rather
  than enforcement.
