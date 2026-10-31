# 0160 — Built-in Shell installation uses a task summary

- **Status**: Accepted
- **Date**: 2026-10-31
- **Extends**: [ADR 0159](0159-concise-default-code-review.md)
- **Evidence**: `cli/src/lifecycle_plan/render/install_summary.rs`, `cli/src/shells/install.rs`

## Decision

A ready, ordinary built-in Shell install uses Source, Commands, Changes and Backup rows and the
short Apply prompt. The default install path omits the redundant built-in source banner. Verbose
review retains the expanded Plan and source note.

The summary requires distribution-trusted Shell delivery, selected targets, recognized install
steps and diagnostics, and filesystem provenance for every concrete effect. Multiple maintenance
attributions may describe one cache write only when their purposes agree. User destinations are
shown explicitly; recovery associations supply the backup row. No ownership is inferred from path
spelling. Unknown effects, forced replacement, execution during installation, shared affected code,
external trust, extra permissions, conflicts and blocking diagnostics keep the expanded review.

This changes no Plan data, metadata, trust, approval, recovery or execution semantics. Existing
confirmation and fresh-state checks still authorize the full Plan. Other lifecycle operations keep
their existing layouts.

## Validation

Compare the whole utils summary and prompt with the accepted output, using an embedded snapshot.
An isolated subprocess runs the real built-in install handler and compares its complete review
stdout, checks that no source banner was emitted, and verifies installed commands and preserved
profile content. Negative cases exercise expanded-review fallback and immutable Plan data.
