# 0010 — shine-managed Git overlay is a depth-1 force-mirror

- **Status**: accepted
- **Evidence**: `cli/src/config/mod.rs` (`presets_overlay_git`, `managed_overlay_dir`,
  `overlay_git_source`, `active_presets_overlay_dir`), `cli/src/git_pull.rs`
  (`sync_managed_overlay`), `cli/src/preset_commands.rs` (`shine preset overlay link --git`)

## Context

An overlay could already be a Git checkout, but every machine had to `git clone` it somewhere and
then `shine preset overlay link <path>` that path. For a user who maintains the overlay on one machine and
only *consumes* it elsewhere, that manual clone/link step on each device is pure friction, and a
full-history clone wastes disk on devices that never need the history. See ADR 0007 for how manual
overlay/preset checkouts are fast-forward-pulled.

## Decision

Add `presets_overlay_git` (+ optional `presets_overlay_git_branch`) to `config.toml`. When set (and
no manual `presets_overlay_dir` is configured), shine owns the checkout at `<shine_dir>/overlay`
(so it follows `SHINE_CONFIG_DIR`). `shine preset pull` clones it `--depth 1` on first use, then
**force-mirrors** it to the remote tip on every subsequent run: `git fetch --depth 1 origin
<branch>` followed by `git reset --hard FETCH_HEAD`. `shine preset overlay link --git <url>` writes the
config and clones immediately; a manual `overlay link <path>` and a Git URL are mutually exclusive
(setting one clears the other).

Force-mirror (not fast-forward) was chosen deliberately: the managed checkout is a read-only mirror
of an overlay maintained elsewhere, so it should always equal the remote — even when the maintainer
rebases, amends, or force-pushes — with no merge conflicts to resolve on consumer devices. This
matches the request's "只拉取最新的，无需关注 git 历史" intent.

## Consequences

- Consumer devices need only the URL (in `config.toml` or via `overlay link --git`); no manual
  `git clone`, and `--depth 1` keeps storage minimal.
- Local edits to the managed checkout are discarded on the next pull. Users who want to *edit* the
  overlay use the manual `overlay link <path>` mode against their own working clone instead.
- The old checkout stays usable when a sync fails: the fetch runs before the reset (an unreachable
  remote never touches the working tree), and a first clone lands via a temp dir + atomic rename (a
  failed clone leaves no half-populated overlay). Overlay *consumption* depends only on the checkout
  existing on disk, so a failed `shine preset pull` leaves the last-good overlay in place.
- The managed overlay is excluded from the fast-forward pull path (`configured_targets`); only
  manual `presets_overlay_dir` checkouts go through `git pull --ff-only` (ADR 0007).
