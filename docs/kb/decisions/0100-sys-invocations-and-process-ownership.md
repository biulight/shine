# 0100 — Sys invocation snapshots and process ownership

- **Status**: Accepted
- **Date**: 2026-10-08
- **Evidence**: `core/src/runtime/{sys_manifest,sys_bootstrap,planner,host,process_scope}.rs`
- **Extends**: ADR 0092, ADR 0093

## Context

A shared Sys directory swap let one approved invocation consume another invocation’s captured
helpers. Materialization also discarded executable intent. Isolated process groups bounded effects
on timeout, but cancellation bypassed group cleanup and inherited terminal reads stopped on SIGTTIN.

## Decision

Sys scripts use a private UUID directory below `runtime/sys/<os-id>/`, containing only captured
bytes and executable intent. Preparation failures and completed process invocations clean only
that directory. Legacy shared trees remain untouched. Plans bind creation and cleanup to the
existing category runtime scope; the random directory identity is never review data.

A process-scope drop guard retains the isolated Unix group ID until execution completes. Error, failed foreground exit,
and cancellation kill that group, including descendants surviving the direct child. Stop failed
groups before waiting for descendants to close inherited output pipes. Successful
execution disarms group termination. For inherited foreground terminal stdin, hand the terminal
to the child group, resume a child stopped before handoff, and restore the original group on every
exit. Block SIGTTOU only on the calling thread during synchronous foreground-group changes.

## Consequences

- Another Sys invocation cannot replace this invocation’s executable inputs.
- Helpers keep captured execute intent without source ownership or setuid/setgid bits.
- Abrupt executor termination may leave a private directory, which is never reused.
- Snapshot delivery does not sandbox scripts or serialize arbitrary effects on shared host state.
- Independent PTY fixtures verify success, timeout, and cancellation without touching the user’s
  terminal; their Unix test driver requires Python 3.
