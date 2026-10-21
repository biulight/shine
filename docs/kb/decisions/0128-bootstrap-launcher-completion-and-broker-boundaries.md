# ADR 0128: Bootstrap, launcher, App completion and broker responsibility boundaries

- **Date**: 2026-10-17
- **Status**: Accepted

## Context

Sys bootstrap mixed selection, provider argv, probes and installer classification with batch
orchestration. Launcher operations repeatedly unpacked the same input fields beside platform
templates. App upgrade combined an optional transaction with a relocation flag. SSH broker
authorization and terminal mechanics shared one implementation, obscuring confirmation lifetime.

## Decision

- Retain bootstrap DTOs and batch/item sequencing in the facade. Separate selection, a named
  provider invocation, detection and bounded execution/result classification into private modules.
  Preserve all-selected preflight, state version gating, owning-item notification before elevation,
  authorization before script materialization, post-detection, failure stopping, additive profiles,
  persistence and the existing fresh approved-entry checks. Detection version probes remain
  optional execution during execution-time detection; planning never invokes them.
- Group private launcher arguments in a borrowed input and share field-to-spec conversion.
  Preserve creation's file-name/stem policy and status reconstruction's command-name policy,
  including current native CMD selection and legacy receipt reconstruction. Move pure platform
  templates without changing bytes. Resource expansion, exact checks and whole-command protection
  when any companion is foreign retain their owners and ordering. Public launcher APIs stay stable.
- Represent App upgrade completion with unjournaled, journaled-update and journaled-relocation
  variants. Only the relocation variant suppresses legacy source cleanup. Each journaled variant
  owns the execution capability, including its privilege guard, until immediate receipt persistence
  and commit finish. Preserve error propagation, rollback, outcome/receipt ordering, per-file guard
  release and explicit recovery; do not change transaction resources or introduce a new transaction.
- Introduce a private broker interaction port. Keep mode checks, direct mapping and whole-workspace
  authorization, nonce claims and session serialization at the existing orchestration owner.
  Production captures the same local TTY and SSH PID; a confirmation guard owns restoration until
  decryption or enrollment persistence completes or is cancelled. Direct requests always confirm;
  trusted-session bypass applies only after exact workspace policy matching. Inspect/enroll never
  release secrets. Keep blocking terminal task behavior and unsupported-platform errors unchanged.

## Validation and consequences

Regressions cover bootstrap request order, later-item preflight before any installer/admin prompt,
timeout stopping, denied authorization and provider argv/proxy/limits. Launcher tests freeze nine
pre-refactor template digests across Unix, CMD, PowerShell, live and legacy/current modes; existing
Host/transaction tests retain native resource and foreign-companion protection. App update tests
exercise copy and managed JSON receipt failure/recovery and receipt-before-rollback-cleanup order;
existing relocation suites retain both sides and backup recovery.

Broker regressions exercise direct confirmation under session trust, consumed nonces on rejection,
exact workspace matching before bypass, metadata-only inspect/enroll and confirmed enrollment.
A controlled local crypto backend verifies the confirmation guard remains alive during decryption
and is released on success, failure and cancellation; concurrent requests remain serialized.

No commands, public DTOs, Plan/fingerprint, approval, permission, receipt, journal, trust or wire
formats change. Native Windows/Linux execution, real providers, remote SSH, terminal pinentry and
physical TTY behavior require separate validation; mocked interaction and local backend tests do
not establish those platform/external outcomes.
