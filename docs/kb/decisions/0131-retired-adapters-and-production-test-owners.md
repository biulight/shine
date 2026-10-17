# ADR 0131: Remove retired adapters and test production owners

- **Date**: 2026-10-17
- **Status**: Accepted

## Decision

Delete the detached CLI Shell template adapter and the unused App materialization/generator/hash
chain left after status-test migration. Replace the test-only CLI Bun resolver with snapshot-based
tests of Core category parsing and dependency policy. Retain the original four scenarios and check
both input hashes, typed policy errors and overlay source-layer isolation. Core managed JSON tests
already cover the old CLI tests' unmanaged-key and absent-key cases; remove that duplicate parser.

Remove uncalled CLI annotation parsers, target loader and color helpers, and the internal Core
runtime's uncalled host-extraction, manifest-update/removal and profile-sync wrappers. Keep the
manifest implementation used by lifecycle deployment. Restrict used Bun-policy and Sys-profile
helpers to `crate::runtime`; the runtime module explicitly has no stable third-party API contract.
Keep SSH context deserialization as a test-only round-trip helper and remove the unused CLI
interaction administrator method; the production Core `RuntimeInteraction` adapter remains intact.

## Boundaries and validation

Commands, V1 reports, Plan/permission checks, lifecycle locks, receipt/journal schemas, legacy
launcher reconstruction and recovery dispatch remain unchanged. This is internal cleanup, with no
public manual behavior change. Source membership and caller searches complement compiler checks:
unmounted files and unused public methods can otherwise pass them.

Establish migrated Bun tests against the existing Core implementation before deleting the CLI
copy. Run Rust workspace tests and lint checks after removal, retaining the existing approval,
transaction, recovery, platform and real-process regressions.
