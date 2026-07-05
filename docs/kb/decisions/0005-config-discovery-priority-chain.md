# 0005 — Config/presets locations resolve through a fixed priority chain

- **Status**: accepted
- **Evidence**: `cli/src/config/discovery.rs`, `AGENTS.md` § Config

## Context

Tests, sandboxed agent sessions, external preset checkouts, and normal user installs all need
different config/presets locations without editing code.

## Decision

`Config::load_or_init()` resolves directories in this order (highest wins):

1. `SHINE_CONFIG_DIR` env var — overrides both the shine dir and the presets dir
   (presets become `$SHINE_CONFIG_DIR/presets/`)
2. `SHINE_PRESETS` env var — overrides the presets dir only
3. `presets_dir` key in `config.toml`
4. Default: `~/.shine/` and `~/.shine/presets/`

Project-local configs inherit unset keys from the global config (commit `a5aed62`).

## Consequences

- Tests and agent sessions isolate state with
  `SHINE_CONFIG_DIR=$PWD/.tmp-home/.shine` (see `AGENTS.md` § Verification Notes).
- Any new directory-dependent feature must resolve through this chain, not hardcode `~/.shine/`.
- Behavior differences between "embedded" and "external presets" modes follow from which level
  of the chain is active — check the active mode before debugging preset lookups.
