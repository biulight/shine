# 0068 — Shell and managed Sys complete the Phase 4 recovery boundary

- **Status**: Accepted
- **Date**: 2026-09-01
- **Evidence**: `core/src/action.rs`, `core/src/runtime/{planner,shell_action_executor,sys_action_executor}.rs`

## Context

Phase 4 already journaled App resources and most Shell deployment effects, but its Roadmap gate
remained open. Shell cache and snapshot uninstall plus shell-profile sentinel reconciliation still
mutated outside the Shell journal. Managed Sys files, split DNS, and explicit Sys profile sentinel
changes had receipt ownership but no durable action journal. Package providers, bootstrap scripts,
and profile three-way merge outputs cannot offer the same local fingerprint rollback proof.

Treating all of these effects as one generic file transaction would either overstate rollback
support or let recovery overwrite unrelated user edits in shell profiles.

## Decision

Action IR v1 adds typed Shell removal actions for embedded cache files and external snapshot trees,
plus a sentinel-owned profile reconciliation action. The existing Shell journal binds exact
receipt transitions and positive commit evidence. Profile recovery compares only Shine-owned
sentinel blocks and restores or removes those blocks in the current file, preserving unrelated
content written after interruption.

Managed Sys mutations use a distinct `sys-operation-journal.toml` and explicit
`shine sys recover [--yes]` Plan. Typed actions cover managed-file create, update, relocation, and
removal; split-DNS state; and the shell-profile sentinel files changed by explicit
`sys profile enable/disable`. The journal binds the exact previous and desired Sys receipt. Receipt
commit is positive evidence: before it, recovery restores only fingerprint-matching previous
state; afterward, it keeps the desired state and cleans exact rollback material. A changed resource,
rollback path, sentinel-owned block, or conflicting receipt blocks mutation.

Sys profile composition retains its established three-way merge for generated active/base/new/merge
files. Those files, bootstrap profile composition, package/provider calls, bootstrap scripts, App
hooks/generators/artifacts, and installed Shell command bodies are explicitly classified as opaque
or non-transactional in the security Plan and executable inventory. Phase 4 does not promise
rollback across package managers, network effects, command side effects, or arbitrary user data.

## Consequences

- App, Shell deployment, and managed Sys resources now have explicit, domain-scoped recovery
  commands and journals.
- Recovery never uses a stale Plan as authorization and never restores bytes or owned blocks after
  their recorded identity changes.
- User-editable profile files use sentinel-granular ownership instead of whole-file rollback.
- Unsupported profile merge, bootstrap, provider, and executable-code effects are visible before
  execution rather than inheriting an inaccurate transaction guarantee.
- These migrated actions and explicit classifications satisfy the Roadmap Phase 4 gate; automatic
  resume remains intentionally out of scope.
