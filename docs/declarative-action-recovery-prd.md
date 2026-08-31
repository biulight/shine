# Declarative Action and Recovery PRD

> **Status:** Roadmap Phase 4 foundation in progress. Slices 4A, 4B, 4B.5, 4C.1, 4C.2a,
> 4C.2b-1, 4C.2b-2, 4C.2b-3a, 4C.2b-3b, 4C.2c, 4C.3, and 4C.4a are
> implemented: approved App install uses managed-file creation IR for absent or backup-eligible
> unowned static Copy destinations, and the explicit CLI recovery path can remove an
> unchanged transaction-created file or restore an unchanged fixed backup. Approved install and
> upgrade also journal in-place replacement of an unchanged, receipt-owned static
> Copy and retain its previous bytes only as same-directory transaction rollback material. Ordinary
> uninstall uses the same material for an unchanged, receipt-owned, unprivileged static Copy with
> no persistent backup until receipt removal and its journal commit marker are durable. Additional
> removal paths journal restoration of an unchanged fixed persistent backup and forced
> removal of a user-modified destination at the same static Copy boundary. Administrator removal
> reuses the same actions with a persisted privilege flag, administrator permission, locked
> privileged moves, and recovery authorization only when a protected path will change.
> Administrator static Copy creation, backup-aware creation, and in-place update now reuse the
> same actions, privileged mutation port, lock-spanning execution capability, and recovery rules.
> JSON merge now uses key-owned typed transactions that preserve unrelated current values during
> recovery without serializing prior or desired JSON payloads. App upgrade stale pruning reuses
> those receipt-gated static Copy and JSON removal transactions while preserving modified stale
> state.
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

Slice 4C.1 extends the same property to an occupied but unowned destination without storing its
bytes in the journal:

```text
prepared journal → rename original to fixed backup → atomic managed write → applied journal
       ↓                         ↓                              ↓
 original + no backup      missing + original backup      desired + original backup
       └──────────── explicit recovery restores only an exact safe state ────────────┘
```

App relocation, remaining Shell/Sys actions, and automatic resume remain later slices.

## Goals

1. Keep review semantics (`PlanV1`) and executable semantics (`ActionIrV1`) versioned and separate.
2. Derive exact permissions from typed declarative action kinds and fail opaque actions closed.
3. Persist a journal before the first action mutation and retain it until the owner receipt is
   durable.
4. Bind each journal to the exact original `PlanApprovalV1` for audit and later resume decisions.
5. Require a fresh, snapshot-bound recovery Plan before rollback.
6. Roll back a transaction-created file only while its bytes still match the recorded desired hash,
   and restore a transaction-created backup only while both paths match the recorded safe state.
7. Reject future journal/action schemas before any recovery mutation.
8. Exercise apply, interrupted write, recovery, and post-interruption user modification entirely
   against the in-memory host.

## Non-goals for the Action IR creation slice

- No implicit recovery during ordinary list, status, planning, install, upgrade, or uninstall.
- No serialized managed content, pre-operation content, environment values, or secret plaintext.
- No rollback of opaque code, package-manager operations, network effects, or privileged work
  outside the implemented App static Copy boundary.
- No global transaction across targets or domains.
- No promise that an internal Rust type is a stable third-party API.

## Contracts

### Security Plan

`PlanV1` remains the only review and approval contract. It describes semantic steps, binds exact
Preset/state snapshots, and resolves permissions. It never embeds `ActionIrV1` or journal payload.

### Action IR v1

`ActionIrV1` contains an operation identity and ordered typed actions. Its executable creation kinds
are:

- `CreateManagedFile`: canonical target/resource identity, resolved destination and desired content
  hash. The managed bytes are supplied separately after Plan approval and must match the hash.
- `CreateManagedFileWithBackup`: the same identity plus the fixed backup path and original content
  hash. It is valid only for an unowned regular-file static Copy destination with an
  absent backup.
- `UpdateManagedFile`: an unchanged receipt-owned static Copy destination, its fixed transaction
  rollback path, previous persistent backup identity, prior mode and original/desired hashes. It is
  valid only for an in-place update with an absent rollback path.
- `RemoveManagedFile`: an unchanged receipt-owned static Copy destination with no persistent backup,
  its fixed transaction rollback path, prior mode and original hash. It is valid for an ordinary
  uninstall or an approved stale-prune upgrade with an absent rollback path.
- `RemoveManagedFileWithBackup`: the same managed destination and transaction rollback identity,
  plus the canonical persistent backup and both files' prior modes and hashes. It is valid for an
  ordinary uninstall or approved stale-prune upgrade whose unchanged regular backup will be
  restored.
- `ForceRemoveManagedFile`: a user-modified, receipt-owned static Copy destination,
  its distinct receipt/current hashes and current mode, canonical rollback path, and optional
  canonical persistent backup identity. The Plan must explicitly review the modification override.
- `MergeManagedJson`: a created or in-place JSON merge, its declared unique top-level keys,
  canonical rollback path, optional whole-file before identity, and previous/desired managed-subset
  hashes. JSON values remain outside the action.
- `RemoveManagedJson`: ordinary, stale-prune, or forced removal of declared JSON keys, binding the
  whole-file before identity plus distinct receipt/current managed-subset hashes and canonical
  rollback path.

Each static Copy action also binds the receipt's `requires_admin` value. When true, it derives
Administrator permission and routes protected writes, moves, removals, and mode restoration through
the privileged host while retaining the same safe-state proof.

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
- ordered per-action `prepared` or `applied` state, plus `receipt-committed` only for removal after
  safe receipt absence is durable.

The journal is written atomically before file creation and again after it. It remains active until
the App lifecycle has durably saved the matching App receipt, then an explicit commit re-reads that
receipt and removes the journal. A new operation must refuse to replace an existing journal.

The creation slice reuses the host-provided cross-process operation lock for journal start, commit,
and recovery. The integrated App lifecycle preserves that serialization boundary.

For in-place managed update, the previous destination is renamed rather than copied to the fixed
same-directory `<name>.shine.rollback` transaction path. The action binds its path, previous App
backup identity, prior mode and before/after hashes, but not either byte payload. The rollback path
must be absent before planning and execution, is removed only after the replacement receipt is
durable, and is preserved if its kind or content changes.

Ordinary managed removal uses the same transaction path but commits through safe receipt absence:
while the exact old receipt remains, recovery restores unchanged rollback material; after no receipt
claims the source, destination or rollback path and the journal durably records `receipt-committed`,
recovery removes only unchanged rollback material. Receipt absence without that marker makes
recovery reconstruct the old payload-free receipt and roll back the unchanged file.
Backup-restoring removal first moves the managed destination to transaction rollback material, then
moves the persistent backup to the destination. Before receipt commit, recovery accepts only exact
`managed/original/missing`, `missing/original/managed`, or `original/missing/managed`
destination/backup/rollback states and recreates a missing old receipt before restoring both paths.
After `receipt-committed`, it keeps the exact user original at the destination and removes only
unchanged managed rollback material. Forced removal uses the same path transitions but binds the
different current user-modified hash separately from the receipt hash; before receipt commit it
restores that modified file and reverses an optional backup restoration, while after commit it
removes only exact modified rollback material. Administrator create, update, and removal reuse
these state machines: the host administrator lock covers revalidation through receipt commit and
cleanup, and every protected write/move/remove/mode operation uses the privileged mutation port.

### Recovery

Recovery is an explicit specialized `app-recovery` Plan. Planning hashes the exact journal bytes,
App manifest, and current destination/backup observations. It requests removal permission for the
journal and only the destination/backup write or removal permissions needed by the exact safe
rollback state.

Recovery outcomes are deliberately narrow:

| Matching receipt | Current destination | Recovery Plan | Apply |
|---|---|---|---|
| Yes | Any state | cleanup only | preserve destination; remove journal |
| No | Missing | cleanup only | remove journal |
| No | Present with desired hash | ready | remove destination, then journal |
| No | Present with another hash | blocked | preserve destination and journal |
| No | Opaque action | blocked | no automatic recovery |

Backup-aware creation adds this exact matrix when no matching receipt exists:

| Current destination | Current backup | Recovery Plan | Apply |
|---|---|---|---|
| Original hash | Missing | cleanup only | preserve destination; remove journal |
| Missing | Original hash | ready | rename backup to destination; remove journal |
| Desired hash | Original hash | ready | remove destination; rename backup back; remove journal |
| Any other combination or non-regular path | Any other combination or non-regular path | blocked | preserve both paths and journal |

Managed update uses the same three safe path states with `.shine.rollback` holding the previous
managed bytes and the previous receipt still durable. If the replacement receipt is durable,
recovery removes only an unchanged rollback file and the journal; a missing rollback permits
journal cleanup. Any changed rollback path, destination, or receipt blocks and preserves all state.

Backup-restoring removal uses three paths. Before receipt commit, recovery either keeps the exact
managed destination plus exact persistent backup, restores managed rollback while leaving the
backup in place, or moves an exact restored user destination back to `.shine.bak` before restoring
the exact managed rollback. After receipt commit it keeps the user destination and removes only
unchanged managed rollback material. Any changed path, mode, hash, or receipt blocks all mutation.

The approved recovery Plan is regenerated again while holding the operation lock immediately
before the first mutation.

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

### Slice 4B.5 — Explicit CLI recovery (implemented)

- Add `shine app recover [--yes]` after accepting its default behavior and exit semantics.
- Render a recovery-specific Plan with an explicit journal Remove or Preserve step.
- Keep ordinary App lifecycle operations blocked with actionable recovery guidance.
- Return non-zero without mutation for missing/invalid journals, user modifications, opaque
  actions, and unsupported schemas.
- Keep the recovery command available independently of the background release check.

### Slice 4C.1 — Backup-aware creation (implemented)

- Bind the fixed backup path and original/desired fingerprints without storing either payload.
- Block a pre-existing backup before approval and revalidate both paths under the operation lock.
- Recover interruptions before rename, after rename, after managed write, and after durable receipt.
- Preserve destination, backup, and journal if either path changes after interruption.

### Slice 4C.2a — Managed update (implemented)

- Add a typed in-place static Copy update with previous/desired fingerprints and prior mode.
- Move previous bytes to same-directory transaction rollback material without serializing them in
  the Action IR or journal.
- Integrate both approved install of an existing target and approved App upgrade.
- Recover before rename, after rename, after replacement write, and after replacement receipt;
  block any changed destination, rollback material, or receipt.

### Slice 4C.2b-1 — Ordinary managed uninstall (implemented)

- Reuse separate transaction-owned rollback material for managed remove without persisting secret
  plaintext casually.
- Bind the exact old receipt, destination kind/hash/mode and absent rollback path.
- Restore unchanged rollback material while the old receipt remains; after receipt removal is
  durable, record a positive journal commit marker before removing unchanged rollback material.
- Treat receipt absence without the positive marker as uncommitted: reconstruct the exact old
  receipt and roll back unchanged transaction state.

### Slice 4C.2b-2 — Backup-restoring managed uninstall (implemented)

- Support persistent user backup restoration with before/after fingerprints and post-operation
  modification protection.
- Move managed and user-original bytes through canonical transaction/persistent paths without
  serializing either payload.
- Recover before, between, and after both renames, plus both sides of receipt commit; block any
  changed path, mode, hash, or receipt.

### Slice 4C.2b-3a — Forced managed uninstall (implemented)

- Give forced removal of user-modified static Copy content a distinct reviewed action contract.
- Bind the old receipt separately from current modified mode/hash and optional persistent backup.
- Recover both sides of receipt commit without serializing user-modified or backup payloads.

### Slice 4C.2b-3b — Privileged managed uninstall (implemented)

- Bind persisted privilege identity and derive Administrator permission from removal actions.
- Hold the administrator lock across exact receipt/path checks, journaled moves, receipt commit,
  cleanup, and recovery.
- Ask for recovery elevation only when the exact recovery Plan mutates a protected path.

### Slice 4C.2c — Privileged managed create and update (implemented)

- Bind persisted privilege identity and derive Administrator permission from create, backup-create,
  and update actions.
- Hold the administrator lock across exact path checks, journaled writes/moves, receipt commit,
  rollback cleanup, and recovery; restore the prior mode through the privileged host.
- Ask for recovery elevation only when the exact safe state changes a protected path; receipt-only
  cleanup remains unprivileged.

### Slice 4C.3 — JSON merge actions (implemented)

- Define key-level ownership and rollback semantics without overwriting unrelated user changes.
- Keep prior and desired JSON payloads out of the Action IR and journal.
- Cover install, update, forced removal, receipt commit, and every interruption boundary before
  admitting JSON merge to the action executor.

The implemented actions move an existing whole JSON file to exact same-directory rollback material
but restore only declared top-level keys into the current object. This preserves unrelated values
changed after interruption. Creation at an absent path removes the whole destination only when it
contains no unrelated keys; committed uninstall preserves the now user-owned JSON object and
cleans only exact rollback material.

### Slice 4C.4a — App upgrade stale-prune removal (implemented)

- Reuse the existing static Copy, backup-restoring, administrator, and JSON removal actions for an
  unchanged stale receipt selected by an approved `upgrade --prune-stale` Plan.
- Bind destination, optional fixed backup, canonical rollback, journal, and manifest effects using
  removal permissions even though the outer lifecycle operation is Upgrade.
- Preserve user-modified stale content and never infer force; treat a missing destination as an
  atomic receipt-only cleanup.
- Save receipt absence and positive journal commit evidence before removing exact rollback material;
  recover an interrupted prune through the existing `shine app recover` Plan.

### Slice 4D — Other domains and opaque inventory

- Shell launcher/profile declarative actions. First-time launcher creation and explicit recovery are
  implemented as Slice 4D.1. Unchanged receipt-owned launcher update is implemented as Slice 4D.2;
  unchanged receipt-owned launcher removal with positive receipt-commit evidence is implemented as
  Slice 4D.3. Shared snapshot/render state and profile sentinel blocks remain.
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

Slice 4A is internal and adds no public commands or behavior. Slice 4B.5 releases the explicit
recovery command and guidance. Slice 4C.1 expands recovery to fixed backups and blocks an occupied
backup rather than replacing it. Slices 4C.2b-1 and 4C.2b-2 expand the same recovery guidance to
receipt removal and fixed-backup restoration. Slice 4C.2b-3a adds forced-removal rollback and the
same documentation remains aligned across both public manual locales. Slices 4C.2b-3b and 4C.2c
document administrator-path authorization and recovery timing in both locales. Slice 4C.3 adds
key-owned JSON merge recovery guidance to both locales. Slice 4D.1 adds first-time Shell launcher
creation recovery and `shine shell recover` guidance to both locales. Slice 4D.2 expands that Shell
recovery guidance to receipt-owned launcher replacement and same-directory rollback material in
both locales. Slice 4D.3 extends it to launcher removal, including receipt reconstruction when the
manifest write is durable but the positive removal commit marker is not. Slice 4C.4a applies the
same App removal recovery guidance to `upgrade --prune-stale` in both locales.
