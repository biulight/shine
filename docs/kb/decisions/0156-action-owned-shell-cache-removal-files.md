# 0156 — Let the Shell cache-removal Action own its file list

- **Status**: Accepted
- **Date**: 2026-10-29
- **Extends**: [ADR 0117](0117-paired-shell-preparation-and-app-assessment-states.md)

## Context

Cache removal prepared an Action file list containing destination, rollback and previous identity,
plus a second list containing only the same paths. Apply zipped the lists to recover their pairing.
Unlike profile or cache replacement, removal has no private byte payload to retain separately.

## Decision

Keep `PreparedShellAction::RemoveCache` as a unit variant identifying the prepared phase. Build
only the existing `ShellCacheFileRemovalV1` list after the same rollback, live-file and receipt
checks. Apply requires the matching Action variant and iterates its file contracts directly,
reobserving each destination before moving it to its recorded rollback path.

Action validation, exact permissions, journal-before-effect ordering, locks, progress markers and
receipt boundaries remain unchanged. No persisted type, schema, public API, error text or resource
order changes. Recovery, directory cleanup and cache replacement remain outside this change.

## Consequences

One duplicate vector, its path clones and a positional zip disappear without adding a type or
helper. The Action already owns all removal inputs; retaining another partial record adds no
execution capability. A regression interrupts either file move in a two-file cache, verifies the
journal and partial state, rejects changed content and stale recovery approval, then restores and
retries without touching an unrelated category. Existing sibling and profile regressions remain.
