# 0095 — Hide unchanged scopes in compact upgrade reviews

- **Status**: Accepted
- **Date**: 2026-10-04
- **Evidence**: `cli/src/lifecycle_plan.rs`
- **Extends**: ADR 0071, ADR 0094

## Context

An untargeted `shine upgrade` still plans all installed Shell, App, and managed Sys targets. The
compact review showed scopes containing only unchanged steps, skipped manual App generators, and
their unverified author capability statements. This made a single pending Shell update appear to
include unrelated App and Sys updates.

## Decision

The default compact upgrade review omits a scope when it has no action, required permission, code
boundary, missing declaration, uncomputable permission, or exceptional diagnostic. Unchanged steps
and skipped manual App generator diagnostics do not make a scope visible; they are also omitted from
visible scopes. [ADR 0097](0097-relevant-verbose-upgrade-review.md) applies the same relevance
rule to `--verbose`. If every scope is omitted, the CLI shows no Security Plan and the normal
lifecycle result says `Nothing to upgrade.` without confirmation.

Actual effects, preservation, blockers, required permissions, code boundaries, and unknown
diagnostics remain visible. The complete Plans still bind approval, are regenerated before apply,
and retain all installed targets. This changes presentation only, not target selection or mutation.
Skipped manual generators remain outside routine upgrade; `shine update --run-generators` can
assess them separately.
