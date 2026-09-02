# 0072 — Shine 2.0 is the lifecycle security and recovery boundary

- **Status**: accepted
- **Evidence**: `core/src/{plan,action,trust}.rs`, `core/src/runtime/`, `cli/src/lifecycle_plan.rs`

## Context

Since 1.8, protected lifecycle mutations gained snapshot-bound default-No Plans, target-scoped
external-code trust, explicit recovery journals, and a reusable `shine-core` runtime. Automation
must add `--yes`, broad `allow_app_hooks`/`allow_sys_code` flags no longer authorize code,
generator inspection is explicit, and global upgrade no longer changes Sys profile activation.
These are intentional public-contract breaks rather than a backward-compatible feature increment.

## Decision

Ship the first reproducible candidate as `2.0.0-rc.1`. Keep 1.8 as the latest stable release and
the default manual while the candidate is evaluated. A compatible 1.8 runtime state must remain
directly inspectable, repairable, upgradeable, and uninstallable; broad grants are the exception
and are never migrated because doing so would silently expand authority.

The moving `preview` channel continues independently. Versioned release candidates are GitHub and
Cargo prereleases, do not become `latest`, and do not trigger stable branch synchronization.

## Consequences

- The public manual keeps one frozen bilingual 1.8 snapshot and publishes candidate documentation
  as `next` until 2.0 becomes stable.
- Legacy manifests normalize in memory and write the current schema only after a successful
  mutation. Compatible legacy Shell launchers remain activation evidence even without a receipt.
- Future version decisions follow Semantic Versioning across the CLI, configuration, automation,
  and published `shine-core` contracts.
