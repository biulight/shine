# Shine Structured Lifecycle Contract PRD

> **Status:** Roadmap Phase 1 structured lifecycle contract and acceptance gate complete. This
> document records the executable Phase 1 contract from [ROADMAP.md](ROADMAP.md); it is not released
> JSON behavior or a public preset-authoring contract.

## 1. Summary

Shine already gives App, Shell, and managed Sys resources safe install, update, upgrade, and
uninstall behavior, but each domain exposes different Rust return types and mixes execution with
terminal rendering. Phase 1 introduces one versioned lifecycle result envelope, explicit runtime
manifest compatibility, and an incremental migration path that preserves each domain's ownership
rules.

The completed execution slices cover App install/update/upgrade/uninstall plus hooks, implicit teardown,
embedded preset cache, and purge; Shell install/update/upgrade/uninstall; and managed Sys
apply/update/upgrade/uninstall. `app-manifest.toml`, `shell-manifest.toml`, and `sys-manifest.toml`
are independently versioned. CLI-private presentation events and interaction adapters now preserve
the existing output, permission prompts, and exit semantics without making terminal rendering part
of lifecycle execution. App read-only update rows and structured outcomes share one typed
assessment pass, and the acceptance suite pins complete App, Shell snapshot, and managed Sys
lifecycle chains plus repository-wide built-in Preset validation.

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

- No public JSON lifecycle command in Phase 1.
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
- one status: `changed`, `unchanged`, `pending`, `previewed`, `skipped`, `preserved`, `conflict`, or
  `failed`;
- zero or more structured effects;
- zero or more stable diagnostic codes.

Effects describe ownership-relevant facts. Contract v1 includes resource and receipt writes,
removals and previews; cache writes, removals, purge and previews; code execution and its preview;
backup creation/restoration; managed-key removal; and managed/user preservation or explicit
override. Exact spellings are pinned by `core/src/lifecycle.rs` serialization tests.

Read-only `update` results use `dry_run = false`: an applicable change is `pending`, while an
ownership conflict remains `conflict`. Explicit dry-run outcomes use `dry_run = true` and
`previewed`; they are not a Roadmap Phase 3 Plan. `changed` means this execution actually changed
Shine-owned state. Existing dry-run paths remain conservative and are not input-snapshot-bound or
approval-capable.

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
- Bootstrap Sys outcomes may later reuse `sys/<item>`, but Phase 1 does not expand bootstrap
  software ownership or uninstall semantics.
- Only shared, global App cache/manifest purge uses the domain-root target `app`.

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

This policy applies independently to `app-manifest.toml`, `shell-manifest.toml`, and
`sys-manifest.toml`, each currently at schema version 1. The per-resource Sys receipt `version`
remains a separate compatibility contract.

## 8. Phase 1 execution slices

### 8.1 Implementation

1. Add the contract types to `shine-core` without Clap, terminal, Config, process, or filesystem
   dependencies.
2. Add internal App, Shell, and managed Sys lifecycle entry points that return
   `LifecycleResultV1`.
3. Preserve the existing public handlers and terminal rendering while they delegate to the result
   producing path through CLI-private reporter and interaction ports.
4. Map handled App files/helpers, Shell commands, and managed Sys items to their canonical targets,
   logical resources, effects, and stable diagnostic codes.
5. Add `schema_version = 1` to all three runtime manifests, normalize legacy version 0 on load, and
   reject unknown versions before mutation.

The App adapter now also covers upgrade branches, preset-cache extraction/removal, `post_install`
and `post_upgrade`, best-effort uninstall teardown, and category/global purge. Hook and teardown
failures remain non-fatal; only the existing fatal generator class changes the aggregate upgrade
exit behavior. Read-only App update derives its rows and `pending`/`unchanged`/`conflict` outcomes
from the same per-file assessment, so an automatic generator is never evaluated twice merely to
produce the reusable result.

The Shell adapter emits one outcome per selected or installed command. Read-only update derives
`pending` from the existing typed `ShellRow`/`UpdateChange` assessment, while foreign launcher
ownership remains `conflict`. Upgrade assesses the same targets before and after execution and
records snapshot, rendered-template, launcher, profile, cache, and receipt work as effects rather
than extra target counts. Shared category state is removed only after the last command receipt.

The managed Sys adapter covers built-in managed-resource drivers only. Driver execution returns
typed resource effects and a typed user-modification conflict instead of requiring the adapter to
parse display details or errors. Receipt comparison produces both the safe existing CLI field
differences and a `pending` lifecycle outcome. Bootstrap, profile enable/disable, and composed
profile synchronization are intentionally outside this slice.

### 8.2 Outcome mapping

| Existing App outcome | Structured status | Required effects |
| --- | --- | --- |
| Installed | changed | resource-written, receipt-written |
| BackedUpAndInstalled | changed | backup-created, resource-written, receipt-written |
| AlreadyManaged | unchanged | none |
| DryRun | previewed | resource-write-previewed, receipt-write-previewed |
| Generator unavailable with last-known-good install | preserved | managed-resource-preserved |
| Install/materialization error handled per file | failed | stable diagnostic code |
| Removed Copy resource | changed | resource-removed, receipt-removed |
| Removed JSON managed keys | changed | managed-keys-removed, receipt-removed |
| Restored backup | changed | backup-restored, receipt-removed |
| Missing destination with stale receipt | changed | receipt-removed |
| User-modified destination kept | preserved | user-resource-preserved |
| Forced removal of modified state | changed | user-modification-overridden plus removal effects |
| Uninstall DryRun | previewed | resource-remove-previewed, receipt-remove-previewed |

The existing terminal summary may continue to call a missing destination “skipped” for compatibility
during this slice; the structured result records the receipt mutation accurately.

## 9. Acceptance matrix

The Roadmap Phase 1 acceptance gate is complete when:

- App install produces structured changed, unchanged, previewed, preserved, and failed outcomes for
  the existing handled branches.
- App uninstall produces structured removal, backup restoration, managed-key removal, stale-receipt
  cleanup, user-preservation, force, and preview outcomes.
- install → no-op install → uninstall round-trip tests assert both filesystem/manifest state and
  structured results.
- targeted App uninstall/upgrade characterization proves other categories remain unchanged.
- Shell embedded, external snapshot, and external live tests cover install → pending update →
  upgrade → uninstall, sibling preservation, foreign launcher conflict, and last-reference cache
  cleanup.
- managed Sys fake-OS tests cover resource/receipt round trips, receipt-only uninstall, typed
  pending differences, missing env, user preservation, and driver failure mapping without touching
  real system resources.
- unversioned App, Shell, and Sys manifests load without read-only rewrites and are written as
  version 1 by the next successful mutation.
- unsupported future App, Shell, and Sys manifest versions fail before domain mutation.
- a serialization golden test pins Contract v1 field names and enum spellings.
- result serialization contains no absolute destination, raw error, content, environment, or secret
  value.
- applicable App, Shell, Sys, formatting, clippy, and `git diff --check` checks pass.
- representative CLI output remains unchanged; no public manual change is required because no
  command, flag, output promise, or authoring contract is added.

## 10. Slice status

1. **Complete:** App upgrade structured outcomes.
2. **Complete:** App hooks, teardown, preset-cache, and purge effects.
3. **Complete:** Shell command-scoped install/update/upgrade/uninstall outcomes.
4. **Complete:** managed Sys item outcomes and typed receipt differences.
5. **Complete:** Shell and Sys manifest schema v1 plus legacy/future gates.
6. **Complete:** App, Shell, and managed Sys execution emit CLI-private presentation events through
   a writer-backed renderer; upgrade sections and interactive confirmation/authorization remain
   frontend concerns, with characterization tests pinning stream and separator behavior.
7. **Complete:** repository-wide built-in Preset validation and App, external snapshot Shell, and
   fake-OS managed Sys install → update → upgrade → uninstall acceptance chains close the broader
   Roadmap Phase 1 gate, including targeted ownership isolation and safe result serialization.
8. **In progress / Phase 2:** reusable executors and host abstractions are moving behind
   `shine-core` without changing Contract v1; the implementation contract is
   [shine-core-runtime-prd.md](shine-core-runtime-prd.md).

## 11. Documentation impact

This PRD and its ADR are internal sources. These Phase 1 slices add no released command, flag,
public JSON schema, preset field, or user workflow, so the English and Simplified Chinese manuals
remain unchanged. Public documentation becomes mandatory when a machine-readable lifecycle surface
or user-visible compatibility error is released.
