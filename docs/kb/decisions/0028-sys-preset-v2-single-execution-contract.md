# 0028 — Sys preset v2 has one bootstrap execution contract

- **Status**: accepted; supersedes the gradual legacy fallback and bootstrap update-check portions of ADR 0027.
- **Evidence**: `cli/src/sys/{manifest,bootstrap,commands,profile_compose}.rs`, `presets/sys/*/shine.toml`

## Decision

Executable sys manifests declare `version = 2`. Every init item declares a read-only detection and
an install action: a fixed provider or an isolated item script. The runtime rejects v1, missing, or
unknown versions before any detection, privilege escalation, installer, or profile write. There is
no platform dispatcher, status wire protocol, or bootstrap-software update checker.

Managed resources retain their receipt-based desired-state lifecycle. `sys-manifest.toml` remains
backward-readable; entries without `profile_enabled` keep their enabled default, and only current
v2 items with integrations participate in composition.

## Consequences

- Preset authors have one auditable lifecycle and no private text protocol to implement.
- Third-party software updates belong to Homebrew, APT, Winget, mise, rustup, or upstream tools.
- External executable sys content remains global-opt-in through `allow_sys_code = true`.
