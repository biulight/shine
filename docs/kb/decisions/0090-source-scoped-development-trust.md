# 0090 — Development trust is source-scoped

- **Status**: Superseded in part by ADR 0091
- **Date**: 2026-09-16
- **Evidence**: `core/src/trust.rs`, `core/src/runtime/trust.rs`, `cli/src/trust.rs`

## Context

Snapshot-scoped trust is the safe default for third-party Presets, but it makes an author re-enroll
their own target after every code edit. `opaque_code = "unrestricted"` describes effects; it cannot
authorize future code because the Preset author controls that declaration.

## Decision

`shine trust grant <TARGET> --development` explicitly enrolls a development grant. The grant remains
target- and capability-local and binds the exact permission set plus the current physical Preset
source roots and source layers. Its source identity is derived by Core from the captured snapshot;
Preset metadata and project configuration cannot supply it. Code-digest changes from the same
source match the grant. A target, capability, permission, source-root, or source-layer change does
not match it.

The owner-only trust store records the source identity and local display labels so `trust list` and
`trust inspect` can distinguish development trust. Security Plan presentation marks active
development grants. Ordinary `trust grant` continues to create snapshot grants, and existing grants
deserialize as snapshot grants. Revocation treats both modes identically.

Development trust does not replace administrator authorization, ownership checks, executor
allowlists, environment-input identity, or per-mutation snapshot-bound Plan approval.

## Consequences

- Preset authors can edit enrolled local code without repeating trust enrollment.
- Moving the source, adding an overlay source, or expanding permissions requires another review.
- A third-party Preset cannot opt itself into development trust.
- Source display labels are machine-local security state and remain in the private trust store; they
  never enter portable Presets, bundles, or frontend-safe reports.
