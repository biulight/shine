# Shine Structured Lifecycle Contract PRD

> **Status:** Phase 1A in progress. This document scopes the first executable slice of
> [ROADMAP.md](ROADMAP.md); it is not released CLI behavior or a public preset-authoring contract.

## 1. Summary

Shine already gives App, Shell, and managed Sys resources safe install, update, upgrade, and
uninstall behavior, but each domain exposes different Rust return types and mixes execution with
terminal rendering. Phase 1A introduces one versioned lifecycle result envelope, explicit runtime
manifest compatibility, and an incremental migration path that preserves each domain's ownership
rules.

The first vertical slice covers App install and uninstall. It records structured results alongside
the existing renderer, versions `app-manifest.toml`, and leaves command output and mutation semantics
unchanged. Later slices adapt App upgrade, Shell, and managed Sys before terminal rendering is fully
separated from execution.

## 2. Problem

The current domains expose incompatible seams:

- App install and uninstall return `Result<()>` and render per-file outcomes directly.
- Shell upgrade returns `ShellUpgradeReport`, while install and uninstall return `Result<()>`.
- Managed Sys uses `SysItemOutcome`, `SysUpdateRow`, and `SysUpgradeReport` with different status
  vocabularies.
- `preset validate` has a versioned machine-readable report, but App, Shell, and Sys runtime
  manifests do not share an explicit version policy.

This prevents Phase 2 from moving reusable product logic behind `shine-core` without either copying
frontend behavior or treating terminal output as an API.

## 3. Goals

1. Define one versioned, serializable lifecycle result envelope owned by `shine-core`.
2. Preserve App, Shell, and Sys domain models rather than flattening their execution logic.
3. Use canonical lifecycle target identities in every outcome.
4. Represent mutation, no-op, preview, preservation, conflict, and failure without parsing prose.
5. Version runtime manifests and define legacy and future-version behavior before their shapes
   evolve.
6. Keep secret plaintext, content bytes, environment values, private destination paths, and raw
   errors out of reusable lifecycle results.
7. Preserve all existing CLI output and exit behavior during the incremental migration.

## 4. Non-goals

- No public JSON lifecycle command in Phase 1A.
- No reviewable Plan, approval, permission derivation, journal, rollback, or recovery; those belong
  to Roadmap Phases 3 and 4.
- No directory-layout migration to `crates/shine-core` or `crates/shine-cli`.
- No generic action IR and no attempt to make App, Shell, and Sys use one domain model.
- No change to backup, managed-key, generator, hook, artifact, privilege, or uninstall policy.
- No UI, registry, or AI execution path.

## 5. Contract v1

### 5.1 Envelope

`LifecycleResultV1` contains:

- `schema_version = 1`;
- one `operation`: `install`, `update`, `upgrade`, or `uninstall`;
- whether the run used the existing dry-run/preview path;
- ordered per-resource outcomes.

The result derives aggregate counts from its outcomes instead of persisting a second summary that
could drift.

Errors that prevent safe target selection or state loading remain an outer Rust `Result::Err` in
v1. Once execution begins, domain handlers should record independently handled resource failures as
structured outcomes instead of parsing their rendered error text.

### 5.2 Outcome

Each `LifecycleOutcomeV1` contains:

- a canonical lifecycle target such as `app/ghostty`, `shell/utils/shine-theme-sync`, or
  `sys/split-dns`;
- an optional logical resource name relative to that target;
- one status: `changed`, `unchanged`, `previewed`, `skipped`, `preserved`, `conflict`, or `failed`;
- zero or more structured effects;
- zero or more stable diagnostic codes.

Effects describe ownership-relevant facts such as a resource or receipt being written or removed,
a backup being created or restored, managed keys being removed, or a user modification being
preserved or explicitly overridden.

Dry-run outcomes are `previewed`, not a Roadmap Phase 3 Plan. Existing dry-run paths may be
conservative and are not input-snapshot-bound or approval-capable.

### 5.3 Safe diagnostics

Reusable results contain diagnostic codes, not arbitrary error messages. They must not contain:

- source or destination file content;
- environment or secret values, ciphertext, subscription URLs, or credentials;
- raw child stdout/stderr;
- absolute destination paths or other machine-private path details.

The CLI may keep rendering existing human diagnostics during Phase 1, subject to the current
domain-specific redaction rules. A future structured diagnostic message contract requires its own
safe-field design.

## 6. Canonical identity

- App lifecycle ownership is category-scoped: `app/<category>`. An App file is the outcome's
  logical resource.
- Shell command activation is command-scoped: `shell/<category>/<command>`. Category operations
  produce one outcome per installed command where practical.
- Managed Sys ownership is item-scoped: `sys/<item>`.
- Bootstrap Sys outcomes may later reuse `sys/<item>`, but Phase 1A does not expand bootstrap
  software ownership or uninstall semantics.

Canonical identity controls reporting and selection only. Filesystem ownership continues to come
from the domain manifest, receipt, managed marker, and current safety invariants.

## 7. Runtime manifest compatibility

Each runtime manifest migrates independently to a top-level `schema_version`:

1. A missing field is legacy version 0 and remains readable.
2. Loading a supported legacy shape normalizes it in memory to the current version.
3. The next successful manifest save from a mutation command writes the current version; read-only
   commands do not rewrite state merely to add a version marker.
4. A future unsupported version fails before mutation with a manifest-specific compatibility
   error.
5. Versioning does not weaken entry-level defaults needed for backward compatibility.
6. A version bump is required for an incompatible shape or semantic reinterpretation. Additive
   optional fields may remain within a version when older readers can safely ignore them.

Phase 1A applies this policy only to `app-manifest.toml`; Shell and Sys follow in their own slices.

## 8. Phase 1A — App vertical slice

### 8.1 Implementation

1. Add the contract types to `shine-core` without Clap, terminal, Config, process, or filesystem
   dependencies.
2. Add internal App install/uninstall entry points that return `LifecycleResultV1`.
3. Preserve the existing public handlers and terminal rendering while they delegate to the result
   producing path.
4. Map every handled App file outcome to a canonical category target and logical source resource.
5. Add `schema_version = 1` to `AppManifest`, normalize legacy version 0 on load, and reject unknown
   versions.

This first adapter covers App file and receipt outcomes. Preset-cache extraction/removal,
post-install hooks, best-effort artifact teardown, and `--purge` cleanup keep their existing
terminal-only reporting until a later App slice defines structured effects for those operations.
Consequently Phase 1A establishes the seam but does not by itself complete the Roadmap Phase 1
structured-result gate.

### 8.2 Outcome mapping

| Existing App outcome | Structured status | Required effects |
| --- | --- | --- |
| Installed | changed | resource-written, receipt-written |
| BackedUpAndInstalled | changed | backup-created, resource-written, receipt-written |
| AlreadyManaged | unchanged | none |
| DryRun | previewed | resource-write-previewed |
| Generator unavailable with last-known-good install | preserved | managed-resource-preserved |
| Install/materialization error handled per file | failed | stable diagnostic code |
| Removed Copy resource | changed | resource-removed, receipt-removed |
| Removed JSON managed keys | changed | managed-keys-removed, receipt-removed |
| Restored backup | changed | backup-restored, receipt-removed |
| Missing destination with stale receipt | changed | receipt-removed |
| User-modified destination kept | preserved | user-resource-preserved |
| Forced removal of modified state | changed | user-modification-overridden plus removal effects |
| Uninstall DryRun | previewed | resource-remove-previewed |

The existing terminal summary may continue to call a missing destination “skipped” for compatibility
during this slice; the structured result records the receipt mutation accurately.

## 9. Acceptance matrix

Phase 1A is complete when:

- App install produces structured changed, unchanged, previewed, preserved, and failed outcomes for
  the existing handled branches.
- App uninstall produces structured removal, backup restoration, managed-key removal, stale-receipt
  cleanup, user-preservation, force, and preview outcomes.
- install → no-op install → uninstall round-trip tests assert both filesystem/manifest state and
  structured results.
- targeted App uninstall/upgrade characterization proves other categories remain unchanged.
- an unversioned App manifest loads and is written as version 1 after a successful mutation.
- an unsupported future App manifest version fails before filesystem mutation.
- a serialization golden test pins Contract v1 field names and enum spellings.
- result serialization contains no absolute destination, raw error, content, environment, or secret
  value.
- existing App tests, formatting, clippy, and `git diff --check` pass.
- representative CLI output remains unchanged; no public manual change is required because no
  command, flag, output promise, or authoring contract is added.

## 10. Follow-up slices

1. App upgrade returns the same result envelope and stops maintaining a separate aggregate-only
   contract.
2. App hook, teardown, preset-cache, and purge effects join the result before renderer separation.
3. Shell install/update/upgrade/uninstall adapts command-scoped receipts and shared deployment
   effects.
4. Managed Sys adapts item outcomes and receipt differences.
5. Runtime manifest versions land for Shell and Sys with legacy fixtures.
6. Terminal renderers consume completed results after characterization tests pin current output.
7. Phase 2 moves reusable executors and host abstractions behind `shine-core` without changing the
   contract.

## 11. Documentation impact

This PRD and its ADR are internal sources. Phase 1A adds no released command, flag, public JSON
schema, preset field, or user workflow, so the English and Simplified Chinese manuals remain
unchanged. Public documentation becomes mandatory when a machine-readable lifecycle surface or
user-visible compatibility error is released.
