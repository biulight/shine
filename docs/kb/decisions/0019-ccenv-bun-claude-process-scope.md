# 0019 — Bun ccenv with Claude-process scope

- **Status**: accepted
- **Evidence**: `presets/shell/agent/cc.ts`, `presets/shell/agent/shine.toml`
- **Supersedes**: the platform-specific sourced `cc.sh` / `cc.ps1` implementation.

## Context

Each provider addition duplicated selection, credentials, environment mappings, argument
forwarding, and tests across Bash and PowerShell. The Unix script was also advertised for every
Unix shell even though its syntax was not valid in Fish or Elvish. Sourcing existed only to leave
Claude-specific variables in the parent shell.

## Decision

`ccenv` is a cross-platform `runtime = "bun"` command. It selects a provider, builds a clean child
environment, and always launches Claude; it does not mutate the parent shell. Provider definitions
live in one TypeScript registry.

Credentials resolve in this order: tagged `KEY_SECRET`, legacy `KEY_GPG_SECRET`, then plaintext
`KEY`. Once a secret exists, a decryption failure is terminal and never falls back to another
credential. `-r` and `--run` remain argument-free compatibility aliases, while a leading `--`
passes a conflicting argument to Claude.

## Consequences

Adding a provider changes one registry and its table-driven tests. Bun and shine must be available
at execution time. Running plain `ccenv` now starts interactive Claude; users who need a general
parent-shell export continue to use `shine env export`.
