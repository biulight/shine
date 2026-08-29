# 0034 — Preset platform selectors distinguish exact operating systems

- **Status**: Accepted
- **Date**: 2026-08-27
- **Evidence**: `utils/src/runtime/{context,app_metadata,shell}.rs`

## Context

App destinations and App/Shell file filters previously recognized only `unix` and `windows`.
That grouped macOS and Linux even when an application existed on only one of them. The built-in
Surge preset was consequently visible and installable outside macOS despite its Surge-specific
destination and hooks.

## Decision

Preset metadata supports exact `macos`, `linux`, and `windows` selectors. `unix` remains a
compatibility group matching macOS and Linux, so existing presets keep their meaning. For App
destination maps, an exact macOS/Linux branch overrides `unix`; a missing effective branch omits
the category or file on that OS. Windows uses only its exact branch.

`platforms` arrays use OR semantics and must not be empty. Destination maps must contain at least
one supported key. Static validation checks every declared destination and evaluates the effective
metadata independently for all three exact operating systems on every host, including duplicate
command and destination detection where exact and group selectors overlap.

## Consequences

- macOS-only and Linux-only App categories and App/Shell files are representable without runtime
  failure as the availability mechanism.
- `unix` and `windows` declarations remain source-compatible.
- `linux` describes the OS family; there is no `ubuntu` alias.
- Unsupported installed App entries are not deleted automatically. Manifest-driven uninstall and
  the existing stale-entry policy remain responsible for safe cleanup.
