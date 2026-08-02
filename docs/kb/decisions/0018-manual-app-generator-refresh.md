# 0018 — Manual app generator refresh

- **Status**: accepted
- **Evidence**: `cli/src/apps/refresh.rs`, `cli/src/apps/metadata.rs`,
  `presets/app/surge/shine.toml`
- **Amends**: ADR 0016's assumption that every enabled generator is remote
  desired state polled by update and upgrade.

## Context

Some subscription providers expose a configured URL only during a short access
window. Treating such a generator as ordinary remote desired state consumes
that window during unrelated `shine update`, `list`, and `info`, and
`upgrade` commands. A time-based cache only postpones the same uncontrolled
request and cannot know when the provider's window is open.

Using `reinstall` as the manual trigger would force every managed file in the
category to be rewritten, and does not allow selecting one of several generated
files.

## Decision

Generator metadata gains `auto`, defaulting to true for compatibility.
`auto = false` prevents every implicit status path and config upgrade from
running that generator. Status is computed only from the manifest snapshot and
installed bytes; upgrade leaves the entry unchanged and does not auto-install a
new manual generated file.

`shine app refresh <category> [source] [--force]` is the explicit trigger.
The optional source is the normalized `[[files]].source` manifest identity;
omitting it selects all installed generated files in the category. Refresh:

- operates only on manifest-owned destinations;
- restores a missing managed destination;
- preserves user-modified content unless `--force` is present;
- retains the last-known-good file on generation failure and reports a nonzero
  final result after attempting the remaining selected generators;
- updates the manifest atomically and runs the category's `post_upgrade` hook
  once only when at least one file changes.

Install and reinstall always run an enabled generator regardless of `auto`.
External preset and overlay execution remains gated by `allow_app_hooks`.

## Consequences

The Surge URI subscription generator declares `auto = false`. Users open the
provider window and run
`shine app refresh surge subscription-proxies.conf`; routine status and upgrade
commands do not access the subscription URL. Existing third-party generators
without the new property retain their previous automatic behavior.
