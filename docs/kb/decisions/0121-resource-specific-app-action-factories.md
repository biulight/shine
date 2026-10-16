# ADR 0121: Resource-specific App Action factories

- **Date**: 2026-10-17
- **Status**: Accepted

## Context

The approved static App factory mixed JSON/Copy relocation, JSON merge, and Copy creation/update in
one method. Each branch paired Plan admission with live source, destination, backup and rollback
checks, then constructed payload-free Action IR. The long method obscured those different contracts.

## Decision

- Keep `app_actions.rs` as the approval/routing facade. Validate approval before Host reads; retain
  operation selection, generator exclusion, per-file transform/hash timing and initial manifest load.
- Pass one private owned per-file input to four observation-only handlers: JSON relocation, Copy
  relocation, JSON merge, and Copy creation/update. The input borrows the same request/Plan/manifest/
  file and holds desired bytes only during construction; it is not cached execution authority.
- Preserve dispatch order, admission predicates, read/error order, diagnostics and skip behavior.
  Share target/resource/action step matching, with an optional exact typed-kind restriction only
  for relocation. Do not make ordinary Create/Update depend on a new step-kind requirement.
- Wrap emitted Actions at one point with the existing IDs, fingerprint and per-file ordering.
  Keep uninstall and appended stale-prune on their existing receipt-driven factory paths, including
  manifest reloads. Retain all Action fields, rollback contracts and permission derivation.

## Consequences

Commands, Plans/fingerprints, receipts, journal schemas and execution/recovery flows are unchanged.
Resource helpers are private and require only `FileSystemObservationHost`. The approved lifecycle
still owns the lock and regenerates the Plan; the executor still validates Action authority and
permissions, rechecks live inputs and journals before effects. No managed bytes or env values enter
Action IR, and an absent Action never authorizes fallback mutation.

Regressions exercise mixed create/backup/update/JSON dispatch, exact IDs/order/permissions, preserved
and generated exclusions, Copy relocation modes/backup/admin/env identities, JSON old/new managed
keys, stale-prune ordering, typed relocation intent, occupied late destinations/rollback paths and
approval rejection before observations. Existing transaction/recovery tests retain per-resource
interruption, receipt, ownership, fresh approval and privilege coverage.
