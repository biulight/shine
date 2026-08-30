# Declarative Action and Recovery PRD

> **Status:** Roadmap Phase 4 foundation in progress. Slices 4A and 4B are implemented: approved
> App install uses the managed-file creation IR for absent, unprivileged static Copy destinations.
> This document is internal and does not define released CLI behavior.

## Summary

Phase 3 made every mutation pass through a reviewable, snapshot-bound security Plan. A Plan is not
an executor: it deliberately contains no file bytes, argv, environment values, secret plaintext,
or generic action payload. Phase 4 adds a separate versioned Declarative Action IR and a durable
operation journal so an approved operation can be recovered after interruption without treating a
stale Plan as authorization or overwriting later user changes.

The first slice proves one complete safety property for App files:

```text
approved Plan
    ↓
ActionIrV1(CreateManagedFile; content hash, no bytes)
    ↓
prepared journal → atomic file creation → applied journal
    ↓                                      ↓
durable receipt                       crash / interruption
    ↓                                      ↓
clear journal                     explicit AppRecovery Plan
                                           ↓
                       receipt exists → clean journal only
                      no receipt → remove only if unchanged
```

It intentionally supports only creation at an absent, unprivileged App destination. App updates,
backup restoration, JSON merge, administrator paths, Shell/Sys actions, automatic resume, and CLI
recovery UX remain later slices.

## Goals

1. Keep review semantics (`PlanV1`) and executable semantics (`ActionIrV1`) versioned and separate.
2. Derive exact permissions from typed declarative action kinds and fail opaque actions closed.
3. Persist a journal before the first action mutation and retain it until the owner receipt is
   durable.
4. Bind each journal to the exact original `PlanApprovalV1` for audit and later resume decisions.
5. Require a fresh, snapshot-bound recovery Plan before rollback.
6. Roll back a transaction-created file only while its bytes still match the recorded desired hash.
7. Reject future journal/action schemas before any recovery mutation.
8. Exercise apply, interrupted write, recovery, and post-interruption user modification entirely
   against the in-memory host.

## Non-goals for the current creation slice

- No new App/Shell/Sys command, prompt, or terminal output.
- No implicit recovery during ordinary list, status, planning, install, upgrade, or uninstall.
- No serialized managed content, pre-operation content, environment values, or secret plaintext.
- No rollback of opaque code, package-manager operations, network effects, or administrator work.
- No global transaction across targets or domains.
- No promise that an internal Rust type is a stable third-party API.

## Contracts

### Security Plan

`PlanV1` remains the only review and approval contract. It describes semantic steps, binds exact
Preset/state snapshots, and resolves permissions. It never embeds `ActionIrV1` or journal payload.

### Action IR v1

`ActionIrV1` contains an operation identity and ordered typed actions. The first executable kind is:

- `CreateManagedFile`: canonical target/resource identity, resolved destination and desired content
  hash. The managed bytes are supplied separately after Plan approval and must match the hash.

The classification-only escape hatch is:

- `OpaqueExecution`: capability, embedded/external/overlay provenance, administrator flag, and an
  explicit unsupported rollback reason. Its permissions are uncomputable until a later typed
  contract provides them, so generic declarative execution fails closed.

Action permission derivation covers only action effects. Journal and receipt permissions are
Core-owned infrastructure permissions added by the corresponding security planner.

### App operation journal v1

The initial journal lives at `<shine_dir>/app-operation-journal.toml` and contains:

- schema version;
- full payload-free Action IR;
- original Plan approval fingerprint and exact permission set;
- ordered per-action `prepared` or `applied` state.

The journal is written atomically before file creation and again after it. It remains active until
the App lifecycle has durably saved the matching App receipt, then an explicit commit re-reads that
receipt and removes the journal. A new operation must refuse to replace an existing journal.

The creation slice reuses the host-provided cross-process operation lock for journal start, commit,
and recovery. The integrated App lifecycle preserves that serialization boundary.

### Recovery

Recovery is an explicit specialized `app-recovery` Plan. Planning hashes the exact journal bytes,
App manifest, and current destination observation. It requests removal permission for the journal
and, only when no matching receipt exists and the transaction-created destination still matches its
desired hash, removal permission for that destination.

Recovery outcomes are deliberately narrow:

| Matching receipt | Current destination | Recovery Plan | Apply |
|---|---|---|---|
| Yes | Any state | cleanup only | preserve destination; remove journal |
| No | Missing | cleanup only | remove journal |
| No | Present with desired hash | ready | remove destination, then journal |
| No | Present with another hash | blocked | preserve destination and journal |
| No | Opaque action | blocked | no automatic recovery |

The approved recovery Plan is regenerated again while holding the operation lock immediately
before the first removal.

## Delivery sequence

### Slice 4A — Foundation (implemented)

- Versioned Action IR and deterministic serialization tests.
- Typed permission derivation and opaque fail-closed classification.
- Versioned App journal and explicit recovery Plan.
- Managed-file create/commit/rollback harness against `InMemoryHost`.
- Injected interruption between destination creation and applied-journal persistence.
- User-modification blocking test.

### Slice 4B — App lifecycle integration (implemented)

- Planner includes journal infrastructure effects and emits the exact Action IR after approval.
- App install persists each receipt before journal commit.
- Commit re-reads the matching receipt; receipt-write failure leaves the journal recoverable.
- Existing output, hooks, manifests, backup semantics, and lifecycle results remain compatible.
- Add CLI recovery presentation only after its default behavior and exit semantics have an accepted
  decision.

### Slice 4C — Managed update and uninstall

- Introduce transaction-owned rollback material without persisting secret plaintext casually.
- Support backup-aware creation over an unowned destination.
- Support managed update/remove with before/after fingerprints and post-operation modification
  protection.
- Add administrator locking and rollback tests before privileged actions join the IR.

### Slice 4D — Other domains and opaque inventory

- Shell launcher/profile declarative actions.
- Managed Sys files and split-DNS typed actions.
- Sys package/provider and executable code classification.
- Migrate or explicitly classify every built-in executable Preset listed in
  `docs/kb/executable-preset-inventory.md`.

## Acceptance for Phase 4 completion

The Roadmap Phase 4 gate remains stricter than Slice 4A:

- fully declarative Presets produce stable actions and Plan semantics for identical inputs;
- every integrated action journals before mutation and can safely resume or roll back;
- rollback never overwrites bytes changed after the operation;
- opaque/non-reversible work is visible before execution;
- all built-in executable Presets are migrated or classified by execution, privilege, provenance,
  and rollback support;
- crash injection covers every action boundary used by released lifecycle commands.

## Documentation impact

Slice 4A is internal and adds no public commands or behavior, so the English and Simplified Chinese
manuals remain unchanged. Any released recovery command, new diagnostic, or changed install flow
must update both locales in the same change.
