# 0097 — Show relevant scopes in verbose upgrade review

- **Status**: Accepted
- **Date**: 2026-10-04
- **Evidence**: `cli/src/lifecycle_plan.rs`
- **Extends**: ADR 0071, ADR 0095

## Context

After the compact upgrade review stopped showing unchanged scopes, `upgrade --verbose` still
printed every unchanged Shell command, skipped manual App generator, and unchanged App and Sys
scope. These entries obscured the paths and permissions that mattered for approval.

## Decision

The verbose upgrade review applies the same relevance rule as the compact review: omit scopes
without an action, required permission, code boundary, blocker, or exceptional diagnostic. Within
a visible scope, omit ordinary unchanged steps and manual App generators skipped by routine
upgrade. Expand the remaining steps, diagnostic codes, exact permissions, snapshot identities,
and fingerprint. Other lifecycle operations retain their complete detailed rendering.

`upgrade --verbose --full-plan` retains the unabridged terminal rendering for auditing. The
`--full-plan` option requires `--verbose` and changes display only.

Filtering changes only the terminal projection. The complete Plans, including omitted steps and
scopes, still bind approval and are regenerated before apply. A permission-only or blocked scope
must remain visible even without an action step.
