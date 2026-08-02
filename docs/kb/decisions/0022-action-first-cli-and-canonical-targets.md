# 0022 — Everyday CLI operations are action-first and use canonical targets

- **Status**: accepted
- **Evidence**: `cli/src/commands/`, `cli/src/shim.rs`, `cli/src/info/`,
  `cli/src/self_install.rs`, `cli/src/completion.rs`

## Context

While preparing the 1.0 vocabulary, Shine still exposed two overlapping navigation models. Users
could start from an action (`install`, `list`, `info`, `update`, `upgrade`) or a resource namespace
(`app`, `shell`, `sys`), but the accepted target syntax and available flags differed. A bare
install category could also trigger an interactive app/shell choice, while `info` and preset copy
already used canonical resource prefixes.

The lifecycle vocabulary had accumulated adjacent operations whose differences were difficult to
infer from their names: `install`, `reinstall`, `upgrade`, app `refresh`, app `build`, and sys
`init`. The capabilities remain distinct, but they do not all need equal prominence.

## Decision

The everyday interface is action-first. `install`, `uninstall`, `info`, `update`, and `upgrade`
accept canonical targets such as `app/starship`, `shell/proxy`, and `sys/split-dns` where the
operation supports that resource. A bare app/shell category remains a shorthand only when it is
unique. Ambiguity is an error with canonical choices, never an interactive prompt in the resolver
used by scripts.

`shine install <TARGET> --replace-managed` replaces `reinstall` as the repair flow. Because this
decision predates the first stable 1.0 release, the old top-level and scoped `reinstall` spellings
are removed instead of retained as aliases.

`shine list` remains the installed inventory. `shine list --available [app|shell|sys]` is the
unified catalog, and top-level `shine info` falls back to available app/shell metadata when the
target is not installed. Installed-only `--diff` and `--verbose` behavior is unchanged.

`shine upgrade [TARGET]` applies either the global upgrade or exactly one app category, shell
category, or managed system item. A file/command target deliberately upgrades at its owning
category boundary. A targeted upgrade must not traverse or mutate entries owned by other targets.

Advanced or author-oriented operations use explicit namespaces:

- preset templates: `shine preset new app|shell`;
- system bootstrap: `shine sys bootstrap`;
- app artifacts: `shine app artifact apply|remove`;
- secret operations: `shine env secret encrypt|decrypt|export|seal|identity`.

Their superseded pre-release spellings are not accepted. This keeps the first stable command
grammar singular instead of beginning 1.0 with compatibility debt.

The read-only `update` versus mutating `upgrade` boundary remains unchanged.

## Consequences

- Common workflows use six stable verbs and one target grammar.
- Canonical targets make terminal and non-interactive behavior identical.
- Superseded pre-release spellings fail explicitly, so scripts converge on one grammar.
- Dynamic completions must offer canonical targets as well as unambiguous shorthand.
- Help, README examples, preset hooks, and the command-routing documentation must use the new
  primary spellings.
