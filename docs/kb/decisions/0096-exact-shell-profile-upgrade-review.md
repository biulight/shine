# 0096 — Plan Shell profile upgrade effects from observed content

- **Status**: Accepted
- **Date**: 2026-10-04
- **Evidence**: `core/src/runtime/{planner.rs,shell.rs}`, `cli/src/lifecycle_plan.rs`
- **Extends**: ADR 0094, ADR 0095

## Context

Shell planning marked `shell/profile` as updated and requested remove/write access to every Shell
startup file whenever any Shell Preset changed. Execution already compared the generated profile
and startup sentinel with observed files and skipped identical files. The review therefore made a
content-only snapshot update look like an additional Preset and an unnecessary `.zshrc` write.

## Decision

Upgrade planning reuses execution's read-only profile-file comparison. It projects source-command
receipts in upgrade order, compares each projected profile with observed managed and startup files,
and requests profile permissions only for paths that can change. The full Shell manifest and the
observed profile paths remain fingerprint-bound; apply still regenerates the Plan before mutation.
Other Shell operations retain their existing conservative profile review.

The default compact upgrade renderer moves actual `shell/profile` steps into a separate
`Shell integration (internal)` section and labels each changed logical path. This identity is an
internal operation, not a Preset category. Detailed review retains the full Plan identity.

## Consequences

An unchanged `.zshrc` contributes no upgrade step or permission. Drift in the managed profile and
drift in a startup file are assessed independently. Snapshot, launcher, manifest, journal, and
recovery effects still retain their own permissions.
