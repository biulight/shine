# 0086 — Bootstrap security review retains permission provenance

- **Status**: Accepted
- **Date**: 2026-09-13
- **Evidence**: `core/src/plan.rs`, `core/src/runtime/planner.rs`, `cli/src/lifecycle_plan.rs`

## Context

A flat permission union obscured which selected installer required administrator access, broad
network access, or a command. Grouping the union in the CLI cannot recover that provenance.

## Decision

Plan v1 adds optional `permission_scopes`: ordered target-local permission resolutions, followed by
shared effects with a null target. Existing Plans omit the empty field. Sys bootstrap captures each
item's declaration, detection, environment, and typed installer effects before merging them into the
existing aggregate resolution. Profile effects have their own scope; runtime materialization and
run-manifest writes are shared. The scope union equals the aggregate required set. Permissions needed
by several items remain present in each scope; no permissions are inferred from command names.

Scopes contain only existing safe capability identities and diagnostics. Serialized scopes enter the
Plan fingerprint, so provenance changes invalidate approval. The operation still uses one approval,
the exact aggregate permission set, fresh re-planning, and all existing trust/ownership gates.

Default bootstrap review shows attention messages, ordered items with action markers and permissions,
profile configuration, shared changes, then abbreviated identities. Blockers remain visible. The CLI
falls back to full aggregate output when attribution is incomplete. `sys bootstrap --verbose` keeps
the grouped layout with complete identities and diagnostic codes; dry-run stays on its earlier path.

## Consequences

- Empty declarations still receive metadata-derived package-provider capabilities.
- Shared internal writes are shown once, without losing target-local permission reuse.
- Other operations retain their existing presentation and omit the new field when unused.
- Tests cover provenance, union completeness, snapshot binding, safe serialization, blockers,
  and compact/verbose rendering without running installers.
