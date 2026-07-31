# 0020 — Shine 1.0 uses resource-scoped commands and a fixed inspection vocabulary

- **Status**: accepted
- **Evidence**: `cli/src/commands/`, `cli/src/main.rs`, `cli/src/completion.rs`

## Context

As Shine grew, preset-source operations occupied five top-level command names and inspection used
both `show` and `info` for similar concepts. The generic `clear` command also concealed that it ran
versioned runtime-state migrations. Shine 1.0 is the boundary for making this public interface
coherent without carrying pre-1.0 aliases.

## Decision

Collections use `list`, one-resource details use `info`, operational state uses `status`, and a
single scalar uses `get`. Preset-source operations live under `shine preset`; overlay inspection is
`shine preset overlay info`. Versioned runtime cleanup is `shine state migrate`, and forcing the
release check is `shine update --refresh-release`.

`shine shell info` accepts a category, command, or canonical `category/command`. Top-level system
inspection is deliberately explicit as `shine info sys/<ITEM>` so bare installed app/shell target
resolution does not change. App/shell-only content flags are rejected for system targets.

The former command names are removed rather than retained as aliases. `update --pull` and
`upgrade --pull` remain because they compose preset synchronization with their parent operation.

## Consequences

- Scripts must move to the 1.0 command spellings before upgrading.
- The top-level help surface is smaller while common install/update commands remain short.
- Help, completion candidates, documentation, and parser rejection tests form part of the public
  CLI contract.
- ADR 0007's pull safety policy and ADR 0010's managed-overlay policy are unchanged; only their
  public command paths move under `shine preset`.
