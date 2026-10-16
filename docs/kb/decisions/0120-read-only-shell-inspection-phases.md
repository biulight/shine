# ADR 0120: Separate read-only Shell inspection phases

- **Date**: 2026-10-17
- **Status**: Accepted

## Context

`inspect_shells` mixed category/manifest iteration, source and launcher observations, deployment
change explanations, status precedence and receipt-only row construction. Moving one condition
could change error order or make a conflict look like an ordinary update. Its generic Host bound
also exposed mutation and privilege capabilities although inspection used neither.

## Decision

- Move inspection to `shell/inspect.rs`. Keep orchestration responsible for effective categories,
  one manifest load, one snapshot comparison per category and appending receipt-only rows.
- Bind each category/file/receipt/snapshot result in a private borrowed target. Observe desired
  delivery, source and launcher state in the original order; preserve Bun policy/trust/probe
  assessment points and source read error contexts.
- Project the ordered change list and status separately from those observations without Host
  access. Keep all existing statuses, text, duplicate/content explanations and rebuild suppression.
  Conflict precedes not-installed, trust, pending update, missing rendered output and live/current.
- Require only `FileSystemObservationHost` for inspection and snapshot comparison. Retain the shared
  planner ownership probe and content/executable snapshot comparison. Snapshot comparison failures
  still become non-current during inspection. Receipt-only rows are never executable metadata.

## Consequences

Public report fields, commands, metadata/receipt schemas, Plan/approval and execution flows remain
unchanged. No source is reopened outside the existing observation flow, no observation is cached
across assessments, and no new ownership or trust proof is introduced.

Consumer tests use a Host with observation ports only. They cover complete change/row ordering,
receipt-only conflicts and preservation, Live trust versus launcher conflict/missing launcher,
missing rendered output versus launcher repair, exact source read errors and snapshot observation
failure remaining an update. Existing CLI/frontend/planner/lifecycle regressions continue to cover
projection, platform launcher sets, retained missing siblings and approval boundaries.
