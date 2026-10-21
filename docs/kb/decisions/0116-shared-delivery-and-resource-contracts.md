# ADR 0116: Share delivery facts and make resource phases explicit

- **Date**: 2026-10-17
- **Status**: Accepted

## Context

Shell inspection, planning and receipt construction independently described desired delivery.
Native PowerShell receipts recorded `native-cmd-v2`, while planning compared only `live-bun-v2`
or absence. Large removal and Sys methods mixed resource phases. App commit used independent
variant checks, and Action validation/permission rules mixed unrelated resource families. Broker
policy, storage and terminal commands shared a module and duplicated persistence and reserved keys.

## Decision

- Remove permanently disabled Sys v1 dispatcher tests. Keep current v2 behavioral coverage in its
  owning modules and migrate the duplicate-item/default-profile/reference validation cases to Core.
- Keep the broker facade and DTOs stable; separate model, pure policy, private store and CLI commands.
  Reuse Core private atomic replacement and one reserved SSH environment-key list. Preserve file
  validation, mutation-lock lifetimes, exact post-confirmation checks and all-declared expansion.
- Build Shell source layout and desired delivery from captured inputs without Host I/O. Inspection,
  planning, launcher preparation and receipt construction share these facts. Build delivery at the
  original Bun-policy assessment point, retaining error/observation order. Planning recognizes both
  native-cmd-v2 and live-bun-v2 receipts; receipt reconstruction still validates persisted input.
- Separate Shell removal into legacy/receipt launchers, rendered output, purge/cache and snapshots;
  separate command rendering from launcher convergence. Retain original step, observation and
  permission order, short circuits and transaction flags. Facts stay within a single assessment.
- Separate managed Sys selection, preflight, resource execution and result persistence. Distinguish
  current items from receipt-only removal. The approved facade keeps the lifecycle lock and fresh
  approval gate; per-resource receipt save still precedes commit and journal cleanup.
- Convert App commit actions to a borrowed domain view with exhaustive conversion and dispatch.
  Explicitly handle creation without rollback cleanup, and reject non-App actions. File and JSON
  handlers retain live identity checks and remove the journal only after every cleanup succeeds.
- Classify Action resource families exhaustively before dispatching validation and permission rules
  to private resource contracts. Incompatible handler input fails validation or marks permissions
  uncomputable. Retain DTOs, diagnostics, rollback pairings and opaque fail-closed behavior.
- Represent file-group seal/run/export and preview output with enums. Preserve metadata-only
  previews, stdout/stderr selection, input limits and absence checks, including repeat preflights.

## Consequences

Public commands, wire formats, receipts and journal schemas remain unchanged. Structural extractions
preserve Plan ordering and fingerprints; the native-cmd-v2 current-state correction changes affected
Windows upgrade Plans and requires fresh review. No generic transaction engine or cached execution
authority is introduced. Regression coverage includes delivery convergence, cross-resource rollback
and exact permission contracts, foreign-domain App commit rejection and partial Sys batch recovery.

Review follow-up exercises twelve delivery combinations (embedded, external snapshot and live;
Native/Bun; transformed/untransformed), including actual live-bun-v2 receipts and repeated-upgrade
convergence. Managed Sys removal tests cover both current presets and receipt-only state: preview
never requests administrator authorization, and denied authorization preserves bytes and receipts
without resource mutations or journal publication.
