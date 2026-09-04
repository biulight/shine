# 0079 — Journal state and progress have explicit safe projections

- **Status**: accepted
- **Scope**: Roadmap Phase 6C

## Decision

Each Core domain owns journal decoding, version validation and progress extraction. Its operation
inspection uses the same captured journal bytes as its existing pure recovery planner. The frontend
service projects only an opaque operation identity, recorded action counts and the existing recovery
Plan. Neither stored approval, Action IR paths, receipt contents nor rollback material crosses the
stable boundary. Unsupported or corrupt journals return a safe failure and remain untouched.

Recorded progress describes durable evidence, not a live worker or an exact account of completed
OS effects. A crash may occur between an effect and its journal update. Recovery readiness is
therefore determined by Core's receipt, positive-marker and live-fingerprint assessment, never by
counting applied actions in an adapter. Observation does not execute recovery, and every later
recovery execution requires a freshly reviewed exact Plan.

Stable events are a new projection, not serialization of `RuntimeEvent`. They carry an explicit
event kind, execution-local sequence, optional reviewed canonical target and typed status. Raw
codes, errors, output, labels, source paths, argv and environment values are not copied. Targets
must match the generated Plan's canonical target allow-list. A future runtime event variant requires
an explicit projection decision. Event sequence is not a persistent replay cursor.

A projected observer may fan out safe events to the frontend while preserving unchanged raw events
for trusted local CLI rendering. AI-facing read-only adapters never receive the raw observer or
local detail channel. Journal observation and events add no execution authority.

## Verification

Fixtures exercise idle, interrupted, committed and conflicting journals in all domains, including
unsupported versions and preserved user changes. Event tests use adversarial paths, labels, process
output and diagnostic strings and verify both redaction and unchanged local delivery. Existing
recovery tests continue to prove exact-resource and owned-subset safety.
