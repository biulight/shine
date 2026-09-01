# 0049 — Interrupted App operations use an explicit recovery command

- **Status**: Accepted
- **Date**: 2026-08-30
- **Evidence**: `cli/src/commands/app.rs`, `cli/src/apps/recovery.rs`,
  `cli/src/lifecycle_plan.rs`, `core/src/runtime/action_executor.rs`

## Context

ADR 0048 introduced a durable App operation journal and a specialized `app-recovery` Plan. Real
App installation can leave that journal after interruption or a manifest-receipt write failure,
and every ordinary App lifecycle Plan then blocks with `app_recovery_required`. Core could safely
plan and apply recovery, but the CLI had no route to those APIs, leaving users unable to resolve a
state that released lifecycle code could create.

Recovery cannot be folded into install, update, upgrade, or uninstall: implicit cleanup would
mutate state without presenting a recovery-specific snapshot and could delete a transaction-created
file after the user changed it. A successful cleanup-only recovery also removes the journal, so its
Plan must display that mutation and require approval even when the matching App receipt is already
durable.

## Decision

Expose recovery as the advanced App command `shine app recover [--yes]`. It always builds and
renders the specialized `app-recovery` Plan. Any ready recovery Plan includes an explicit Remove
step for `app/operation-journal`, asks once with a default answer of No, and is regenerated and
validated before apply. `--yes` skips only that prompt; there is no separate dry-run because the
unapproved Plan is already a non-mutating preview.

Recovery remains rollback-only for the supported action slice. A matching durable receipt preserves
the managed destination and removes only the stale journal. Without a receipt, Core removes a
transaction-created destination only while its current hash still matches the journal. User-modified
content, opaque actions, unsupported schemas, invalid journals, and missing journals return a
non-zero error without mutation. A blocked recovery Plan explicitly preserves the journal.

Ordinary App lifecycle Plans that find a journal remain blocked and direct the user to
`shine app recover`; they never invoke it. The command is exempt from the background release check
so a patch-version gate cannot make the recovery path unavailable. Successful CLI output reports
only whether the journal was cleared and how many interrupted file changes were rolled back; it
does not print physical paths, journal contents, or Action IR identities.

## Consequences

- A durable state produced by the App action executor now has a supported user-facing resolution.
- Every recovery mutation uses the same snapshot-bound review, default-No approval, revalidation,
  and cross-process lock as the Core contract.
- Recovery stays intentionally separate from lifecycle retry, resume, force deletion, and manual
  journal editing.
- Future Action IR kinds must extend Core recovery semantics before this command can recover them;
  the CLI does not implement action-specific rollback logic.
- The command and recovery guidance are part of the bilingual public manual contract.
