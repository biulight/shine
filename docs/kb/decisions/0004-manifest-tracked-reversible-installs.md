# 0004 — App installs are manifest-tracked and reversible

- **Status**: accepted
- **Evidence**: `cli/src/apps/manifest.rs`, `cli/src/apps/file_ops.rs`

## Context

`shine app install` writes into user-owned locations (`~/.gitconfig`,
`~/.config/starship.toml`, even `/etc/docker/daemon.json`). Uninstall must remove exactly what
shine installed and nothing else, and must restore what was there before.

## Decision

Every installed file is recorded as an `AppEntry` in `~/.shine/app-manifest.toml` (dest, content
hash, strategy, `requires_admin`). Pre-existing user files are backed up to `<name>.shine.bak`
before overwrite. Uninstall and upgrade are driven purely by the manifest: remove the tracked
file (via sudo when the entry says `requires_admin`), restore the backup, drop the entry.

## Consequences

- Uninstall is safe by construction — it cannot touch files shine never installed.
- The manifest schema is load-bearing: fields must survive serialization round-trips
  (see lessons entry on `requires_admin`, commit `70ee910`).
- `shine upgrade` can find and refresh every installed file without re-scanning the system.
