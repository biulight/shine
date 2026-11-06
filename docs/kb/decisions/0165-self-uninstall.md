# 0165 — Self uninstall with retained personal data

- **Status**: Accepted
- **Extends**: ADR 0148 and ADR 0158

## Decision

Coordinate App, Shell, Sys and proxy uninstall through their existing owners. Keep domain recovery
journals and protected files available for retry. Independent domain actions can complete before
another domain stops. Final collection requires an empty managed inventory, empty secret receipts
and a fresh comparison of the approved files under the lifecycle, trust, broker and config locks.
Keep persistent lock files so concurrent processes continue to lock the same inode. Preserve
unrecognized files and personal data. Remove exact generated Sys startup blocks only for the
default account configuration. A whole-Shell uninstall verifies that the startup block loads
the selected configuration's profile. Shared markers alone also match other installations.
The legacy service uses its dedicated cleanup routine.

Record each published self-install copy's path, canonical path, native identity and SHA-256 digest.
Removing a program requires both a matching receipt and matching running-build contents. A
configured destination or a PATH match alone cannot establish ownership. Cargo and older
installations receive a channel handoff. Windows requires another running copy so the OS can
release the target executable. A failed cleanup can be retried with a surviving copy.

## Consequences

A successful uninstall can leave user configuration, locks, external sources and bootstrap
software on disk. Downgrade compatibility depends separately on the retained configuration and
preset metadata supported by the version being installed.

On Windows, stage startup and config edits with the original owner, group and access rules.
Native replacement merges alternate streams and the DACL. When a Shell transaction has moved
the original into rollback, merge against a temporary link to it, then publish without overwriting
an intervening file. Keep the rollback object usable until the transaction commits.
