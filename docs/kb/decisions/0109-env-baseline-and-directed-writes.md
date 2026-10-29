# 0109 — Env defaults are runtime fallbacks; writes target explicit layers

- **Status**: accepted
- **Evidence**: `cli/src/config/env_resolution.rs`, `env_write.rs`, `save.rs`,
  `cli/src/env/commands.rs`, `tests/env_configuration.rs`

## Context

Overlay priority historically let maintaining devices override stale consumer config (July 15
lesson). Source-directed `--force` writes could modify disposable Git mirrors. Persisting defaults
also obscured user intent and prevented new defaults from taking effect.

## Decision

Use one breaking precedence order: builtin < overlay < global config < global env file < project
config < project env file. Overlays now provide a shared baseline. Preserve every historical
explicit entry; never infer whether an old value was generated. No automatic env migration,
compatibility policy, default cleanup, or version bump accompanies implementation.

Keep raw layers, effective values, independent value/description sources, and explicit pending
edits. Config persistence applies changes to the raw active file; inherited defaults and values
never materialize during unrelated saves. An explicit equal-value set still writes an entry.

Scope and file options choose targets independently of effective sources. Implicit writes refuse
shadowed keys; explicit targets may maintain shadowed entries and report saving separately from
effectiveness. Delete requires a target-local entry and restores inheritance. Retired `--force`
fails without writing. Its diagnostic identifies the retired target from raw override definitions
(project env file > overlay > global env file, otherwise the active config), even when the new
effective source differs; this is not a second runtime read policy. Git mirrors remain disposable and reject env mutations (ADR 0010).

Preflight captures input bytes/existence and context before encryption. Commit takes a persistent
OS file lock named `.<target-name>.shine.lock` in the canonical target parent, verifies the selected
inputs, then replaces privately and atomically. Config saves, proxy config changes and recipient
config migration use the same lock. Locks are never deleted on release. This serializes Shine's
final check/replacement per target; it is not a transaction across editors, Git, or multiple layers.
After saving, reload the current context; failures explicitly report that saving already occurred.

`env list/get/explain` and directed writes route before initialization and advisory work. Explain
hides all values unless explicitly revealed and never decrypts. Paths stay in local diagnostics,
outside safe Frontend/Plan DTOs. Encrypted-first injection and lifecycle approval remain unchanged.

## Consequences

Users review each important project and manually remove unwanted local entries to inherit overlay
or new builtin defaults. Update scripts to explicit scope/file options. Installed templates change
only through the existing reviewed lifecycle. Downgrading restores old ordering but cannot restore
deleted configuration or reverse installed-artifact changes. Public manuals include both procedures.
The [Windows validation and downgrade drill](../operations/env-configuration-validation.md) retains
replacement-failure, concurrent-write and stable v2.3.1 downgrade scenarios and their limits.
Old binaries can resume builtin-default expansion, but cannot recover deleted custom values.
