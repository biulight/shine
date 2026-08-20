# 0027 — Sys standardizes bootstrap execution, not software management

- **Status**: accepted
- **Evidence**: `cli/src/sys/{bootstrap,selection,profile_compose,profile_commands}.rs`,
  `presets/sys/*/shine.toml`

## Context

Sys preset authors previously had to add each software item to a platform-wide dispatcher, emit a
private status protocol, repeat package-manager and detection logic, and edit a shared shell profile.
That made a simple ensure-present item look like a small software manager and made targeted bootstrap
carry unrelated shell integrations.

Selection profiles also became easy to confuse with shell profiles. The former select work for one
invocation; the latter are persistent Shine-owned shell content and need independent activation
state.

## Decision

`shine sys bootstrap [ITEM]...` accepts ordered, deduplicated init items and is mutually exclusive
with `--preset`. Standard items declare a read-only `command`, `path`, or `any` detection and either
a fixed package provider (`homebrew`, `homebrew-cask`, `apt`, or `winget`) or one per-item script.
Rust owns provider argv, elevation, proxy handling, timeout, bounded output, post-install detection,
dry-run, outcomes, and receipts. It offers ensure-present only: package managers and upstream tools
continue to own versions and upgrades, while `sys update` remains read-only.

OS manifests opt into `profile_composition`. Base pre/post files contain only platform-wide setup;
each init item declares bounded PATH, env, guarded eval/source, aliases, or an item-owned fragment.
The composer orders by phase, priority, manifest order, and declaration order, then reconciles the
result through the existing per-phase sentinels. `profile_enabled` in `sys-manifest.toml` is the
minimal persistent activation state. Selection profiles only select bootstrap items and never
disable previously enabled integrations. `sys profile enable/disable` changes only generated
Shine-owned integration content.

Embedded sys code is trusted with the binary. Bootstrap/managed scripts and persistent executable
profile content from a complete external source or overlay require the separate `allow_sys_code = true`
global opt-in. Project configuration cannot grant this permission to its own preset code. Static
detection, provider metadata, PATH, env, and aliases do not require it.

Legacy platform dispatch remains a per-item fallback during migration. An item with `[items.install]`
always uses the standard executor; an item without it uses the legacy dispatcher, so both paths can
never install the same item in one run.

## Consequences

- Adding an ordinary package requires metadata, not lifecycle shell protocol knowledge.
- Targeted bootstrap and profile activation no longer pull unrelated software or integrations.
- Complex product-specific installers remain possible but are isolated and permission-gated.
- `profile_enabled` records shell integration intent, not installation provenance or package state.
- Builtin migration can proceed item by item while old update checks remain compatible.
