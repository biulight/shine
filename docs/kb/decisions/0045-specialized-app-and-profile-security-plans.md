# 0045 — App refresh, artifact, and Sys profile mutations use specialized security Plans

- **Status**: Accepted
- **Date**: 2026-08-30
- **Evidence**: `core/src/plan.rs`, `core/src/runtime/planner.rs`,
  `core/src/runtime/app.rs`, `core/src/runtime/app_metadata.rs`,
  `core/src/runtime/validation.rs`, `core/src/runtime/sys_profile/mod.rs`,
  `cli/src/lifecycle_plan.rs`, `presets/app/`

## Context

ADRs 0042–0044 placed App, Shell, managed Sys lifecycle, and Sys bootstrap mutation behind pure,
snapshot-bound security Plans. Three mutation surfaces remained outside that boundary: explicit App
generator refresh, App artifact apply/remove, and explicit Sys profile enable/disable. They can run
Preset code, expose environment inputs, create runtime directories, update manifests, and rewrite
managed shell profile content.

Artifact execution also inherited the complete active `[env]` table. A target-local Plan could not
derive a complete environment permission set from that behavior without treating every configured
variable as an implicit capability.

## Decision

Plan schema v1 gains additive operation identities: `app-refresh`, `app-artifact-apply`,
`app-artifact-remove`, `sys-profile-enable`, and `sys-profile-disable`. These operations do not
pretend to be lifecycle install/update/uninstall and retain their existing domain reports.

Each operation has an observation-only planner and an approved Core entry point. The approved entry
point regenerates the Plan and validates its exact fingerprint and permission set before invoking
the internal executor. CLI mutation renders the Plan, asks with a default answer of No, and accepts
`--yes` only as a prompt bypass. Sys profile dry-run remains a separate preview and conflicts with
`--yes`.

App refresh binds category, selected generated file, force mode, App manifest ownership, live
destination state, generator input identities, executable source, potential output mutation, and
applicable post-upgrade hooks. For an embedded generator it also binds and declares the runtime
script materialization under the Shine directory. It never runs a generator or hook while planning.

Artifact planning binds apply/remove identity, the effective script and trust layer, declared
environment identities, runtime command, executable source, Preset cache, and runtime directories.
Artifact processes receive only fixed `SHINE_APP_*` contract variables plus their explicit
`[artifact].env` mappings; each source also requires a category `[permissions].environment`
declaration. Generator processes likewise receive only their explicit `generator.env` mappings plus
fixed contract variables. Plain values contribute only hashes to Plan state; secret values require
opaque versions when present and never enter Plan serialization. Missing optional artifact inputs
are bound as absent and omitted from the child environment.

App uninstall now plans declared artifact teardown when its executable source is available. When
external code remains blocked by `allow_app_hooks`, uninstall records a safe skipped teardown and
continues ownership-safe file removal rather than inventing or executing code.

Sys profile planning binds the item and desired enabled state, live detection for enable, run
manifest, the complete desired enabled-item permission declarations, generated profile files, shell
configuration state, and external-code compatibility gate. Planning never runs detection commands
or profile code.

## Consequences

- Every currently supported App, Shell, managed Sys, Sys bootstrap, App refresh/artifact, and Sys
  profile mutation has a snapshot-bound Plan operation and fresh approval validation.
- Artifact presets that depended on undeclared ambient `[env]` variables must list those names in
  `[permissions].environment`; this is an intentional fail-closed compatibility change.
- Permission declarations still do not replace `allow_app_hooks` or `allow_sys_code`. Migrating
  those coarse grants to scoped trust remains a separate Phase 3 slice.
- Auto-generator behavior in read-oriented status/update remains governed by its existing contract
  until a separate compatibility ADR changes it.
