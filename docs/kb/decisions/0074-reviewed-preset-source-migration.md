# 0074 — Migrate Preset sources through an explicit reviewed workflow

- **Status**: accepted
- **Evidence**: `core/src/runtime/preset_migration.rs`, `cli/src/preset_migration.rs`

## Context

Shine 2 introduces versioned App metadata, target-local permission declarations, declarative Sys
items, and stricter external-code trust. A 1.x external source or overlay can therefore fail before
an install or upgrade Plan is useful. Treating those failures as an empty update set hid the real
problem, while automatically rewriting source during `update`, `upgrade`, or `self upgrade` would
mix authoring changes with runtime mutation and could grant capabilities the user never reviewed.

Runtime state migration cannot solve this problem: `shine state migrate` owns Shine manifests,
receipts, and caches, whereas Preset metadata belongs to the user or its upstream repository.

## Decision

`shine preset migrate [PATH]` is the only automatic Preset-source migration entry point. Without a
path it uses read-only config discovery to inspect the active external source and overlay; with a
path it accepts a repository, category, or `shine.toml`. The command captures one immutable
effective snapshot, creates candidate metadata with `toml_edit`, validates candidates against that
same logical scope, renders every diff, and defaults confirmation to No. `--dry-run` performs no
state or source writes. JSON is a versioned, content-free report and may apply only with `--yes`.

Automatic edits are limited to changes that can be proved safe:

- exact released 1.x built-in metadata can rebase to current embedded metadata only when relevant
  executable identity still matches;
- a legacy App can receive metadata schema v2, lose only an exact same-category recursive artifact
  hook, and receive an empty permission table only when it contains no opaque code;
- an overlay metadata override can be removed only when the revealed lower layer is already the
  validated current metadata.

The migration never changes payloads, scripts, environment values, runtime manifests, or trust
grants. It never guesses Shell/App/Sys opaque-code permissions or broad network access and never
splits a Sys v1 dispatcher. Those cases receive stable blockers plus target-local validation,
planning, trust, or Sys-v2 authoring instructions. A managed Git overlay is diagnosed but remains
read-only; its upstream checkout must be migrated explicitly.

Before apply, the CLI rechecks every source hash and creates a complete owner-only backup set under
the Shine private state directory, including a manifest with logical identity, hash, mode, and
source layer. Only after every backup succeeds does it atomically replace or remove individual
metadata files. An interruption may leave completed files and the backup set in place; rerunning
converges on the remaining work. Any blocker, source change, backup/write failure, or refusal makes
the command fail even if independent safe edits completed.

`shine update` and `shine upgrade` run the same compatibility assessment. Update still completes
the executable configuration and release checks before returning a blocker. Upgrade, including the
post-pull state, stops before lifecycle Plan construction or mutation. Neither command invokes the
migrator implicitly.

## Consequences

- Users see the active logical target and source layer instead of an empty or misleading update.
- Review, backup, source revalidation, and candidate validation form one source-mutation boundary.
- Runtime state and external-code trust remain separate, explicit workflows.
- Some legacy sources require manual authoring work; safety takes precedence over broad automatic
  conversion.
