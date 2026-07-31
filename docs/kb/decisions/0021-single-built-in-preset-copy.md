# 0021 — Single-preset copy creates a removable overlay snapshot

- **Status**: accepted
- **Evidence**: `cli/src/preset_commands.rs` (`shine preset copy`),
  `cli/src/presets.rs` (`extract_embedded_prefix`)

## Context

Customizing presets such as Surge and Clash Verge previously required finding their examples in
the source repository and manually recreating the matching paths in a presets overlay. Exporting
every built-in preset was excessive when only one category was needed.

Copying only metadata-declared data files would reduce cleanup, but it would also create an
implicit and app-specific definition of “customizable.” Users sometimes need to fork metadata or
artifact scripts as well as payload files.

## Decision

Add `shine preset copy <kind>/<name> [--force]`. The target must be exactly one canonical category
under `app`, `shell`, or `sys`. The command copies the category's complete snapshot from the
current binary into the current directory while preserving its `kind/name/` prefix. It reads the
embedded bundle directly and never merges the active external presets source or overlay.

Existing files are skipped unless `--force` is explicit. The command does not activate or modify
configuration. After copying, users delete every file they do not intend to customize and link the
current directory as their overlay if needed.

## Consequences

- One command replaces repository browsing and manual path reconstruction.
- Files left in the overlay are intentional snapshots and do not receive built-in updates.
- Deleted files continue to resolve through the existing per-path fallback to the built-in preset.
- A complete copy supports advanced metadata and artifact customization without additional flags
  or a policy for classifying files as user-editable.
