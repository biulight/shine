# Frontend Service and Conformance PRD

> **Status:** Roadmap Phase 6 in progress; inventory contract implemented first.
> This document is internal and does not define a released CLI or JSON interface.

## Summary

Phase 6 establishes one frontend-neutral application service above `shine-core` for the CLI,
future local MCP adapter, and `shine-ui`. The service owns stable, redacted contracts for host
inventory, inspection, Plan review, operation state, diagnostics, events, recovery, and lifecycle
results without turning the workspace-internal `CoreRuntime` into a general remote API.

The first slice is deliberately read-only. It introduces Contract v1 inventory and migrates
`shine list` to consume it while preserving existing terminal behavior. Later slices may extend the
service only after their wire shape, approval ownership, and conformance expectations are accepted.

## Goals

1. Make every frontend consume one immutable Preset snapshot, captured runtime configuration, and
   Core lifecycle implementation.
2. Version every serializable frontend contract and keep it free of raw errors, machine-private
   paths, content, argv, environment values, and secret plaintext.
3. Preserve the distinction between a review request, explicit human approval, and execution.
4. Prove adapter equivalence with fixtures and conformance tests rather than stdout parsing.
5. Preserve released CLI behavior while its data collection moves behind the service.

## Non-goals

- No public JSON command, remote API, daemon, MCP server, Tauri application, or registry protocol.
- No mutation, operation progress, recovery execution, or new approval surface in Slice 6A.
- No serialization of existing `RuntimeEvent`, inspection paths, process output, or local errors.
- No third-party stability promise for `CoreRuntime`, host traits, or domain request types.

## Contract boundary

`FrontendService<H>` wraps one fully captured `CoreRuntime<H>`. Distribution frontends continue to
supply embedded bytes and resolved settings through the existing shared snapshot bootstrap. The
service projects domain state into versioned reports; adapters may filter, group, and render those
reports, but may not walk Preset directories, parse manifests, infer ownership, or recreate
lifecycle decisions.

Inventory Contract v1 contains only:

- canonical App, Shell-command, and Sys target identities;
- domain kind plus `available` and `installed` facts;
- stable diagnostic severity, code, and optional canonical target.

`available` means the capability is valid for the captured platform in the effective snapshot.
`installed` means Shine has manifest/receipt evidence or the target launcher is present; it does
not mean an external program was detected. Launcher ownership/conflict detail remains an inspection
concern. The report includes the union of available and installed targets,
sorts by kind and canonical identity, and diagnoses installed targets whose current Preset is
missing. A local service failure retains its source error only in a non-serializable wrapper; the
stable diagnostic never copies that source.

The existing `RuntimeEvent` remains a local presentation side channel. A future event contract must
be a new explicit safe projection, not `Serialize` added to `RuntimeEvent`.

## Approval ownership

A Plan or serialized review request never proves approval. A trusted CLI or UI may create a
process-local `PlanApprovalV1` only after an affirmative human action over the freshly displayed
ready Plan. An AI or MCP adapter may request review but must not expose approval construction,
accept an approval-shaped payload from a model, forward `--yes` authority, or call mutation on the
user's behalf. Apply must still recapture state, regenerate the exact Plan, and validate its
fingerprint and permission set.

## Delivery sequence

1. **Slice 6A — inventory contract:** add `FrontendService`, Contract v1 inventory and diagnostics,
   then migrate `shine list` without output changes.
2. **Slice 6B — inspection and Plan review:** add redacted inspection and review reports that reuse
   `PlanV1`; migrate CLI info/status/Plan collection and establish cross-adapter fixtures.
3. **Slice 6C — operation state, events, and recovery:** define journal-derived operation state and
   safe progress events; keep private presentation details out of serializable reports.
4. **Slice 6D — trusted mutation and conformance:** expose approved lifecycle/recovery calls only to
   trusted frontends and prove CLI/UI/MCP projections agree for the same captured snapshot.

## Slice 6A acceptance

- In-memory fixtures cover available-only, installed-only, combined, empty, and missing-Preset
  inventory across all three domains.
- JSON golden tests lock schema version, enum spellings, deterministic order, and redaction.
- `shine list` retains its existing sections, sorting, empty-state message, exit behavior, and
  legacy App/Shell visibility rules.
- No generator, hook, artifact, bootstrap script, detection command, mutation host method, or
  administrator authorization runs during inventory.
- Architecture KB and ADR 0077 describe the implemented boundary; public manuals remain unchanged.
