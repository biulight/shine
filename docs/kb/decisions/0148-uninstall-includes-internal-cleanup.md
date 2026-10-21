# ADR 0148: CLI uninstall includes internal cleanup

- **Date**: 2026-10-19
- **Status**: Accepted

## Decision

Remove the CLI `--purge` option instead of retaining a hidden compatibility parameter. Top-level,
App and Shell CLI handlers always request the existing Core cleanup behavior (`purge: true`),
including dry-run. Existing invocations containing `--purge` fail argument parsing before config
initialization; callers should omit the option.

App category removal includes the entire internal category cache, including extra files, and keeps
the manifest file. Whole-App removal includes the entire internal App cache; its manifest is deleted
only when no receipts remain. Protected and failed removals retain receipts for retry. Targeted
Shell removal preserves extra cache files and sibling dependencies; whole-Shell removal includes
extra internal cache files. Empty managed directory cleanup is included. External sources remain
preserved. This supersedes ADR 0136's distinction between ordinary CLI removal and explicit purge.

## Boundaries and validation

Keep Core request fields, fingerprints, observations, approval, locks, receipts, execution and
recovery machinery unchanged. Cleanup must be part of the original reviewed Plan, never a second
post-approval deletion pass. Retain protected-resource and exit-status policies.

Real CLI regressions cover default cleanup through all three entry points, target and whole scopes,
external source retention, preview/denied-approval non-mutation, removal of the old option, help and
completion candidates. Existing Core tests retain both explicit cleanup policies for API callers.

CLI handlers and the top-level shim expose no cleanup boolean. Previews project receipt removals
in memory only; cleanup messages derive from lifecycle effects, not from the request flag.
