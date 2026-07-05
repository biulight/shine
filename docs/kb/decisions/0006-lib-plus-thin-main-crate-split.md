# 0006 — The cli crate is a library with a thin bin entry point

- **Status**: accepted
- **Evidence**: commit `dad2821` (refactor: split bin crate into lib + thin main), `cli/src/lib.rs`,
  `cli/src/main.rs`, `cli/src/test_support.rs`

## Context

A single large bin crate made unit testing awkward and forced everything through `main.rs`.
A phase 1–3 reorganization (commits `dad2821`…`740be5f`) split the code into focused modules.

## Decision

`cli/` builds a library crate `cli` containing all logic, plus a thin `main.rs` bin root that
does `run()` dispatch and a few inline handlers (`init`, `env`, completions). Clap arg types
live in the lib (`commands/cli.rs`) because `completion.rs` needs them.

## Consequences

- New logic goes in the lib, not `main.rs`; `main.rs` should stay a dispatcher.
- `#[cfg(test)]` does **not** cross the lib/bin boundary. Shared test helpers such as
  `test_support::env_lock()` are compiled unconditionally into the lib on purpose — do not
  "fix" this by gating them.
- Integration-style tests can exercise the lib directly without spawning the binary.
