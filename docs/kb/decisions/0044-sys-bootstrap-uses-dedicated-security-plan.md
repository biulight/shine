# 0044 — Sys bootstrap uses a dedicated snapshot-bound security Plan

- **Status**: Accepted
- **Date**: 2026-08-30
- **Evidence**: `core/src/plan.rs`, `core/src/runtime/planner.rs`,
  `core/src/runtime/sys_bootstrap.rs`, `cli/src/sys/commands.rs`

## Context

ADR 0040 reserved specialized operations instead of encoding them as lifecycle install or upgrade.
ADR 0043 therefore excluded Sys bootstrap from approval enforcement even though it can run package
providers or Preset scripts, request administrator access, materialize executable Preset bytes,
write the Sys run manifest, and reconcile shell profiles. Roadmap Phase 3 requires every mutation
operation to derive permissions and bind approval to the exact source and state later applied.

Bootstrap also supports interactive, profile, and explicit item selection. Selection cannot remain
an executor-time choice after review: it changes targets, scripts, permissions, profile content,
and receipts.

## Decision

Plan schema v1 gains the additive `sys-bootstrap` operation identity through `PlanOperationV1`.
Existing lifecycle spellings remain unchanged. Bootstrap does not pretend to produce a
`LifecycleResultV1`; its existing domain report remains authoritative after execution.

Interactive, named-profile, positional, and repeated `--item` selection resolve before planning to
one ordered, duplicate-free item list. `SysBootstrapPlanRequest` carries only that exact list,
the OS and shell identities, `force_profile`, and opaque input versions. The planner accepts only
`FileSystemObservationHost`. It cannot run detection commands, installers, profile code, writes,
or administrator authorization.

The planner observes path and command-presence metadata without running `--version`, binds the Sys
run manifest, effective PATH identity, required environment identities, proxy configuration, and
profile resources, and combines typed provider/interpreter/profile effects with each item's
permission declaration. Missing declarations or environment inputs fail closed. Executable
external/overlay scripts and profile content remain additionally blocked by `allow_sys_code` during
the compatibility period.

The CLI renders the final Plan and uses the same default-No/non-TTY policy as protected lifecycle
commands. `--yes` skips only confirmation and conflicts with `--dry-run`. Approved execution
regenerates the Plan from fresh inputs and validates the exact fingerprint and permission set
before detection, materialization, package/script execution, profile mutation, or receipt writes.

`sys bootstrap --item ITEM` is an explicit repeatable spelling for orchestrators. Existing ordered
positional items remain supported; positional items, repeated `--item`, and `--preset` are mutually
exclusive.

## Consequences

- A source, selected item, detection presence, environment/proxy input, run-manifest, or profile
  state change invalidates approval before bootstrap mutation.
- Detection can still run its configured `--version` command during approved execution, but never
  during planning.
- Package providers and interpreter invocation are Core-bounded typed permissions; opaque script
  capabilities continue to come from target-local declarations.
- App artifact/refresh and explicit Sys profile operations still require their own operation
  contracts before Roadmap Phase 3 is complete.
- The coarse `allow_sys_code` gate remains until the scoped trust migration covers every Sys code
  execution surface.
