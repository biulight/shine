# 0171 — Bootstrap review uses a numbered table

- **Status**: Accepted
- **Date**: 2026-11-16
- **Extends**: [ADR 0159](0159-concise-default-code-review.md), [ADR 0094](0094-security-plan-filesystem-review-summaries.md)
- **Evidence**: `cli/src/lifecycle_plan/render/bootstrap.rs`, `cli/src/lifecycle_plan/render/bootstrap_tests.rs`, `cli/src/lifecycle_plan.rs`, `tests/cli_dispatch.rs`

## Decision

Default ready bootstrap review uses one numbered row per item, in execution order, with Installation,
Commands and Execute columns. Installation shows `run` for Execute and `skip` for Update: the latter
means detection succeeded, not a software upgrade. Other actions retain explicit status labels.
Detected items can still contribute detection commands, shell integration and saved state changes.
Columns use escaped display widths and wrap without truncation on narrow
terminals. A dash means no explicit permission in that column, not absence of code execution.
Inline permission values containing commas, quotes, surrounding whitespace or a literal dash
marker are quoted and escaped so data cannot be mistaken for a list separator or an empty cell.

Administrator and unrestricted-network requirements move to Attention with every owning scope
named. Package-conflict removal and unsupported profile recovery are also shown there once per
target. Other permissions and unknown diagnostics remain attributed and visible. Profile and shared
filesystem summaries keep the existing conservative provenance rules and explicit user paths.

Distribution-trusted, selected bootstrap/profile code groups by entry kind, timing and bootstrap
action. Only Execute steps enter the installation execution group; Update steps retain a separate
“installation skipped; trust requirements retained” group. The summary
says “all” only when its exact target set equals the bootstrap item set. External trust and shared
affected targets remain individual entries. Shared profile resource identities do not by themselves
make selected built-in code exceptional; the typed affected role does.

The same interpretation applies to individual code boundaries in verbose and fallback reviews,
including external trust and operation-only consent. A uniquely matched SysBootstrap step with
resource `bootstrap` is required; ambiguous or unrelated steps keep the original boundary wording.
Blocked installation is described as blocked, without claiming detection succeeded. The existing
Core boundary records remain unchanged: script preflight and trust checks still precede execution's
presence check, so removing an installed item's boundary could also remove required consent.

Verbose and blocked review retain expanded output. Unsatisfied scopes, duplicate or unexpected
steps, or profiles interleaved with bootstrap items cannot use the table. Missing or ambiguous
permission attribution falls back to the full review. Other lifecycle layouts are unchanged.

This is a terminal projection only: Plan data, fingerprints, trust, confirmation, fresh-state checks,
execution and recovery remain unchanged. The public system-init guide already covers reviewing
permissions/recovery and using `--verbose`; no additional user procedure is required.

## Validation

Assert the complete embedded Ubuntu recommended layout and inspect macOS/Windows embedded plans.
Also cover all-detected and mixed Ubuntu selections, retained detection commands, skipped external
scripts with missing trust, operation-only consent and changed detection after approval.
Cover exceptional trust, timing, scope ownership, extra permissions, unknown diagnostics, wrapping,
control escaping and conservative fallbacks. Rendering must preserve serialized Plan data and
fingerprints. An isolated real CLI process checks colored, redirected and `NO_COLOR` review while
closed stdin prevents approval and leaves bootstrap state and shell configuration untouched.
