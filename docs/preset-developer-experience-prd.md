# Preset Developer Experience PRD

> **Status:** Roadmap Phase 5 planning. This document is internal and does not define released CLI
> behavior. Each public command becomes part of the bilingual manual only when its implementation
> lands.

## Summary

Phase 5 turns the existing `shine preset new` and `shine preset validate` foundation into a complete,
side-effect-free authoring workflow:

```text
scaffold → validate → lint → plan → fixture test → pack → CI
```

The installed Shine binary and `shine-core` remain the only schema and lifecycle authority. Authoring
commands must reuse the runtime parsers, permission model, pure planners, and in-memory host instead
of introducing a second Preset implementation.

## Goals

1. Let an author scaffold, validate, lint, plan, and test a Preset without linking, activating, or
   installing it.
2. Give CI and AI repair loops deterministic, versioned machine-readable reports.
3. Model platform and host-state variations with declared fixtures rather than access to the real
   HOME, process table, network, package manager, or administrator APIs.
4. Produce a reproducible bundle only after validation, lint policy, executable provenance, and the
   complete input file set are known.
5. Keep public English and Simplified Chinese documentation semantically aligned as each command is
   released.

## Non-goals

- Activating a source or overlay as part of authoring.
- Treating a hypothetical authoring preview as an approval for real mutation.
- Executing generators, hooks, artifacts, bootstrap scripts, installed Shell command bodies, or
  arbitrary fixture scripts during validate, lint, plan, or fixture setup.
- Copying the complete Preset schema into an Agent Skill, JSON Schema file, or CI action that can
  drift from the installed binary.
- Signing, publishing, registry upload, or remote dependency resolution in Phase 5.
- Environment setup and project bootstrap; those are separate product work.

## Command boundaries

### `preset validate`

`validate` remains the sole authority for schema and semantic correctness. It checks every supported
platform branch, referenced files, permission declarations, transform metadata, and executable
dependency policy without loading runtime configuration or executing Preset code. Existing report
schema v1 and diagnostic codes remain compatible.

### `preset lint`

`lint` reports author-quality, portability, minimization, and packaging-policy findings that do not
change whether the runtime parser accepts a Preset. It consumes the same parsed snapshot as
`validate`; it must not reimplement TOML or annotation parsing. CI may opt into treating warnings as
failures, but validation errors always fail.

The initial lint policy should cover legacy metadata, missing human-facing descriptions, overly
broad declared permissions, private machine paths, ignored material such as `node_modules`, and
executable files whose provenance or declaration would prevent packing. Secret-like content
detection must use conservative stable diagnostics and may not print the suspected value.

### `preset plan`

`plan` builds a hypothetical first-install review against a deterministic empty in-memory host. It
reuses the Core security planners but emits a distinct authoring report containing only semantic
steps, resolved permissions, stable blockers, platform, target identity, and explicit assumptions.
It does not emit a `PlanApprovalV1`, an apply token, or a reusable Plan fingerprint.

The first slice accepts exactly one category directory or its `shine.toml`, plus an explicit
`--platform macos|linux|windows`. App and Shell categories produce an install preview. Sys categories
produce a managed-resource install preview for managed items and a bootstrap preview for init items.
Synthetic environment values, trust grants, detected commands, and installed receipts are absent;
the report must say so. Later fixture support supplies those observations explicitly.

### `preset test`

`test` reads declarative fixture cases and runs validation plus authoring planning against
`InMemoryHost`. A fixture may describe platform, non-secret environment presence, opaque secret
versions, files, receipts, command/path detection, trust grants, and expected diagnostic/action/
permission codes. It may not contain executable setup or teardown code.

Fixture reports use stable case identities and deterministic ordering. The first schema should
prefer exact structured assertions over snapshots of terminal prose.

### `preset pack`

`pack` is last in the dependency order. It accepts only a validation-clean, policy-clean Preset
snapshot and produces deterministic bytes from logical paths, file modes, content hashes, and a
versioned bundle manifest. Timestamps, checkout location, filesystem enumeration order, and host
metadata must not affect output.

Packing rejects plaintext secrets, private absolute paths, `node_modules`, symlinks that escape the
category, unlisted executable code, unsupported file kinds, and inputs not represented in the
logical snapshot. Phase 5 records provenance and content digests but does not add signing or a
registry protocol.

## Versioned reports and schema reference

Each command owns a versioned top-level report. Shared diagnostic, semantic-step, and permission
types come from Core. A command may add fields compatibly within its current schema, while changing
field meaning, severity semantics, or exit mapping requires a new report version.

The schema reference is generated from the shipped Core/CLI types and command help. A handwritten
schema that can diverge from runtime parsing is not authoritative. Examples must be executable test
fixtures or generated from tested sources.

## Safety and determinism

- Authoring commands route before `Config::load_or_init()` and background update checks.
- Validation, lint, and planning receive only observation-capable hosts.
- Planning uses an immutable source snapshot and one captured synthetic state per report.
- Reports contain logical target/resource identities and stable codes. Private checkout roots,
  content, argv, environment values, secret plaintext, and raw process output are excluded.
- The same source snapshot, fixture, platform, and Shine version produce byte-identical JSON after
  normal serialization.
- `--format json` writes one JSON document to stdout; diagnostics do not add non-JSON stdout.

## Delivery sequence

### Slice 5A — Authoring contract and Plan report (implemented)

- Accept the command boundaries and the separate authoring-report ADR.
- Add `shine preset plan <CATEGORY> --platform <PLATFORM> --format text|json`.
- Reuse one immutable validation snapshot and Core planners over `InMemoryHost`.
- Cover App, Shell, managed Sys, and Sys bootstrap categories without execution or real-host state.
- Publish bilingual command documentation and update the authoring skill.

### Slice 5B — Lint and CI policy (initial rules implemented)

- Add Core-owned lint rules and a versioned report.
- Define warning/error and optional strict-CI exit behavior.
- Run built-in Presets and checked-in examples through validate and lint in repository CI.

### Slice 5C — Declarative fixtures and `preset test` (empty-host assertions implemented)

- Add fixture schema v1 and safe host-state materialization.
- Support structured assertions over validation, steps, permissions, and blockers.
- Add examples for all three Preset kinds and a reusable CI workflow example.

### Slice 5D — Reproducible pack (bundle v1 implemented)

- Add bundle manifest v1, deterministic archive generation, and policy gates.
- Verify reproducibility across different checkout paths and enumeration orders.
- Reject secrets, private paths, ignored dependency trees, escaping links, and undeclared code.

### Slice 5E — Reference and workflow completion (in progress)

- Generate schema/report references from shipped types.
- Keep examples executable and documentation bilingual.
- Update the Agent Skill to prefer validate, lint, plan, and fixture test before isolated runtime
  dry-runs.

## Exit criteria

Phase 5 is complete when all Roadmap criteria are satisfied and every authoring command is proven
read-only against the real host boundary, machine-readable reports are versioned and deterministic,
the bundle is reproducible and policy-gated, examples run in CI, and both public manual locales
describe identical released behavior.
