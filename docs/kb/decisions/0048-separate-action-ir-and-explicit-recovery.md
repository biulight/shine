# 0048 — Executable actions and recovery stay separate from the security Plan

- **Status**: Accepted
- **Date**: 2026-08-30
- **Evidence**: `core/src/action.rs`, `core/src/runtime/action_executor.rs`,
  `core/src/runtime/planner.rs`, `core/src/runtime/app.rs`,
  `docs/declarative-action-recovery-prd.md`

## Context

Phase 3 established `PlanV1` as a payload-free review and approval contract. Executing it directly
would either make semantic Plan steps ambiguous or force file bytes, argv, private paths, and
rollback data into a contract designed not to carry them. Conversely, recovering an interrupted
operation implicitly during ordinary planning would mutate state without a recovery-specific
snapshot and permission review.

App managed-file creation provides the smallest useful Phase 4 slice. A crash can occur after the
destination is written but before its manifest receipt is durable. Blind cleanup could then delete
a file the user changed after the crash, while blindly committing the receipt could claim ownership
of unexpected bytes.

## Decision

Core owns a versioned `ActionIrV1` distinct from `PlanV1`. An Action IR contains ordered typed
actions, resolved effect identities, and content hashes, but not managed bytes, environment values,
secret plaintext, or raw argv. Typed actions derive their required permissions. Opaque execution is
represented explicitly with provenance, privilege and unsupported rollback metadata, and generic
permission derivation fails closed.

The first executable action is creation of one previously absent, unprivileged App managed file.
The caller supplies bytes separately and Core verifies their stable hash against the Action IR.
Updates, JSON merge, administrator destinations and opaque execution are not accepted by this slice.

Before the first file mutation, Core atomically writes a schema-v1 App operation journal containing
the payload-free Action IR, original `PlanApprovalV1`, and per-action state. The journal is updated
after apply and remains until the App receipt owner explicitly commits it. A pre-existing journal
blocks another operation instead of being overwritten. Journal start, commit and recovery use the
host-provided cross-process operation lock.

Approved App install planners add the journal's write/remove infrastructure permissions only for
eligible absent-destination creation. After exact approval validation, the planner emits a
deterministic one-action IR bound to the Plan fingerprint, logical App resource, resolved
destination, and desired hash. Real App install executes that IR, saves its matching manifest
receipt immediately, and then asks the executor to commit. Commit re-reads the manifest and refuses
to remove the journal unless the Copy receipt matches source, destination, hash, privilege, and
backup state.

Recovery uses the specialized `app-recovery` Plan operation. Its state snapshot binds the exact
journal bytes, App manifest, and destination observation, and its permission set includes every
removal it may perform. Apply replans under the operation lock before mutation. A transaction-created file is
removed only if it still matches the recorded desired hash; a missing file permits journal cleanup,
while changed bytes or opaque actions block and preserve all state. If the matching receipt was
already durable before interruption, recovery treats ownership as committed, preserves the
resource, and removes only the stale journal.

## Consequences

- Plan review and action execution can evolve without weakening the Phase 3 payload boundary.
- Interrupted mutation becomes durable and testable without making ordinary read paths mutating.
- Rollback permission is separately reviewable rather than silently inherited from the original
  write approval.
- The first slice proves safe rollback only for transaction-created files; it does not imply update,
  uninstall, administrator, package-manager, or cross-target transactions.
- Journal files are ownership evidence comparable to manifests: future actions must validate schema,
  target scope, fingerprints and current bytes before mutation.
- Released recovery UX and automatic resume remain separate decisions.
