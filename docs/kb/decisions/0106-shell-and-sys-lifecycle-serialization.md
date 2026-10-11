# ADR 0106: Serialize Shell and Sys lifecycle state before approval revalidation

Date: 2026-10-09
Status: Accepted

## Context

Shell uninstall and managed Sys operations loaded entire manifests before obtaining the privileged
transaction lock. Two operations could approve unchanged state, wait for that lock, then save stale
manifests that discarded another operation's receipts. Locking only resource mutation was too late.

## Decision

- Extend ADR 0105's scoped lifecycle locking to `<shine_dir>/shell-lifecycle.lock` and
  `<shine_dir>/sys-lifecycle.lock`, acquired before fresh planning and approval validation and held
  through effects, receipt persistence, and transaction cleanup.
- Shell install/uninstall/upgrade, recovery, completion updates, live rendering, and standalone
  receipt mutations share the Shell lock. Internal receipt updates reuse the outer guard.
- Managed Sys, bootstrap, profile enable/disable, recovery, profile synchronization, and standalone
  managed-file receipt operations share the Sys lock because they use one manifest.
- Always acquire the domain lock before the privileged transaction lock. Planning and dry-run
  entry points remain observation-only. Waiting does not authorize changed state.
- Model the privileged lock with an async mutex in InMemoryHost as well as scoped locks. Regression
  tests suspend operations on an explicitly held guard instead of depending on sleeps or scheduling.

## Consequences

Unrelated Shell changes retain their receipts even when their reviewed Plan remains valid. Sys
approval binds complete manifest observations, so a waiting operation can require a new reviewed
Plan after another operation succeeds. A long bootstrap or rendering operation can cause a waiter
to reach the existing 30-second timeout. Domain locks coordinate cooperating Shine operations;
external edits still require the existing ownership and snapshot validation.
