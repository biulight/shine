# 0037 — Lifecycle operations produce versioned structured results

- **Status**: Accepted
- **Date**: 2026-08-28
- **Evidence**: `utils/src/lifecycle.rs`, `cli/src/apps/{install,upgrade,uninstall,hooks,build}.rs`,
  `cli/src/shells/{install,uninstall,deployment}.rs`, `cli/src/sys/{managed,resources,run_manifest}.rs`,
  `cli/src/install_core/manifest.rs`

## Context

App, Shell, and Sys already implement safe lifecycle behavior, but expose unrelated outcome types
and frequently render terminal output from the same function that mutates state. Terminal prose is
therefore the only common observable contract, which cannot be reused safely by a future UI or by a
CLI-independent Rust harness.

`preset validate` already demonstrates a versioned machine-readable report. Runtime manifests,
however, have evolved through optional fields without a shared rule for legacy, current, and future
shapes. Roadmap Phase 2 depends on a stable Phase 1 lifecycle seam before product logic can move
behind `shine-core`.

## Decision

`shine-core` owns a serializable `LifecycleResultV1` envelope and its operation, status, effect,
outcome, and derived-summary types. The contract has no dependency on Clap, terminal rendering,
runtime Config, process execution, or filesystem access.

Every outcome uses the canonical lifecycle target identity and may include a logical resource name.
The common contract describes cross-domain facts while App, Shell, and Sys retain their own planning,
receipt, and execution models.

Reusable results contain structured effect and diagnostic codes only. They do not contain arbitrary
error prose, raw logs, source/destination content, environment or secret values, or absolute
destination paths. Human rendering remains a frontend responsibility even while the first migration
slices temporarily generate results alongside existing inline output.

Before any public JSON surface existed, Contract v1 added `pending`. Read-only `update` has
`dry_run = false` and reports applicable work as `pending`; explicit dry-run has `dry_run = true`
and reports `previewed`. `changed` is reserved for an execution that actually changed Shine-owned
state. Dry-run is not called a Plan and carries no approval or snapshot guarantee. The reviewable
Plan contract remains a separate Roadmap Phase 3 decision.

The App, Shell, and Sys runtime manifests adopt a top-level `schema_version` independently. A
missing version is legacy v0, supported legacy state normalizes in memory and upgrades on the next
successful write, and an unsupported future version fails before mutation. An incompatible shape
or semantic reinterpretation requires a version bump. Sys resource receipt versions remain
independent of the container manifest schema.

Execution slices 1–5 cover App files, upgrade, hooks, teardown, embedded cache and purge; Shell
command-scoped lifecycle; managed Sys built-in resources; and all three manifest gates. Existing
aggregate report types remain CLI compatibility adapters: reusable facts come from structured
results, while restart hints, Shell presentation totals, and Sys field-difference prose remain
frontend metadata. Renderer separation remains a characterization-backed follow-up.

## Consequences

- Phase 1 gains a stable seam that Phase 2 can move behind `shine-core` without treating stdout as
  an API.
- Canonical targets and stable codes can be tested without copying terminal formatting.
- Safe reusable results intentionally carry less diagnostic detail than current human errors; a
  future structured diagnostic message design must define safe fields rather than serializing raw
  errors.
- Runtime state migrations become explicit and reject future incompatible formats before writes.
- Initial adapters may still print during execution; completion of Phase 1 requires renderers to
  consume results after behavior is characterized.
- This decision adds no public command or preset syntax, so it does not publish an unreleased schema
  in the user manual.
