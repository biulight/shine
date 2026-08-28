# Shine Core Runtime Extraction PRD

> **Status:** Roadmap Phase 2 implementation contract.
> This document is internal and does not define released CLI or JSON behavior.

## Summary

Roadmap Phase 1 established a frontend-neutral `LifecycleResultV1`, versioned runtime manifests,
and replaceable presentation and interaction seams. Phase 2 moves the reusable product and domain
logic behind `shine-core` so the CLI becomes a frontend instead of a second lifecycle runtime.

The migration preserves commands, exit behavior, terminal output, runtime state formats, ownership
rules, and Lifecycle Contract v1. The existing `utils/` and root package layout remains in place;
dependency inversion is the gate, not a directory reshuffle.

## Goals

1. Make `shine-cli -> shine-core` the only product dependency direction.
2. Execute App, Shell, and Sys lifecycle operations through Core APIs.
3. Keep configuration, preset parsing, validation, inspection, manifests, and OS-effect decisions
   reusable without Clap, dialoguer, terminal formatting, Tauri, or `rust-embed`.
4. Provide real and in-memory hosts. The in-memory host must test complete lifecycle chains without
   reading the real HOME, environment, process table, network, or administrator state.
5. Preserve Lifecycle Contract v1 and all existing CLI behavior byte-for-byte.

## Non-goals

- No Phase 3 permission model, snapshot-bound approval, or reviewable Plan.
- No Phase 4 action IR, journal, rollback, or crash recovery.
- No public lifecycle JSON command and no stable third-party Rust API commitment.
- No `crates/` layout migration and no public-manual changes.

## Runtime boundary

`shine-core` owns an internal `CoreRuntime` and domain request types. Requests carry the existing
target, dry-run, and force inputs; mutations continue to return `LifecycleResultV1`. Read-only
assessment is not called a security Plan and has no approval or snapshot guarantee.

Core receives all external capabilities through ports:

- a host for filesystem, links, processes, privileged writes, and platform resources;
- an immutable preset snapshot/provider, with the `rust-embed` implementation remaining in the
  distribution frontend;
- typed interaction requests for confirmation and administrator authorization;
- typed, non-serializable observer events for progressive frontend presentation.

Events may carry private paths and human diagnostics needed by the CLI, but they never enter
`LifecycleResultV1`. The CLI maps them to its existing stdout/stderr text and prompt order.

## Migration slices

1. Move shared persistence, transforms, manifest compatibility, configuration, preset discovery,
   validation, canonical identities, and inspection models into Core.
2. Add a Core-only harness for validate, inspect, and the existing assessment/preview seam.
3. Move App install/update/upgrade/uninstall, generators, hooks, artifacts, cache, and manifest
   ownership behind Core.
4. Move Shell metadata, snapshot/live deployment, transforms, launchers, profile integration, and
   manifest ownership behind Core.
5. Move managed Sys, bootstrap, selection, profile composition, receipts, and platform drivers
   behind Core. Bootstrap/profile keep domain reports rather than extending Contract v1.
6. Remove superseded CLI implementations, enforce dependency boundaries, and close the Phase 2
   acceptance matrix.

Every slice must compile and retain the previous CLI characterization suite.

## Acceptance

- A Rust harness depending only on `shine-core` can validate presets, inspect installed state, and
  assess an operation against an immutable input snapshot.
- In-memory App, Shell, and Sys tests cover lifecycle round trips, targeted isolation, conflicts,
  backup/receipt behavior, and denied authorization without real host access.
- The CLI does not write App, Shell, or Sys manifests or corresponding resources directly.
- `shine-core` has no Clap, dialoguer, console, Tauri, or `rust-embed` dependency.
- Existing CLI output, prompts, exit codes, manifest formats, and Contract v1 serialization remain
  unchanged on macOS, Linux, and Windows.

## Documentation impact

This is an internal architecture refactor. Update this PRD, ADR 0038, and the architecture KB as
the migration lands. Do not update the public manuals unless a separately approved user-visible
change is introduced.
