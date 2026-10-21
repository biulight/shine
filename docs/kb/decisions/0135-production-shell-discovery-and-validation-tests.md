# ADR 0135: Test production Shell discovery and validation reports

- **Date**: 2026-10-17
- **Status**: Accepted

## Decision

Move Shell description regressions from CLI parsing copies to `CoreRuntime::shell_categories`.
Fixtures supply native/Bun sources and metadata; production parsing produces every description.
Preserve current whitespace, annotation, invalid-UTF-8 and declared-description precedence rather
than restoring the old parser's narrower rules.

Move embedded discovery tests to the pristine embedded runtime snapshot. External discovery tests
capture real temporary trees through `capture_preset_snapshot`, then query Core metadata. Preserve
legacy native discovery, descriptions, sorted categories/commands, nested source paths and missing
root behavior. A missing selected external category remains an error. Exercise declared native
platform alternatives and Bun sources for all three runtime platforms; undeclared files stay out.
Remove the three old CLI scanners, their two test-only DTOs, and the description/annotation helpers.
Consolidate the two missing-root tests once they exercise the same production path.

Build validation text and JSON golden reports through the production CLI `validate_path` adapter
and Core validator. Assert actual canonical paths before normalizing only those temporary paths
for the JSON golden. Add mixed invalid/legacy categories to verify error and warning aggregation.
Remove the CLI test copy of Core's report aggregation; rendering and the V1 report remain unchanged.

Make live Shell target validation, Sys manifest validation and split-DNS host removal private to
their owning modules, removing unused public re-exports. Restrict Shell manifest category/target
updates, Sys shell-name conversion and the not-directory error predicate to `crate::runtime`.
Keep their bodies and lifecycle callers, including compatibility removal and manifest updates.
Retain schema compatibility aliases and profile installation/removal entry points.

## Boundaries and validation

The migrated tests pass before deleting the obsolete implementations. Run workspace regressions,
approval/read-only capability doctests, formatting, lint and dependency checks after cleanup.
This changes internal ownership and test coverage only: production discovery, validation, Plans,
approval, locks, receipts, journals and recovery behavior stay unchanged. The platform metadata
matrix is deterministic coverage on the local host; it does not establish native Windows/Linux
execution or real administrator/provider behavior.
