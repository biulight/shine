# 0039 — Align the Core package directory and dependency identifier

- **Status**: Accepted
- **Date**: 2026-08-29
- **Evidence**: `core/`, `Cargo.toml`, `docs/kb/architecture/module-map.md`

## Context

The reusable package was originally stored under `utils/` and imported as `utils` while its
published package name was already `shine-core`. After the Core runtime migration, it owns the
App, Shell, and Sys lifecycle runtime rather than a miscellaneous helper collection. The old name
also overlaps with the user-visible `shell/utils` preset category.

## Decision

Store the `shine-core` package under `core/` and depend on it as `shine-core`, producing the Rust
crate identifier `shine_core`. Keep the existing package name, versions, APIs, lifecycle contracts,
and user-visible `shell/utils` preset identifiers unchanged.

## Consequences

- Filesystem layout, Cargo metadata, Rust imports, and architecture terminology describe the same
  Core boundary.
- The rename has no CLI, preset, runtime-state, or published package compatibility impact.
- Rust code uses `shine_core`, not `core`, avoiding ambiguity with Rust's built-in `core` crate.
