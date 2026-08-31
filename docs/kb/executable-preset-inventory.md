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
generators and relocation remain to migrate.

## Shell commands

Shell command *deployment* is Core-owned and declarative, but invoking the installed command runs
Preset code and can affect user-selected inputs. Uninstall owns only the launcher, rendered copy and
manifest receipt; it never attempts to reverse command side effects.

First-time launcher creation is now a typed, journaled action across Unix symlinks, Unix Bun/live
files, and Windows shim pairs. Explicit recovery removes only unchanged transaction-created
resources or preserves an exact durable command receipt. Replacement of an unchanged,
receipt-owned launcher is also typed: exact old resources move to same-directory rollback material
until the new receipt commits. Unchanged receipt-owned launcher removal is typed with a positive
receipt-commit marker. Shared snapshot/render material and profile sentinel blocks remain to
migrate.

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
| all supported platforms | `split-dns` | typed managed system action | administrator | embedded | receipt/fingerprint guarded removal; Phase 4 action migration pending |
| all supported platforms | generated shell profile integrations and other `mode = "managed"` resources | typed managed file/profile actions | item dependent | embedded | receipt/sentinel guarded removal; Phase 4 action migration pending |

## Phase 4 migration order

1. App absent-destination managed file create and explicit recovery (implemented).
2. App backup-aware unowned regular-file static Copy create and restore (implemented).
3. App receipt-owned in-place static Copy update (implemented).
4. App ordinary, backup-restoring, and forced static Copy remove, including administrator paths
   (implemented).
5. Administrator static Copy create, backup-aware create, and in-place update plus key-owned JSON
   merge install/update/removal (implemented).
6. App upgrade stale-prune removal for unchanged static Copy and JSON receipts (implemented);
   relocation remains.
7. Shell first-time launcher creation plus unchanged receipt-owned launcher update and removal
   (implemented); snapshot/render files and profile blocks remain.
8. Managed Sys files/profile blocks and split DNS.
9. Preserve App hooks/generators/artifacts, Shell command bodies and Sys scripts/providers as explicit
   opaque escape hatches unless a narrower typed action replaces them.

Any new built-in executable capability must enter this inventory in the same change.
