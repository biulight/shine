# ADR 0119: Explicit CLI preset asset readers

- **Date**: 2026-10-17
- **Status**: Accepted

## Context

Config loading wrote the active overlay directory into a process-global mutex. Asset enumeration,
reads and export consumed it implicitly, so their source depended on the last configuration loaded.
Tests serialized overlay access with a separate lock. Core runtime assembly already captured preset
sources explicitly through the host; it did not need this global state.

## Decision

- Replace the mutable global with a borrowed `PresetAssetReader`, constructed from the consuming
  Config's active overlay root. Use the same reader for enumeration, reads and extraction.
- Keep pristine embedded APIs independent of configuration for `preset copy`, Core bootstrap and
  completion's baseline candidates. Completion separately unions its read-only discovered overlay.
- Preserve sorted/deduplicated path union, overlay-first reads with embedded fallback, node_modules
  exclusion and existing source path checks. Full-external env catalogs remain authoritative.
  Export continues to mean embedded bundle plus overlay, including in full-external mode.
- Keep the shared extraction implementation and its output/link/atomic publication checks intact.
  Do not change managed Git scope, synchronization, ownership, trust or snapshot-bound approval.

## Consequences

Loading or replacing one Config no longer selects asset sources for another consumer. Readers are
live filesystem views, not immutable snapshots or execution authority; lifecycle paths still use
Core's captured snapshot. No command, persisted configuration, Plan, receipt or journal schema changes.

Regressions cover independent readers and concurrent extraction, per-Config env metadata including
managed/manual precedence, concurrent command-level export with collision policy, pristine copy,
and completion after unrelated config loading. Existing extraction and runtime tests continue to
cover destination links, hard links, concurrent publication and lifecycle safety boundaries.

The CLI assembly regression in `cli/src/core_runtime.rs` also checks two independent overlays, live
reader updates versus frozen Core bytes/env, fresh capture invalidating earlier approval, and no
state or destination initialization during capture/planning.
