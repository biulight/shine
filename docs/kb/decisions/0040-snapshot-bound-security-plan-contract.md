# 0040 — Security approval binds a reviewable Plan to exact snapshots

- **Status**: Accepted
- **Date**: 2026-08-29
- **Evidence**: `core/src/plan.rs`, `core/src/runtime/preset.rs`,
  `docs/security-plan-trust-prd.md`

## Context

Lifecycle Contract v1 describes execution outcomes, and Phase 2 Core assessments preserve existing
status and dry-run behavior. Neither is an authorization boundary: dry-run may share executor
paths, App status may run an automatic generator, and current assessments are not bound to all
source and live-state inputs. Reusing them as a security Plan would permit review and execution to
observe different Preset bytes, state, or permissions.

Roadmap Phase 3 also requires explicit permission derivation. Existing `allow_app_hooks` and
`allow_sys_code` grants are coarse compatibility gates, not a complete filesystem, network,
command, administrator, env/secret, and system permission declaration.

## Decision

`shine-core` owns a versioned `PlanV1` contract distinct from `LifecycleResultV1` and dry-run. Plan
v1 initially covers the existing lifecycle operation vocabulary for App, Shell, and managed Sys.
Specialized bootstrap, artifact, refresh, and profile operations will join through a later
operation-contract decision rather than being encoded as ambiguous lifecycle actions.

A Plan contains ordered semantic steps, captured input digests, and permission resolution. Steps
are review descriptions only and cannot carry executable argv, content, environment values, secret
plaintext, or a generic Action IR. Permission derivation fails closed: missing declarations and
uncomputable requirements make the Plan non-ready, as does any blocked step.

Preset and state snapshots use domain-separated SHA-256 over sorted, length-framed observations.
The Preset digest binds effective logical paths, bytes, and embedded/external/overlay identity, but
not a machine-local checkout root. Future state planners must bind every observation that affects
steps or permissions and must represent secrets through opaque handles or versions, never decrypted
plaintext.

Frontend approval produces `PlanApprovalV1` only after reviewing a ready Plan. Approval binds the
exact Plan fingerprint and exact required permission set. A future apply path must capture current
inputs, regenerate the Plan, and validate the approval before any mutation. Any source, state,
step, or permission change rejects the approval; permission expansion always requires review.

The contract foundation does not change current command routing. Existing external-code gates stay
in force until a later migration defines permission declaration syntax, updates built-ins, and
preserves compatibility without silent privilege expansion. Auto-generator execution during
read-oriented status remains subject to the separate ADR required by the roadmap.

## Consequences

- A Plan cannot be substituted with existing dry-run or lifecycle result output.
- CLI and future UI approval can share one deterministic Core contract.
- The first slice is intentionally non-enforcing; current mutations remain unchanged until pure
  planners and apply validation land together per domain.
- Phase 4 action execution, journal, rollback, and recovery remain separate from review semantics.
- New planner code must expose read-only inputs and cannot gain process, write, privileged, or
  external-code capabilities merely because an executor host already has them.
