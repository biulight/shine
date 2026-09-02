# 0043 — CLI lifecycle mutation requires snapshot-bound approval

- **Status**: Accepted
- **Date**: 2026-08-30
- **Evidence**: `cli/src/lifecycle_plan.rs`, `core/src/runtime/planner.rs`

## Context

ADR 0040 defined the Plan/approval contract, ADR 0041 added target-local permission declarations,
and ADR 0042 made App, Shell, and managed Sys planning observation-only. The existing CLI could
still invoke mutation helpers directly, re-evaluate interactive stale deletion during App upgrade,
and synchronize the Sys profile outside the reviewed managed-resource operation. A reviewed Plan
therefore did not yet constrain execution.

## Decision

App, Shell, and managed Sys install, upgrade, and uninstall enter Core through approved execution
methods. Each method regenerates the Plan from a fresh Preset snapshot, captured configuration,
opaque input identities, manifests/receipts, and live host state, then validates both the exact
fingerprint and complete permission set before invoking the existing executor. The unapproved
mutation helpers remain internal implementation details.

The CLI renders every ready Plan with ordered steps, required permissions, blockers, input digests,
and fingerprint. [ADR 0071](0071-compact-upgrade-plan-presentation.md) later refined the default
`upgrade` presentation into a grouped compact review while retaining the full rendering under
`--verbose`. Approval is process-local and one-shot. Interactive confirmation defaults to No;
non-interactive mutation requires command-level `--yes`. That flag skips only the prompt: it does
not skip rendering, blockers, missing declarations, or fresh validation. Existing `--dry-run`
paths retain their preview semantics and conflict with `--yes`; they do not create an approval or
claim to be a security Plan.

Untargeted `shine upgrade` reviews Shell, App, and enabled managed Sys Plans as one batch and
validates all three before the first protected mutation. `upgrade --pull` keeps pull, reload, final
planning, approval, and apply ordering. Stale App deletion is bound only by `--prune-stale`; the
executor cannot ask to expand it later. Administrator authorization remains a separate interaction
after Plan approval. The aggregate command no longer synchronizes the composed Sys profile outside
the managed Sys Plan; explicit `sys profile enable/disable` remains unchanged.

Artifact apply/remove, App refresh, and explicit Sys profile operations were excluded from this
slice until [ADR 0045](0045-specialized-app-and-profile-security-plans.md) added their dedicated
operation contracts. Sys bootstrap joins through the specialized contract in
[ADR 0044](0044-sys-bootstrap-uses-dedicated-security-plan.md). ADR 0045 also made available App
teardown reviewable during uninstall.

## Consequences

- A changed source, live state, ordered step, or permission set aborts before the approved executor
  mutates its target.
- External Presets missing required permission declarations now fail closed for protected
  mutations, while `allow_app_hooks`, `allow_sys_code`, ownership checks, and administrator
  authorization remain additional gates.
- Automation must pass `--yes`; add `--verbose` when it needs the complete unabridged Plan on stdout.
- Approval is not persisted, reused as a grant, or automatically retried after a mismatch.
