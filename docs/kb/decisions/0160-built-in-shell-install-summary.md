# 0160 — Built-in Shell installation uses a task summary

- **Status**: Accepted
- **Date**: 2026-10-31
- **Extends**: [ADR 0159](0159-concise-default-code-review.md)
- **Evidence**: `cli/src/lifecycle_plan/render/install_summary.rs`, `cli/src/shells/install.rs`

## Decision

A ready, ordinary built-in Shell install uses Source, Commands, Changes and Backup rows and the
short Apply prompt. Plain environment requirements add an Environment row containing sorted,
deduplicated variable names between Changes and Backup. Secret requirements keep expanded review.
Variable names use stdout-aware blue styling after control escaping; ordinary redirection and
`NO_COLOR` retain plain text under the existing terminal color policy.
The default install path omits the redundant built-in source banner. Verbose
review retains the expanded Plan and source note.

The summary requires distribution-trusted Shell delivery, selected targets, recognized install
steps and diagnostics, and filesystem provenance for every concrete effect. Multiple maintenance
attributions may describe one cache write only when their purposes agree. User destinations are
shown explicitly; recovery associations supply the backup row. No ownership is inferred from path
spelling. Unknown effects, forced replacement, execution during installation, shared affected code,
external trust, permissions other than the summarized filesystem and plain environment requirements,
conflicts and blocking diagnostics keep the expanded review.

This changes no Plan data, metadata, trust, approval, recovery or execution semantics. Existing
confirmation and fresh-state checks still authorize the full Plan. Other lifecycle operations keep
their existing layouts.

## Validation

Compare the whole utils and image-tools summaries and prompts with the accepted output, using an
embedded snapshot. Isolated subprocesses run the real built-in install handler, compare its complete
review stdout in forced-color, redirected and `NO_COLOR` modes, check the Environment value's blue
style and absent source banner, and verify installed commands and preserved profile content.
Negative cases include secret requirements alongside plain requirements of the
same name, unrelated permissions, expanded-review fallback and immutable Plan data.
