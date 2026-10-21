# ADR 0113: Capture runtime inputs once and scope test resources

- **Date**: 2026-10-16
- **Status**: Accepted

## Context

EnvConfig's async load/init API had become a synchronous copy of resolved Config fields. CLI
adapters copied those fields back into an already captured runtime and retained broad Context
mutation access. Sys installer timeout classification depended on English error text. Local Bun
hook coverage omitted several maintained presets. Test cleanup lost original env values, while
the admin test directory lock could revoke a live owner or silently ignore acquisition errors.

## Decision

- Use synchronous `EnvConfig::from_config` for env read/edit views. Runtime assembly captures
  `Config.env`; adapters do not recopy it. Keep pending edits and Config persistence unchanged.
- Remove CLI access to arbitrary runtime Context mutation. Core unit tests retain a private
  test-only accessor. Sys bootstrap supplies its explicit proxy map through consuming
  `with_proxy_env`; fresh Plan and exact approval checks remain mandatory.
- Classify Sys installer timeouts using `std::io::ErrorKind::TimedOut` through anyhow context,
  deriving display duration from the existing timeout constant. Other failures remain errors.
- Match all maintained preset TypeScript and tooling inputs in the Bun hook. Check the actual
  filter against the source tree in the ordinary Bun test suite. Local/CI entry points use
  `bun run --bun check:ts` so nested scripts retain the mise-selected runtime; tests verify the
  running Bun version against `mise.toml`.
- Keep one shared test mutex across the CLI library/binary boundary. Its guard captures env/cwd
  and restores them before unlocking, including during panic unwinding. Test mutations must hold
  that guard; child Command environments remain the preferred isolation boundary.
- Reuse Core's persistent OS file locks for admin-category tests under a distinct `.lockfile`
  path, with the existing bounded wait and fail-closed acquisition. Never unlink to reclaim a lock.

## Consequences

Commands, env precedence, encrypted-first lookup, pending writes, approval fingerprints, receipts
and journal schemas retain their contracts. Tests cover typed failure routing, environment capture,
proxy approval invalidation, hook coverage and process-state/OS-lock lifecycle behavior. Native
platform evidence must be reported separately from local validation.
