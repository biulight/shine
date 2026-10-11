# 0108 — Scope maintenance advisories and defer heavy results to later invocations

- **Status**: Accepted
- **Date**: 2026-10-11
- **Evidence**: `cli/src/maintenance/`, `cli/src/legacy_serve.rs`, `tests/runtime_boundaries.rs`

## Context

Removing the HTTP service leaves user-owned native registrations that require an explicit action
after upgrading (ADR 0107). Repeating expensive checks before every command adds latency; ordinary
Tokio tasks cannot finish after the CLI exits. Informational migration reminders must not become
new authorization or release gates.

## Decision

- Register typed rules with stable IDs, revisions, a manual action, one of four frequencies
  (every invocation, minute, hour, day), and inline/background execution. Reminder cooldown is
  independent and currently one day per action. Newly revised rules bypass old schedule/results.
- First integrate legacy HTTP registration inspection inline on every eligible invocation, and
  native service-manager inspection hourly in a short-lived detached worker. Only results already
  present at invocation start may produce background reminders; no constant daemon is installed.
- Cache per absolute config root/effective user home/native config root/platform, plus build and
  rule revision. Store only typed findings/timestamps/tickets, not native output or secret values.
  Atomic private writes and persistent OS locks protect concurrent state updates. Use pending
  leases, probe/worker timeouts and failure backoff; worker failure never blocks ordinary commands.
- Workers use the current executable with a hidden, config-independent route and null stdio.
  Detach on Unix/Windows and reap when the parent remains alive. Only registered read-only probes
  run; do not resolve or execute arbitrary commands/Presets from a serialized cache.
- `shine state check` explicitly runs all current rules synchronously and repeats findings even
  when stderr is redirected or configuration is unreadable. Automatic checks require terminal
  stderr and skip automation, authoring, shell/theme startup, maintenance and recovery paths.
  If another check or cleanup supersedes explicit inspection, report a retry warning and exit
  nonzero without a completion summary; automatic advisories may silently discard obsolete results.
- Successful `clear serve` advances the selected config scope's cache generation. Publishing
  requires a matching generation, rule revision and ticket, so in-flight old results are discarded.
  Separate config scopes keep separate reminder histories; actual service removal remains per user.
- Retain the existing runtime-schema warnings and patch-version upgrade gate unchanged. Safety,
  trust, permission and snapshot-bound approval checks remain synchronous at affected operations.

## Consequences

Heavy findings can be delayed until another eligible invocation. Users needing immediate feedback
use `state check`; no eligible invocation means no scheduled work. A failed native inspection is
incomplete, never proof that a service is absent. Advisory cache failure degrades to no automatic
reminder, while explicit inspection surfaces the error. Native Windows detachment/service behavior
still requires native smoke validation; macOS process-lifecycle regressions use isolated PTYs and
mock service managers, never the developer's real registration.
