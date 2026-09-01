# Built-in Executable Preset Inventory

Last classified: 2026-08-31

This inventory tracks the Roadmap Phase 4 requirement that every built-in executable Preset is
either migrated to typed declarative actions or explicitly classified. It is not a public feature
matrix. Re-run validation against the Preset source before changing execution, privilege,
provenance, trust, or rollback semantics.

Classification vocabulary:

- **declarative** — Core can determine the effect without executing Preset code;
- **opaque** — effect depends on a script, shell body, package manager, hook, generator, or artifact;
- **provenance** — built-ins are embedded; the same logical path may become external or overlay at
  runtime and then requires the scoped trust contract;
- **rollback** — describes lifecycle recovery, not reversal of user data processed by a command.

## App lifecycle code

| Target | Executable capability | Class | Privilege | Built-in provenance | Current rollback classification |
|---|---|---|---|---|---|
| `app/clash-verge` | `post_install`, `post_upgrade` invoking `shine app artifact apply` | opaque hook | user | embedded | hook side effects unsupported |
| `app/clash-verge` | Bun `build.ts` / `unbuild.ts` artifact | opaque artifact with explicit teardown | user | embedded | best-effort explicit teardown; not transactional |
| `app/surge` | `post_install`, `post_upgrade` app reload | opaque hook | user | embedded | unsupported |
| `app/surge` | Bun `build.ts` / `unbuild.ts` artifact | opaque artifact with explicit teardown | user | embedded | best-effort explicit teardown; user-owned profile preserved by its own guards |
| `app/surge` | manual Bun `generate-subscription.ts` generator | opaque generator | user | embedded | last-known-good managed file preserved; execution itself not reversible |

All other built-in App file copy/transform/JSON-merge effects are Core-typed rather than executable
Preset code. Phase 4 now covers absent-destination and backup-aware unowned regular-file static Copy
creation, unchanged receipt-owned in-place static Copy update, ordinary removal of an unchanged
receipt-owned static Copy with or without restoration of its fixed persistent backup, and forced
removal of a user-modified static Copy. These static Copy actions support both user and
administrator paths. JSON merge install/update/removal is also typed with key-owned rollback;
static Copy relocation is typed across its old receipt/path/backup and absent new destination.
JSON relocation is typed across separate old/new managed-key sets, both destinations, the old
rollback, and the replacement receipt. Generators remain explicitly opaque.

## Shell commands

Shell command *deployment* is Core-owned and declarative, but invoking the installed command runs
Preset code and can affect user-selected inputs. Uninstall owns only the launcher, rendered copy and
manifest receipt; it never attempts to reverse command side effects.

First-time launcher creation is now a typed, journaled action across Unix symlinks, Unix Bun/live
files, and Windows shim pairs. Explicit recovery removes only unchanged transaction-created
resources or preserves an exact durable command receipt. Replacement of an unchanged,
receipt-owned launcher is also typed: exact old resources move to same-directory rollback material
until the new receipt commits. Unchanged receipt-owned launcher removal is typed with a positive
receipt-commit marker. Raw external snapshot-mode selections without rendered output now replace
their category tree through a typed action with deterministic stage/rollback directories, selected
receipt transitions, and positive commit evidence. Embedded cache writes now use category-scoped
file-patch actions, and lifecycle-rendered output uses file-scoped replacement/removal actions.
Invocation-time live rendering is atomic, serialized with lifecycle/recovery, and blocked by a
pending journal without creating its own persistent transaction. Cache and snapshot uninstall use
typed removal actions with receipt transitions and positive commit evidence. Shell profile
reconciliation is sentinel-owned: recovery restores only Shine blocks in the current file and
preserves unrelated edits.

| Category | Targets | Runtime/class | Privilege | Built-in provenance | Rollback classification |
|---|---|---|---|---|---|
| `shell/agent` | `ccenv` | opaque Bun command | user | embedded | launcher deployment reversible; command effects outside lifecycle |
| `shell/image-tools` | `img-compress`, `img-resize`, `img-convert` | opaque Bun commands | user | embedded | launcher deployment reversible; output files are user-owned |
| `shell/proxy` | Unix/PowerShell `setproxy`, `usetproxy` | opaque sourced shell code | user | embedded | launcher/profile activation reversible; current-shell effects not journaled |
| `shell/utils` | `copyfile`, Unix/PowerShell `shine-env-export`, `shine-theme-sync` | opaque shell/PowerShell code | user | embedded | launcher/profile activation reversible; command effects outside lifecycle |

## Sys bootstrap and managed resources

Package-provider invocations and scripts are opaque external effects even when their command and
package identities are typed. They are presence-oriented bootstrap operations: Shine records that
the target ran but does not own third-party package uninstall or version rollback.

| Platform | Targets | Execution class | Privilege | Built-in provenance | Rollback classification |
|---|---|---|---|---|---|
| macOS | `homebrew`, `rust`, `astronvim` | opaque scripts | script/provider dependent | embedded | unsupported |
| macOS | Homebrew/Cask package items | opaque typed provider invocation | provider dependent | embedded | package uninstall/rollback unsupported |
| Ubuntu | `rust`, `astronvim`, `atuin`, `yazi`, `starship`, `zoxide`, `bat`, `eza`, `bun`, `pnpm`, `mise`, `homebrew`, `zerotier` | opaque scripts | script/provider dependent | embedded | unsupported |
| Ubuntu | APT/Homebrew package items | opaque typed provider invocation | provider dependent | embedded | package uninstall/rollback unsupported |
| Windows | WinGet package items | opaque typed provider invocation | provider/elevation dependent | embedded | package uninstall/rollback unsupported |
| all supported platforms | `split-dns` | typed managed system action | administrator | embedded | journaled exact-state create/update/remove with receipt-gated recovery |
| all supported platforms | other `mode = "managed"` resources | typed managed-file action | item dependent | embedded | journaled create/update/relocate/remove with fingerprint and receipt recovery |
| all supported platforms | explicit `sys profile enable/disable` shell sentinels | typed owned-subset profile action | item dependent | embedded | journaled sentinel recovery preserves unrelated current content |
| all supported platforms | generated active/base/new/merge profile files and bootstrap profile composition | typed composition plus three-way merge | item dependent | embedded | explicitly non-transactional; conflict/force behavior is reviewed before execution |

## Phase 4 migration order

1. App absent-destination managed file create and explicit recovery (implemented).
2. App backup-aware unowned regular-file static Copy create and restore (implemented).
3. App receipt-owned in-place static Copy update (implemented).
4. App ordinary, backup-restoring, and forced static Copy remove, including administrator paths
   (implemented).
5. Administrator static Copy create, backup-aware create, and in-place update plus key-owned JSON
   merge install/update/removal (implemented).
6. App upgrade stale-prune removal for unchanged static Copy and JSON receipts plus static Copy and
   key-owned JSON relocation (implemented).
7. Shell launcher creation/update/removal, snapshot/cache replacement and removal, rendered-output
   replacement/removal, and sentinel-owned profile reconciliation (implemented). Live rendering is
   explicitly invocation-scoped and serialized with lifecycle recovery.
8. Managed Sys files, explicit profile sentinel blocks, and split DNS (implemented).
9. Preserve App hooks/generators/artifacts, Shell command bodies and Sys scripts/providers as explicit
   opaque escape hatches, and profile composition as explicitly non-transactional, unless a narrower
   typed action replaces them (classified).

Any new built-in executable capability must enter this inventory in the same change.
