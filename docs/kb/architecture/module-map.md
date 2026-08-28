# Module Map

Use this map to find the owner of a command or behavior. It describes module responsibility, not
behavioral contracts; read [`invariants.md`](invariants.md) and [`data-flows.md`](data-flows.md)
before changing cross-module behavior.

Update this file when modules move, split, merge, or take on a different responsibility.

## Workspace layout

| Path | Responsibility |
|---|---|
| `Cargo.toml` | Workspace manifest and publishable `shine-cli` package root |
| `cli/` | Main `shine` binary plus its library crate |
| `cli/build.rs` | `rust-embed` rebuild trigger for `presets/` |
| `utils/` | Reusable `shine-core` package with no CLI/Tauri dependency |
| `utils/src/lifecycle.rs` | Versioned frontend-neutral lifecycle result envelope and safe effect/status vocabulary |
| `utils/src/runtime/` | Internal Core runtime facade, immutable preset inputs, host ports, in-memory host, domain models, manifests, and migrated executors |
| `utils/src/install/` | Core-owned transforms, EOL handling, App manifest, and host-neutral managed-file operations |
| `utils/src/persist.rs` | Core-owned atomic persistence and versioned TOML helpers |
| `presets/` | Embedded shell, app, and OS bootstrap assets |
| `skills/shine-preset-author/` | Portable AI workflow and kind-specific preset author references |
| `docs/manual/` | Default English public manual |
| `website/i18n/zh-Hans/` | Simplified Chinese public manual and UI locale |
| `docs/kb/` | Internal agent knowledge: invariants, flows, decisions, operations, lessons |

## CLI entry and routing

| Path | Responsibility |
|---|---|
| `cli/src/main.rs` | `fn main`, top-level `run()` dispatch, background version notification |
| `cli/src/lib.rs` | Library module tree; exposes handlers that require lib-level tests |
| `cli/src/commands/cli.rs` | Root Clap parser and top-level argument types |
| `cli/src/commands/*.rs` | Domain subcommand enums and argument types |
| `cli/src/shim.rs` | Canonical `install`/`uninstall <TARGET>` routing and bare-category ambiguity checks |
| `cli/src/init.rs` | `shine init` project-local `shine.config.toml` creation |
| `cli/src/completion.rs` | Static and dynamic shell completion before ordinary CLI initialization |
| `cli/src/list.rs` | Top-level installed/available status views |
| `cli/src/info/` | Canonical target resolution, installed data collection, and diff rendering |
| `cli/src/path_display.rs` | Home-relative terminal path formatting |
| `cli/src/colors.rs` | Terminal color helpers |
| `cli/src/output.rs` | Shared command output mode and rendering support |
| `cli/src/presentation.rs` | CLI-private lifecycle events, writer-backed terminal renderer, and interaction ports |
| `cli/src/platform.rs` | Platform classification shared across command domains |
| `cli/src/privilege.rs` | Cross-platform administrator/elevation orchestration |
| `cli/src/proc.rs` | Small domain-neutral subprocess helpers |
| `cli/src/persist.rs` | Compatibility re-export of Core persistence helpers |
| `cli/src/shell_quote.rs` | Copy-paste-safe shell argument rendering |
| `cli/src/version.rs` | Version string formatting |

### Command routing table

| Top-level command | Handler |
|---|---|
| `shell list/info/install/uninstall` | `cli/src/shells/` |
| `app list/install/uninstall` | `cli/src/apps/` |
| `install/uninstall <TARGET>` | `cli/src/shim.rs` → `apps/` or `shells/` |
| `list [--available [KIND]]` | `cli/src/list.rs` or scoped list handlers |
| `info <TARGET>` | `cli/src/info/`; explicit system items delegate to `sys/` |
| `app artifact apply/remove <app-id>` | `cli/src/apps/build.rs` |
| `app refresh <app-id> [file]` | `cli/src/apps/refresh.rs` |
| `sys list/bootstrap/profile/...` | `cli/src/sys/` |
| `theme sync` | `cli/src/theme/` |
| `env ...` | `cli/src/env/` plus `cli/src/secret/` |
| `preset export/copy/link/unlink/overlay` | `cli/src/preset_commands.rs` |
| `preset new/validate` | Kind template handlers and `cli/src/preset_validation.rs` |
| `preset pull`, `update --pull`, `upgrade --pull` | `cli/src/git_pull.rs` plus top-level routing |
| `init` | `cli/src/init.rs` |
| `self install/upgrade` | `cli/src/self_install.rs`, `cli/src/update_check/` |
| `update [TARGET]`, `upgrade [TARGET]` | filtered app/shell/sys handlers plus update/self-install logic |
| `state migrate` | `cli/src/state.rs` |
| `serve install/start/status/uninstall/url` | `cli/src/serve.rs` |
| `ssh ...` | `cli/src/ssh/mod.rs` |
| `local download/upload/status` | `cli/src/ssh/remote_client.rs` |
| `task save/run/list/info/delete`, `run <NAME>` | `cli/src/task/` |
| `completions` | `cli/src/main.rs` with `clap_complete` |

## App installation

| Path | Responsibility |
|---|---|
| `cli/src/apps/mod.rs` | Shared app kernel and handler re-exports |
| `cli/src/apps/install.rs` | App install orchestration and structured file/cache/hook outcomes |
| `cli/src/apps/uninstall.rs` | Manifest-driven category uninstall and structured teardown/cache/purge outcomes |
| `cli/src/apps/upgrade.rs` | Installed app upgrades, stale-entry cleanup, and structured outcomes |
| `cli/src/apps/info.rs` | App list/info status |
| `cli/src/apps/report.rs` | Install/uninstall outcome formatting |
| `cli/src/apps/metadata.rs` | `shine.toml` app schema and parsing |
| `cli/src/apps/annotation.rs` | Legacy `shine-dest:` annotation parsing |
| `cli/src/apps/generator.rs` | `[[files]].generator` execution, limits, and permission gate |
| `cli/src/apps/refresh.rs` | Explicit generator refresh with ownership/modification guards |
| `cli/src/apps/hooks.rs` | Shared `post_install`/`post_upgrade` hook runner |
| `cli/src/apps/build.rs` | Explicit artifact apply/remove and uninstall teardown |
| `cli/src/apps/json_merge.rs` | Managed-key JSON merge strategy |
| `cli/src/install_core/file_ops.rs` | Copy, backup, restore, privileged filesystem operations |
| `cli/src/install_core/manifest.rs` | Compatibility re-export of Core-owned `app-manifest.toml` types |
| `utils/src/install/` | App manifest, file ownership primitives, `jsonc-to-json`/`template`, and EOL helpers |

`install_core` contains app/sys-shared primitives only; `sys` depends on it, not on app-specific
logic.

## Shell deployment

| Path | Responsibility |
|---|---|
| `cli/src/shells/mod.rs` | Shell types, shared accessors, handler re-exports |
| `cli/src/shells/deployment.rs` | Embedded/external snapshot/live orchestration using Core-owned Shell models and manifest |
| `cli/src/shells/install.rs` | Category/command install, read-only pending assessment, and installed-shell upgrade results |
| `cli/src/shells/uninstall.rs` | Category/command uninstall results with sibling/cache and foreign-launcher protection |
| `cli/src/shells/links.rs` | Launcher/link specifications and conflict reporting |
| `cli/src/shells/report.rs` | Shell list/install/uninstall/upgrade reporting |
| `cli/src/shells/profile.rs` | PATH/source-command profile blocks |
| `cli/src/shells/template.rs` | Installed shell template rendering |
| `cli/src/shells/metadata.rs` | Shell category/file metadata parsing |
| `cli/src/bin_links.rs` | Native symlinks/shims and managed Bun launchers in `~/.shine/bin/` |
| `cli/src/bun_runtime.rs` | Shared source-scoped Bun dependency policy and command construction |
| `cli/src/sentinel.rs` | Shared sentinel-block primitives used by shell and sys profiles |

## System configuration

| Path | Responsibility |
|---|---|
| `cli/src/sys/commands.rs` | List/info/status/bootstrap orchestration and manifest loading |
| `cli/src/sys/bootstrap.rs` | Read-only detection plus Homebrew/APT/Winget/script install actions |
| `cli/src/sys/detect.rs` | OS and Linux distribution detection |
| `cli/src/sys/managed.rs` | Managed-resource apply/update/remove/upgrade and structured result adapters |
| `cli/src/sys/model.rs` | Sys manifest and runtime outcome models |
| `cli/src/sys/manifest.rs` | Preset parsing and validation |
| `cli/src/sys/run_manifest.rs` | Compatibility re-export of Core-owned Sys manifest and receipt state |
| `cli/src/sys/selection.rs` | Positional/profile/interactive item selection |
| `cli/src/sys/execution.rs` | Bootstrap reporting and proxy environment |
| `cli/src/sys/render.rs` | System command presentation helpers |
| `cli/src/sys/resources.rs` | Typed `SystemDriver` resource outcomes/conflicts and built-in dispatch |
| `cli/src/sys/drivers/` | Built-in managed-resource drivers such as split DNS and managed file |
| `cli/src/sys/profile.rs` | Generated profile install and three-way reconciliation |
| `cli/src/sys/profile_compose.rs` | Deterministic base plus enabled-item composition |
| `cli/src/sys/profile_commands.rs` | Explicit profile enable/disable |
| `cli/src/sys/profile_blocks.rs` | Phase-specific sentinel insertion/removal and BOM handling |

## Configuration, presets, and runtime state

| Path | Responsibility |
|---|---|
| `cli/src/config/mod.rs` | `Config` model and accessors |
| `cli/src/config/discovery.rs` | Global/project and environment path priority |
| `cli/src/config/load.rs` | Load/init and schema/version layering |
| `cli/src/config/save.rs` | Atomic, comment-preserving, sparse project saves |
| `cli/src/config/env_layer.rs` | `[env]` parsing/defaults and override files |
| `cli/src/presets.rs` | Embedded extraction, active asset reads, category enumeration |
| `cli/src/preset_commands.rs` | Preset copy/export/link/unlink/overlay commands |
| `cli/src/preset_meta.rs` | Shared preset kind and canonical target metadata |
| `cli/src/preset_validation.rs` | Config-independent preset discovery, static validation report, and text/JSON rendering |
| `cli/src/git_pull.rs` | FF-only external source pulls and managed overlay mirroring |
| `cli/src/state.rs` | Versioned runtime-state cleanup |
| `cli/src/status.rs` | Shared typed App/Shell status assessment, row builders, and App read-only lifecycle results |

Config discovery priority is documented as a behavioral contract in
[`data-flows.md`](data-flows.md#config-discovery) and
[ADR 0005](../decisions/0005-config-discovery-priority-chain.md).

## Environment and secrets

| Path | Responsibility |
|---|---|
| `cli/src/env/mod.rs` | `[env]` configuration and substitution core |
| `cli/src/env/commands.rs` | Env CRUD and secret command handlers |
| `cli/src/env/workspace.rs` | Workspace env sources, export, seal, and child execution |
| `cli/src/env/catalog.rs` | Known variable metadata |
| `cli/src/env/identity.rs` | age identity generation and recipient inspection |
| `cli/src/env/upgrade.rs` | Reapply env templates to installed content |
| `cli/src/env/proxy.rs` | Transparent command proxy rules and execution |
| `cli/src/env/broker.rs` | Secret-broker workspace snapshots and local policy store |
| `cli/src/secret/mod.rs` | Backend-independent tagged ciphertext router |
| `cli/src/secret/gpg.rs` | GPG backend |
| `cli/src/secret/age.rs` | age and Secure Enclave backend |
| `cli/src/secret/exec.rs` | Shared temporary-file and subprocess helpers |

## SSH

| Path | Responsibility |
|---|---|
| `cli/src/ssh/mod.rs` | System-ssh wrapper, session setup, remote command wrapper, cleanup |
| `cli/src/ssh/session_context.rs` | Trusted reconnect arguments and local session state |
| `cli/src/ssh/protocol.rs` | Transfer and log-relay wire frames |
| `cli/src/ssh/agent.rs` | Local transfer server and rsync/scp execution |
| `cli/src/ssh/remote_client.rs` | Remote-side `shine local` handlers |
| `cli/src/ssh/broker.rs` | Local authorization, TTY confirmation, and decrypt-on-demand broker |

See [`data-flows.md`](data-flows.md#ssh-environment-forwarding),
[`invariants.md`](invariants.md#ssh-transfer), the
[transfer PRD](../../ssh-local-transfer-prd.md), and the
[secret-broker PRD](../../ssh-secret-broker-prd.md) before changing this subsystem.

## Other domains

| Path | Responsibility |
|---|---|
| `cli/src/task/mod.rs` | Personal command registry handlers and direct argv execution |
| `cli/src/task/manifest.rs` | `tasks.toml` model and persistence |
| `cli/src/theme/` | Terminal theme detection and synchronization |
| `cli/src/serve.rs` | Loopback server and per-user service management |
| `cli/src/update_check/` | Release lookup, cache, notification, and binary upgrade |
| `cli/src/self_install.rs` | Self-install and installed-configuration upgrade orchestration |
| `cli/src/home.rs` | Sudo-aware home discovery and path expansion |
| `cli/src/test_support.rs` | Shared cross-lib/bin test environment mutex |

## Embedded preset tree

| Path | Contents |
|---|---|
| `presets/shell/` | Shell commands/functions, including native and Bun runtimes |
| `presets/app/` | Managed application configuration, generators, hooks, and artifacts |
| `presets/sys/macos/` | macOS bootstrap manifest, scripts, and profile fragments |
| `presets/sys/ubuntu/` | Ubuntu bootstrap manifest, scripts, and profile fragments |
| `presets/sys/windows/` | Windows bootstrap manifest and PowerShell profile fragments |

Authoring rules and verification live in [`../preset-authoring.md`](../preset-authoring.md).
