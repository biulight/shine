# ADR 0132: Test production App destinations and preserve Shell repair targets

- **Date**: 2026-10-17
- **Status**: Accepted

## Decision

Move the six legacy App annotation/fallback cases to snapshot-backed Core metadata parsing and
`app_destination`. Exercise effective destination collisions through `validate_app_category_snapshot`
with distinct relative targets that resolve to one path. Embedded CLI destination tests use Core
with an explicit captured data directory; installed Vim/Ghostty fixtures use
`from_installed_presets`, retaining the original installed-source selection. Distinct source bytes
prove these tests read their disk fixtures rather than silently falling back to embedded presets.
Delete the CLI-only destination, data-directory and collision implementations after migration.

Migration exposed a production gap: legacy `shine-dest` returned before the common parent-component
check. Resolve the full annotation path through that check while retaining annotation precedence,
tilde/absolute paths and nested fallback targets. A planning regression also rejects the invalid
legacy destination. This change does not redefine Windows destination case comparison.

For Shell conflict guidance, try each source root independently; failure to match presets must not
skip rendered scripts. Preserve explicit category hints and generic fallback for unknown sources.
The presentation regression covers both roots without requiring filesystem mutation.

Remove the uncalled `ShellManifest::remove_category` leaf. Restrict the five snapshot-validation,
snapshot-materialization/removal and template-rendering helpers to `crate::runtime`; their lifecycle
and compatibility callers keep the same implementations and effect ordering.

## Boundaries and verification

Keep legal legacy App annotations, Shell delivery variants, snapshot-bound approval, lifecycle locks,
manifest/receipt/journal formats and recovery dispatch. Only invalid legacy parent destinations and
rendered-source repair guidance change behavior. Existing Shell profile test adapters already call
Core and do not contain duplicate business rules.

Establish failing regressions before the two fixes, then pass migrated tests before deleting copies.
Run workspace tests, approval/capability doctests, Rust lint checks and bilingual documentation checks.
Native Windows/Linux behavior and real external providers remain separate validation boundaries.
