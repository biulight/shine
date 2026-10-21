# 0126 — Separate review, validation and code execution responsibilities

- **Status**: Accepted
- **Date**: 2026-10-17
- **Evidence**: `cli/src/lifecycle_plan/render/`, `core/src/runtime/validation/`,
  `core/src/runtime/app/{generator,hooks,artifact}.rs`, `cli/src/ssh/agent/`

## Context

Plan rendering mixed three layouts with permission grouping and code warnings. Static validation
mixed observation-backed source capture with portable path and declaration checks. App code entry
points and the SSH agent combined execution contracts with different failure and trust boundaries.
These responsibilities can move independently without redefining their rules.

## Decision

- Retain the Plan rendering facade and classification owner. Separate compact/detailed/bootstrap
  layouts, permission summaries, code boundaries and labels. Preserve bootstrap's distinct labels
  and ordered grouping; share only existing common rules. Rendering never creates approval.
- Retain validation's public V1 DTOs and report orchestrator. Move source capture, declaration
  checks, portable destinations and snapshot-only Sys checks into private modules. Keep permission
  errors before destination checks, then macOS/Linux/Windows metadata validation with the original
  first-error behavior. Repository/category/manifest inputs, traversal, `node_modules` filtering,
  snapshot executable flags and observation-only Host bounds remain unchanged.
- Move App generator, hook and artifact methods into separate use-case modules. Keep the common
  snapshot script seam and scoped trust checks. Do not unify generator activation/manual policy,
  30-second/8 MiB stdout/64 KiB stderr limits and redaction with hook non-fatal failures or artifact
  implicit/explicit I/O and error behavior. Existing lifecycle locks and fresh approval gates stay
  at their callers. Script entries retain their mappings plus the existing fixed contract;
  command hooks receive only their declared mappings in the explicit process environment.
- Keep the SSH listener and tracked tasks in `agent.rs`. Separate handshake/token/broker routing,
  transfer policy, argv construction and relay supervision. Broker allow-lists, policy matching,
  nonce claims and local confirmation remain in the broker. Wire values stay untrusted operands;
  only captured local context supplies reconnect options. Preserve fallback/probe/preview order,
  secret response zeroization, child termination/reaping and Unix process-group cancellation.

## Validation

Regressions exercise full-review fallback for incomplete/ambiguous bootstrap attribution, visible
code/privilege/unknown blockers across layouts, read-only validation entry points and diagnostic
precedence, App process mappings/limits/trust/no-effect gates and distinct failure contracts,
command-versus-script hook environments and failed-invocation cleanup, and
SSH handshake/token/broker routing with nonce replay rejection. Existing consumer suites retain
snapshot cleanup, malicious argv, IPv6 fallback, real local tool stubs and process-group cleanup.

Only unit-test builds record complete in-memory process requests for assertions; production Host
operation logs and serialized contracts do not gain environment values. No command, wire/report
schema, approval, receipt, journal or trust format changes. Native Windows/Linux and real remote
hosts require separate platform validation.
