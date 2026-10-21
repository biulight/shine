# ADR 0123: Typed CLI upgrade handoffs and production launcher tests

- **Date**: 2026-10-17
- **Status**: Accepted

## Context

Aggregate upgrade assigned generic prepared Plans to Shell/App/Sys adapters by vector position.
Adapters executed a generic operation and assumed the result variant afterward. Launcher tests
also maintained a separate filesystem implementation for create/status/unlink, so passing tests
could leave the production Host behavior unexercised.

## Decision

- Keep reviewed request/approval fields private to lifecycle planning; expose only borrowed
  permissions for the existing App recovery administrator check.
- Validate the complete three-domain Upgrade batch before capturing any runtime. Preserve
  Shell/App/Sys review, prevalidation and execution order. Return named, consuming domain handoffs;
  all three preparations must succeed before execution begins.
- Use the same typed preparation for single-target App/Shell upgrade. Keep managed Sys install,
  upgrade and removal on their existing shared adapter, with a stricter Sys Upgrade wrapper at
  the aggregate boundary. Each handoff returns the existing typed domain report.
- Retain existing configuration revisions, operation code grants, default-No/automation rules,
  lifecycle locks and Core fresh Plan/permission revalidation. Do not change serialized contracts,
  report formatting, cache/preparation timing or introduce cross-domain transactions.
- Move launcher tests to `launcher/tests.rs`, preserving names, assertions and platform guards.
  Exercise production Host APIs and prepared transaction resources; remove the independent
  test-only create/status/unlink implementations. Test fixtures may construct specs and seed state,
  but cannot substitute their own ownership or mutation logic for the production implementation.

## Consequences

Commands, Plans/fingerprints, receipts, journals and valid-operation output retain their behavior.
Wrong-domain/malformed internal handoffs fail before capture instead of reaching a later result
assumption. Prepared values remain process-local and one-shot; preparation is not execution authority.

Regressions cover incomplete/extra/duplicate/misordered batches, non-Upgrade and wrong singleton
requests before capture, later-domain prevalidation failure without earlier effects, typed report
results and state changes after preparation. Launcher coverage retains native links, conflicts,
Bun idempotency/dependency/env behavior and removal protection, plus Live config identity and
whole-command preservation when any launcher resource is foreign (including Windows companions).
Native Windows shim execution remains platform-gated; virtual Host tests do not prove OS execution.
