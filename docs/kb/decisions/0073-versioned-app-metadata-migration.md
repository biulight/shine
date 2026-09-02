# 0073 — Version App metadata independently from runtime state

- **Status**: accepted
- **Evidence**: `core/src/runtime/{app_metadata,planner}.rs`, `cli/src/lifecycle_plan.rs`

## Context

Shine 2 changed the lifecycle security boundary: App artifacts have their own reviewed Plan and a
lifecycle hook must not recursively invoke `shine app artifact apply`. Older full-copy overlays can
still replace the built-in `app/<category>/shine.toml`, making missing permission declarations and
external-code trust errors appear before users understand that the active metadata is obsolete.

## Decision

App `shine.toml` has a root `metadata_schema_version`; absent means legacy v1 and current built-in
metadata declares v2. This is independent of `[permissions].schema_version`. When legacy metadata
contains a recursive artifact-apply hook, planning blocks it with a stable legacy-metadata
diagnostic before collecting hook permissions or checking hook trust. Overlay-origin metadata gets
its own code so the CLI can explain that only the overriding `shine.toml` needs migration or removal
while payload files remain intact.

`shine state migrate` remains limited to Shine-owned runtime state, receipts, and caches. It never
rewrites user Preset sources or overlays. `shine preset migrate` provides the separate explicit,
dry-run-capable, reviewed conversion and backup contract defined by
[ADR 0074](0074-reviewed-preset-source-migration.md); `update`, `upgrade`, and `self upgrade` never
apply it implicitly.

## Consequences

- Users receive a deterministic migration path instead of a misleading request to grant trust or
  add a permission declaration.
- Existing custom metadata without recursive artifact hooks remains compatible as legacy v1.
- Future App metadata changes can be introduced without using the installed Shine binary version as
  a proxy for source compatibility.
