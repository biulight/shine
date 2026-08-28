# 0037 — Lifecycle operations produce versioned structured results

- **Status**: Accepted
- **Date**: 2026-08-28
- **Evidence**: `utils/src/lifecycle.rs`, `cli/src/apps/{install,uninstall}.rs`,
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

Dry-run is represented as `previewed`; it is not called a Plan and carries no approval or snapshot
guarantee. The reviewable Plan contract remains a separate Roadmap Phase 3 decision.

Runtime manifests adopt a top-level `schema_version` independently. A missing version is legacy v0,
supported legacy state normalizes in memory and upgrades on the next successful write, and an
unsupported future version fails before mutation. An incompatible shape or semantic
reinterpretation requires a version bump.

The first slice covers App file/receipt outcomes from install/uninstall and `app-manifest.toml`. It
preserves public CLI output and exit behavior. App hooks/teardown/purge, Shell, managed Sys, renderer
separation, and their manifest versions follow as separate characterization-backed slices.

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
