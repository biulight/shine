# 0080 — Trusted execution consumes a local reviewed request

- **Status**: accepted
- **Scope**: Roadmap Phase 6D

## Decision

The shared frontend service captures resolved runtime context and the effective Preset snapshot.
Distributions still supply configuration, embedded bytes and local human interaction. They do not
reimplement approval matching or lifecycle dispatch.

A trusted frontend produces a local human review containing the exact request, Plan and opaque
configuration revision. After an affirmative human action it can consume that review to obtain an
`ApprovedOperation`. The handoff is neither cloneable nor serializable, has private fields, and is
consumed by execution. Execution compares the current configuration revision, regenerates the Plan
from the retained request, validates exact approval and delegates to existing approved Core methods.
Those methods retain their final freshness checks before effects. Frontends must capture fresh
runtime inputs before applying a reviewed operation, including when preparing a batch.

AI-facing adapters receive only `ReadOnlyFrontend`, which exposes safe inventory, default
inspection, Plan requests and journal observation. It has no runtime accessor, generator evaluation,
human-review constructor or mutation method. Even its errors contain only safe diagnostics. This
is a capability boundary for adapters, not a sandbox against arbitrary trusted Rust code in-process.

Execution reports reuse normal `LifecycleResultV1`; refresh, artifact, bootstrap, profile and recovery
keep explicit specialized operation identities. Rich domain reports and raw observers remain local
and non-serializable. Safe progress is emitted for every operation, including domains without raw
events. A completed progress event means the call returned; callers must inspect result outcomes
for per-item failures. Dispatch failures emit a failed progress event and a safe diagnostic;
validation rejection happens before execution events or effects.

## Verification

Conformance must cover shared capture, read-only report equivalence, exact reviewed request reuse,
configuration/source/state/permission invalidation, ordinary and specialized mutation, journal
recovery, and redacted execution events/results. Compile-fail examples enforce the restricted
facade and non-cloneable, non-serializable one-shot handoff. CLI compatibility remains a required
gate; no new MCP server or GUI is shipped by this decision.
