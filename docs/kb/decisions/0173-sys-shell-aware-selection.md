# 0173 — Sys automatic selection reuses shell integration declarations

- **Status**: Accepted
- **Evidence**: `core/src/runtime/sys_model.rs`, `core/src/runtime/sys_bootstrap/selection.rs`,
  `cli/src/sys/{commands,render}.rs`

## Decision

Default Sys lists, interactive candidates/defaults, and named/default bootstrap profiles use the
existing captured target shell and `items.shell.shells`. `SysItem::matches_shell` includes items
without integrations or with any matching integration; managed items remain unchanged. Detection,
`when_command`, phase and installation state do not determine this selection. A second shell
compatibility field would duplicate the author-maintained declaration, so none is added.

Validate the complete manifest and original selection before filtering. A valid filtered-empty
selection succeeds with its original source; an originally empty init catalog retains `NoItems`.
Missing default profiles, unknown names and invalid metadata still fail. Explicit positional IDs
and repeated `--item` bypass filtering. Full catalog and info lookup remain available.

Filtering is a selection/presentation policy, not an installation prohibition. Keep the complete
manifest for composition and approval, including unselected enabled integrations. Approval and
execution use exact resolved IDs, without re-expanding profiles. The CLI retains the initial
`SelectionSource` for display because approved execution resolves these IDs as `Items`; display
metadata does not enter Plan identity or change receipts.
An empty exact-ID approved request revalidates approval and returns an empty report directly;
passing it to the implicit batch selector would incorrectly expand the default profile. Empty
CLI results escape the retained selection source just like nonempty run headers.

## Platform boundaries

Fish/Elvish have no `SysShellKind` match: selection accepts items without integrations and excludes
items with integrations, without a fallback or error in this layer. Existing profile/installer
support remains unchanged. macOS uses the same filtering but retains its Zsh base/fragments and
`.zshrc` loading; selection does not promise macOS Bash integration support. Built-in declarations,
fragments, manifest versions, trust, permissions, recovery and approval contracts are unchanged.
