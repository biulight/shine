# 0063 — App JSON relocation is one key-owned two-destination transaction

- **Status**: Accepted
- **Date**: 2026-09-01
- **Evidence**: `core/src/action.rs`, `core/src/runtime/planner.rs`,
  `core/src/runtime/action_executor.rs`, `core/src/runtime/app.rs`

## Context

ADR 0062 moved static Copy relocation from independent install/remove calls into one receipt
transaction. App `json-merge` relocation retained the legacy path even though it replaces the same
source-keyed receipt across two destinations. A crash after creating the new JSON object but before
removing the old managed keys or persisting the replacement receipt could leave duplicate managed
keys, no reliable owner, or a recovery attempt that overwrote unrelated user settings.

Static Copy relocation cannot be reused. Shine owns only the declared top-level JSON keys, not
either whole object, and users or applications may change unrelated keys after interruption. The
old and desired managed-key sets may also differ in the same upgrade.

## Decision

Add `RelocateManagedJson` for an approved Upgrade Plan whose exact target/resource carries
`app_destination_relocated`. It binds the exact previous and desired receipts, old and absent new
destinations, the old canonical same-directory rollback path, optional previous whole-file
hash/mode, separate old/new managed-key sets and subset hashes, and old/new environment identities.
It contains no JSON values.

Execution writes the App operation journal before mutation. When the old destination exists, Core
moves its exact whole object to rollback material, writes the old object with only its previously
managed keys removed, then creates the new destination from the desired managed subset. The caller
atomically replaces the source receipt and commits only after that new receipt is durable.

Before receipt commit, explicit `shine app recover` first removes only the desired managed keys at
the new destination, then restores only the previous managed keys at the old destination from exact
rollback material. Unrelated values currently present at either destination survive. A new file is
removed only when it contains no unrelated keys. A missing old destination is supported without
rollback material. Any changed managed subset, invalid/non-object JSON, changed rollback
kind/hash/mode, occupied path, or conflicting receipt blocks all recovery mutation.

After the desired receipt is durable, both destination objects are in their final ownership state.
Recovery ignores later values at the now user-owned old destination, requires the desired managed
subset at the new destination to remain exact, and removes only exact old rollback material.

## Consequences

- JSON relocation no longer has an unjournaled two-destination/one-receipt window.
- Changing the managed-key set together with the destination is safe because old and new key
  identities are bound separately.
- Whole-file rollback material remains only a value source and fingerprint; recovery never restores
  the complete old object over current unrelated settings.
- Generators and remaining Shell/Sys resources retain their existing Phase 4 classifications.
