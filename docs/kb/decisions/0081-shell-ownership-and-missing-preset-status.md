# 0081 — Shell ownership and missing Preset status

- **Status**: Accepted
- **Date**: 2026-09-05

## Context

Inspection accepted receipt/overlay launcher targets while the planner used a narrower local
marker check. Commands remaining in the manifest after source deletion disappeared from CLI
inspection, and upgrade attempted to reconcile them against absent metadata.

## Decision

Use the existing observation-only launcher probe and the same category/overlay/receipt roots for
planning and inspection. Never relax marker, path, or paired-launcher checks merely to obtain a
ready Plan. Conflicts are attention items, not pending upgrades, in summary and detailed views.

Append inspection-only records for manifest commands absent from the effective platform-selected
Preset set. These records are not metadata for execution. Upgrade emits a preserve step and skips
execution for such commands; explicit receipt-driven uninstall remains available. Foreign launcher
conflicts still block. Shared external snapshot replacement is blocked if a missing installed
sibling remains in the category, since replacing that tree could delete its retained source.

Default update remains a managed-configuration assessment, not a full lifecycle Plan. App cache
maintenance stays in upgrade review with an explanation that cache counts are not configuration
update counts. Generator opt-in, approval, and all-domain preflight ordering remain unchanged.

## Consequences

Deleting source metadata does not silently uninstall a command. Users can restore its source or
explicitly uninstall it; unaffected categories may still upgrade. A category sharing an external
snapshot may require resolving missing installed siblings before it can converge. An invalid
remaining metadata file (for example a declaration referencing a deleted payload) still fails
validation rather than being silently accepted as a valid Preset.

Receipt-backed stale symlinks within the captured Shine/preset roots retain their upgrade repair
behavior. This compatibility rule does not widen ownership for regular launcher files or links
outside those roots.
