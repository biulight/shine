# 0159 — Default code review focuses on concrete decisions

- **Status**: Accepted
- **Date**: 2026-10-31
- **Updates**: ADR 0091, ADR 0094
- **Evidence**: `cli/src/lifecycle_plan/render/`, `cli/src/lifecycle_plan.rs`

## Context

Built-in Shell installation displayed distribution trust followed by two generic warnings about
unisolated code and the absence of per-invocation review. Required permissions mixed concrete
changes with an unrestricted-code marker and optional author command statements. This made a
normal installation look like a trust failure and suggested runtime restrictions that do not exist.

## Decision

Default lifecycle and scoped bootstrap review show concrete planned effects, code source, execution
timing and trust. External code consent, shared-resource-affected targets, missing declarations,
privilege, preservation and unknown/blocking diagnostics remain visible. Routine Shell cache
transaction codes are detailed-view information.

Default review omits the generic execution disclaimers, author capability statements and abstract
opaque-code permission marker. Detailed review retains them, exact permissions and identities.
Incomplete bootstrap attribution continues to fall back to the full review, preserving evidence.

This is a terminal projection only. Plan data, fingerprints, snapshot trust, confirmation,
ownership, fresh-state validation and execution are unchanged. Built-in metadata, including
`commands = ["shine"]`, stays intact: hiding implementation detail must not rewrite preset snapshots.
No command-name special case is introduced.

The built-in Shell installation layout is further refined by
[ADR 0160](0160-built-in-shell-install-summary.md).

## Validation

Exercise the embedded utils Plan and embedded bootstrap plans, compact and detailed review,
external trust and unknown blockers. Assert that rendering preserves the full serialized Plan and
fingerprint. Existing approval and trust tests remain the execution-boundary coverage.
