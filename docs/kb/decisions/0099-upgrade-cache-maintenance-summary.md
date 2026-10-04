# 0099 — Summarize routine upgrade cache maintenance separately

- **Status**: Accepted
- **Date**: 2026-10-04
- **Evidence**: `cli/src/lifecycle_plan.rs`
- **Extends**: ADR 0071, ADR 0095, ADR 0097

## Context

Status reports pending user-resource updates, while global upgrade also converges installed
categories’ internal caches. Missing App cache copies and changed Shell metadata made a single
Shell update appear to update unrelated applications even after unchanged steps were hidden.

## Decision

Compact upgrade review replaces routine App per-file cache creates/updates and successful Shell
cache replacement steps with one internal source-copy maintenance summary counting distinct
categories. Verbose and full review keep their original category/file steps. Non-upgrade reviews
retain their existing rendering.

Only known routine writes qualify: App cache steps without diagnostics and Shell cache replacement
steps with only the expected transaction diagnostic. Removal, preservation, blockers, and unknown
diagnostics remain explicit. Scope permissions and code boundaries remain visible. The complete
Plan, fingerprint, approval, and execution are unchanged; cache-only work still requires approval.

For ready App scopes containing only routine cache writes, ordinary no-ops, and known stale-source
preservation notices, move cache permissions under the internal maintenance summary and render
preservation notices under Warnings. Move only filesystem-write permissions uniquely attributed
by Core to maintenance for the cache targets, with complete required permission membership.
Code boundaries, author statements, actual App changes, blockers, unexpected diagnostics, and
ambiguous/missing provenance retain the original App section. Warning-only scopes need no
permission section. Verbose/full reviews keep their original scope structure.
