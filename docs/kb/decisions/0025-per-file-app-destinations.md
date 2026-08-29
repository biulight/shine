# 0025 — App files may override the category destination

- **Status**: accepted
- **Evidence**: `utils/src/runtime/{app,app_metadata}.rs`,
  `cli/src/apps/upgrade.rs`, `presets/app/clash-verge/shine.toml`

## Context

An app category historically had one destination root. That fits conventional dotfile categories,
but not a preset whose source/config snapshot belongs under Shine while a small declared subset must
live in another application's user-data directory. Implementing the second write in an artifact
would bypass the app manifest's backup, modification guard, status, and uninstall semantics.

Clash Verge's local file-provider example exposed this gap: `merge.yaml` belongs under
`~/.shine/clash-verge`, while mihomo accepts its local rule lists only inside HomeDir (unless the user
widens `SAFE_PATHS`).

## Decision

An explicit app `[[files]]` entry may declare `dest`. It overrides the category `dest` for that file;
files without it are unchanged. Absolute strings and existing platform maps keep their current
meaning. A file may also use `{ base = "data-dir", path = "..." }`, resolved against the platform's
per-user application-data directory only when an active `Config` is available, preserving test-home
isolation. The rooted path and `target` are relative and reject parent traversal.

Effective destinations must be unique across the active app metadata before install or upgrade
writes. Manifest identity is one entry per preset `source`. If a metadata update changes that
source's effective destination, upgrade relocates only when the old copy is still manifest-current
and the new path is free; otherwise it keeps both filesystem locations unchanged and reports the
conflict. Manifest upsert replaces by source or destination so a successful move cannot retain two
owned entries.

## Consequences

- Presets can use ordinary Copy/JSON-merge lifecycle safety across multiple destination roots.
- `app info`, dry-run, refresh, status, upgrade, and uninstall share the same effective destination
  resolver; artifact scripts do not need a parallel ownership ledger.
- A missing platform branch on a file destination omits that file, matching category destination
  selection. The category-level `dest` remains required for compatibility and simple defaults.
- `data-dir` describes standard installed-app data. Portable third-party applications with private
  roots still require a custom file destination.
