# 0076 — App script hooks share the parent lifecycle Plan

- **Status**: accepted
- **Evidence**: `core/src/runtime/{app,app_metadata,planner}.rs`,
  `presets/app/clash-verge/{shine.toml,build.ts}`

## Context

Command-form lifecycle hooks can reload an application after managed files change, but they cannot
portably locate a Preset script or receive the fixed `SHINE_APP_*` contract. Calling
`shine app artifact apply` from a hook is unsafe: the artifact owns a separate snapshot-bound Plan,
so the nested process attempts to compose approvals and transaction state after the parent Plan was
reviewed. Clash Verge nevertheless needs an idempotent post-upgrade synchronization: local provider
files should refresh immediately, while changed subscription bindings retain their existing
reselect-then-refresh flow.

## Decision

App hooks accept either `command` or `script`; the two forms are mutually exclusive. Script hooks
reuse the native/Bun script resolver, dependency policy, declared environment mapping, fixed App
contract, exact-path overlay behavior, and scoped App-hook trust. Their executable permission,
runtime command, input identities, source snapshot, and any embedded category materialization are
included in the parent install/upgrade Plan. Execution occurs only after the category's managed
files and receipt commit, and a failure remains a non-fatal hook outcome.

`clash-verge` declares only a Bun `post_upgrade` script hook and reuses `build.ts`. When its four
bound editor documents are already current, the script refreshes every declared rule-provider and
then closes existing mihomo connections. When rendering changes a bound document, it performs no
controller request; the user reselects the subscription and runs the explicit artifact command.

## Consequences

- Lifecycle automation gains the artifact-style path and environment contract without nesting a
  second mutation or approval.
- Existing command hooks and explicit artifact apply/remove semantics remain unchanged.
- Script-hook metadata must declare exactly one action, validate Bun extensions, and describe its
  execute, command, environment, and network capabilities.
- Automatic Clash refresh runs only when that installed category actually changes; controller
  failure warns without undoing a successful managed-file upgrade.
