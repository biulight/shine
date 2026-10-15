# 0006 — The cli crate is a library with a thin bin entry point

- **Status**: accepted
- **Evidence**: commit `dad2821` (refactor: split bin crate into lib + thin main), `cli/src/lib.rs`,
  `cli/src/main.rs`, `cli/src/test_support.rs`

## Context

A single large bin crate made unit testing awkward and forced everything through `main.rs`.
A phase 1–3 reorganization (commits `dad2821`…`740be5f`) split the code into focused modules.

## Decision

`cli/` builds a library crate `cli` containing all logic, plus a thin `main.rs` bin root that owns
process setup and delegates `run()` dispatch to the CLI library. Clap arg types live in the lib (`commands/cli.rs`) because
`completion.rs` needs them.

A later pass (commit `7f7e739`) moved the last two handler groups out of `main.rs` proper:
`env show/set/delete/get/decrypt/export/encrypt` became `env::commands` in the lib crate (since
`env/` is already a lib module), and the top-level `install/reinstall/uninstall <category>` shim
resolution became `cli/src/shim.rs`, a bin-local module declared via `mod shim;` — the same
pattern `presets_commands.rs` and `self_install.rs` already used.

## Consequences

- New logic goes in the lib, not `main.rs`; `main.rs` should stay a dispatcher.
- `#[cfg(test)]` does **not** cross the lib/bin boundary. Shared test helpers such as
  `test_support::env_lock()` are compiled unconditionally into the lib on purpose — do not
  "fix" this by gating them.
- Integration-style tests can exercise the lib directly without spawning the binary.

## 2026-10-15 follow-up

[ADR 0111](0111-lifecycle-phase-and-cli-dispatch-refactoring.md) moves command dispatch and parser
regressions into `cli/src/dispatch/`. The binary retains early completion, argument parsing,
pre-runtime configuration selection and Tokio setup. Initialization-free commands remain separate
from configured dispatch so moving handlers cannot silently initialize state.
