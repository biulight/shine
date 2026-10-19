# ADR 0149: Bind remaining Shell integration to an empty manifest

- **Date**: 2026-10-19
- **Status**: Accepted

Full Shell uninstall can find managed integration after category uninstall has removed every
command receipt. Do not invent a launcher receipt, allow an unbound profile action, or remove the
integration outside the reviewed transaction.

Reserve `shell/profile` in `ReconcileShellProfile.legacy_targets` for this observed profile-only
removal. It must be the sole boundary, with no receipt transitions/removals. Whole-file actions
must remove an identified existing managed profile; sentinel actions must remove an identified
existing owned block. The existing action representation and journal schema are unchanged.

Preparation, commit and recovery require the entire manifest to remain empty for this boundary.
The planner captures profile/rollback state and permissions even when there are no launcher or
cache changes. Exact file identities, sentinel ownership, canonical rollback paths, stale-approval
rejection and journal commit markers continue to govern mutation and recovery.
