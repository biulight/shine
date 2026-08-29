# Invariants

Non-obvious properties that must hold. Breaking any of these has caused (or would cause) real
bugs. Check this list before changing the modules named in each entry.

## Install / uninstall safety

- **A security Plan is not dry-run and approval is snapshot-bound.** `PlanV1` contains only ordered
  semantic steps, safe diagnostic codes, resolved permissions, and digests of exact source/state
  observations. Planning must not write through a host, execute generator/hook/artifact/bootstrap
  code, or carry content, env values, secret plaintext, or raw argv. A future apply path must
  regenerate the Plan from fresh captured inputs and match both its fingerprint and exact required
  permission set before the first mutation; missing or uncomputable permissions fail closed.
- **Preset snapshot identity excludes checkout location but includes the trust layer.** The v1
  digest binds sorted effective logical paths, exact bytes, and each file's embedded, external, or
  overlay origin. It must not include physical source roots: relocating unchanged source is not a
  semantic change, while changing an effective layer is. State observations use the same framed
  SHA-256 contract and represent secrets only by opaque handles or versions, never plaintext.
- **A Preset permission declaration is not a grant.** App categories, Shell commands, and Sys items
  may declare schema-v1 capability identities, but those declarations do not bypass
  `allow_app_hooks`, `allow_sys_code`, administrator authorization, ownership checks, or future
  Plan approval. Filesystem declarations use logical bases and never embed a physical Preset
  checkout path; command entries contain no argv and environment entries contain names and
  sensitivity only. Missing declarations are a static compatibility warning until the enforcement
  migration explicitly changes that policy.

- **Host observation is explicit and domain execution receives captured inputs.** A shared runtime
  bootstrap discovers external/overlay directories and constructs snapshots through the selected
  filesystem host, so CLI, UI, tests, and future host adapters reuse one implementation. Runtime
  contexts carry home, Shine roots, platform, environment, and the resulting immutable snapshot;
  App/Shell/Sys domain logic never rediscovers them from ambient globals. Distribution-only
  embedded bytes enter from the frontend. Validation additionally receives captured cwd, manifest
  persistence always requires a filesystem host, and Sys inspection/preflight stays read-only;
  only authorized script execution atomically materializes the category below the Shine runtime
  root. Core never reopens an external preset tree through `Path` helpers.
- **A host abstraction does not weaken ownership checks.** Real and in-memory managed-file paths
  share the same content hash, backup suffix, receipt, user-modification preservation, and
  manifest-version gates. Adding a new host operation must retain those checks rather than treating
  a successful write primitive as ownership evidence.

- **Reusable lifecycle results contain identities and codes, never payloads.** Structured outcomes
  may record canonical targets, logical resource names, status, effects, and stable diagnostic
  codes. They must not copy raw errors/logs, source or destination content, environment or secret
  values, subscription URLs, credentials, or absolute destination paths into a reusable result.
  Human CLI diagnostics remain subject to their domain-specific redaction rules.
- **Lifecycle presentation is a CLI-only side channel, never a result payload.** App, Shell, and
  managed Sys may emit home-relative paths, human diagnostics, hook notes, and section events to a
  replaceable reporter, but those events are not serializable and must never be copied into
  `LifecycleResultV1`. Confirmation and administrator authorization use the separate interaction
  port so reusable execution does not depend on dialoguer or terminal rendering.
- **Lifecycle status distinguishes observation from execution.** Read-only `update` uses
  `dry_run = false` and `Pending`; explicit dry-run uses `dry_run = true` and `Previewed`.
  `Changed` means this execution changed Shine-owned state. App hooks/teardown may record execution
  effects without changing their established non-fatal exit semantics; managed Sys user
  modification remains a typed preservation conflict while the CLI keeps its existing failure exit.
- **App update presentation and reusable results share one Core assessment pass.** `AppRow` and
  `LifecycleOutcomeV1` must derive from the same per-file `AppFileAssessment`; do not rebuild rows
  to obtain the structured result. Automatic generators may execute during the established status
  path, so evaluating the file twice can duplicate external code execution and observe inconsistent
  snapshots. Typed inspection paths remain non-serializable and must not enter the lifecycle result.
- **Managed-file update details are field labels, not payloads.** The read-only comparison may
  report that destination or content changed, but must not copy the destination, desired bytes, or
  environment values into structured lifecycle outcomes. Ownership and user-modification checks
  still occur at apply/remove time against the receipt and live resource.
- **Every runtime manifest is a pre-mutation gate.** App, Shell, and Sys load their own manifest
  before destination/resource, embedded cache, snapshot/render, launcher, receipt, or profile
  mutation. Missing `schema_version` is legacy v0, read-only loads never rewrite it, the next
  successful mutation writes v1, and an unsupported future version fails first. The inner Sys
  receipt `version` is independent of `sys-manifest.toml`'s schema version.

- **Uninstall never touches user files.** `presets::remove_prefix` removes only embedded-asset
  files; `bin_links::unlink_managed` removes only symlinks pointing into the managed presets dir,
  plus **regular files that carry the `# shine-managed` marker and a `# shine-target:` under the
  managed root** (Windows `.ps1`/`.cmd` shims and Unix `runtime = "bun"` launcher scripts); app
  uninstall is driven by `~/.shine/app-manifest.toml` entries only.
- **Bun launchers are marked regular files, identified only by content, never by name.** A Unix
  `runtime = "bun"` command is a generated executable script (not a symlink); ownership and
  current-ness (`bin_links::launcher_target`/`unix_launcher_status`, mirrored by
  `windows_shim_status`) key on the `# shine-managed` marker + a `# shine-target:` path under the
  managed root. An unreadable/non-UTF-8/unmarked file is always `NotManaged` — treated as a user
  conflict, never overwritten or removed. `unix_bun_launcher_content` is byte-deterministic so a
  format change re-detects installed launchers as stale on upgrade; changing it is a format bump.
  The content embeds the entry's ordered `env` spec (the `--with` tokens of the
  `shine env run --no-workspace … -- bun … <script>` wrapper) and its Bun dependency policy, so
  either declaration changing refreshes the launcher. Ownership/removal still key only on the
  marker + target, independent of `env` and dependency mode.
- **Backups use the `<name>.shine.bak` suffix** (`install_core/file_ops.rs::backup_path`).
  Uninstall restores from that exact name; changing the suffix orphans existing backups.
- **An app source has exactly one manifest destination.** A per-file `dest` overrides the category
  root, but changing that effective destination must relocate by manifest `source`: verify the old
  copy is unmodified and the new path is free, then replace the manifest entry. Never update the old
  path and separately install the new one; that leaves two owned copies for one source. Duplicate
  effective destinations within the active metadata fail before any install or upgrade writes.
- **`requires_admin` must persist on every manifest entry** (`install_core/manifest.rs::AppEntry`).
  Uninstall routes to the sudo removal path based on the stored flag, not by re-checking the
  path. Dropping it during (de)serialization silently breaks privileged uninstall (commit
  `70ee910`).
- **Privileged filesystem transactions must hold the host-provided cross-process admin lock** for
  their complete ownership-check, backup, mutation, and rollback sequence
  (`PrivilegedFileSystemHost::acquire_privileged_operation`, `$TMPDIR/shine-admin.lock`). Locking
  individual writes is insufficient because another process could race between backup and replace.
  Self-install uses the same host lock (commit `fbd9c55`).
- **No code path should ask the user to manually type `sudo` on Unix.** Frontend interaction
  authorizes through `privilege::ensure_admin`; the real privileged host performs non-interactive
  mutations with the cached authorization. Self-install uses its CLI-only command adapter.
  Windows has no `sudo` equivalent, so its privileged paths still surface a manual
  "rerun elevated" hint instead.

## Shell profile editing

- **Sentinel blocks are the only thing shine writes to user shell configs**
  (`# >>> shine >>>` … `# <<< shine <<<`, Core Shell profile handling; sys uses per-phase sentinels
  like `# >>> shine <os> sys pre >>>`). Both delegate to the shared primitives in
  `core/src/sentinel.rs` (`find_block`/`extract_block_with_newline`/`remove_block_bytewise`/
  `remove_block_linewise`/`insert_block`/`trim_outer_blank_lines`).
- **Two sentinel removal styles exist and must not be unified without golden-output proof.**
  `sentinel::remove_block_bytewise` (shells' semantics) consumes one preceding blank line and
  never rewrites line endings; `sentinel::remove_block_linewise` (sys' semantics) never consumes
  a preceding blank line but normalizes CRLF to LF unconditionally (via `str::lines`), even when
  the sentinel isn't present. Canonicalizing them without characterization tests proving neither
  caller depends on the difference risks a silent formatting regression in a file shine doesn't
  own.
- **Line-ending differences must not register as changes on the sys path.** Preset templates are
  pinned LF (repo `.gitattributes`: `presets/** text eol=lf`), so the rust-embed'd template is
  byte-deterministic across build hosts. Core Sys reconciliation (`runtime/sys_profile/`) compares
  content line-ending-agnostically via `install::eol_eq`/`normalize_eol`: a pure CRLF↔LF difference
  (e.g. a Windows editor
  re-saving an installed loader file or profile) reports **no update** and leaves the user's file
  bytes untouched. When on-disk endings differ, the three-way merge skips `git merge-file` (which
  would see spurious per-line diffs) and uses the pure-Rust fallback over normalized bytes.
  Genuine content changes still write LF. The two sentinel *removal* styles above are unchanged —
  only the comparison layer normalizes.
- **Paths under `$HOME` are written as `$HOME/...`**, not absolute, for portability.
- **PowerShell profiles: preserve a leading BOM** when rewriting the file
  (`core/src/runtime/sys_profile/blocks.rs`, commit `81244f8`), and update **both** `Documents/PowerShell/` and
  `Documents/WindowsPowerShell/` profile files so pwsh and Windows PowerShell stay in sync.
- **Sys profile composition is activation-additive, not selection-replacing.** A targeted item or
  named selection profile enables successful item integrations but never disables previously
  enabled ones. Only `shine sys profile disable <ITEM>` removes that item's generated content.
  Composition order is phase, explicit priority, manifest order, then integration order; failures
  must leave the last installed profile intact.

## Config files

- **All `config.toml` writes go through `shine_core::sync_table`**, which preserves user comments.
  Never serialize the whole file from a struct — that destroys comments.
- **Config discovery priority is fixed**: `SHINE_CONFIG_DIR` > `SHINE_PRESETS` > `presets_dir`
  key > `~/.shine/` default. Code and
  [`data-flows.md`](data-flows.md#config-discovery) must agree.
- **External app preset hooks and generators are opt-in only.** `post_upgrade`
  runs commands after upgrades, while an automatic file generator may run
  during install/update/upgrade and supply effective source bytes. Embedded code may
  run implicitly, but external preset or overlay code must be gated by
  `allow_app_hooks = true`; otherwise a user-controlled presets checkout would
  gain command execution during ordinary read-oriented update checks.
- **Bun package installation is source-scoped and explicit.** Embedded scripts and external scripts
  without a locked declaration run with `--no-install`. Only an effective external/overlay script
  whose own physical category contains both `package.json` and `bun.lock` may run with
  `--install=fallback`; one file without the other and any `trustedDependencies` field are errors.
  Overlay package metadata never changes an inherited embedded script. Shine never runs
  `bun install`, owns `node_modules`, or cleans Bun's global cache/virtual store, and dependency
  download never bypasses `allow_app_hooks`. See ADR 0031.
- **External sys executable code is separately opt-in.** Static detection/provider metadata and
  declarative PATH/env/aliases are safe to inspect, but external or overlay bootstrap/managed scripts,
  guarded eval/source, fragments, and base profile code require `allow_sys_code = true`. Read-only
  status paths must never execute sys code, and update-check scripts require the same permission.
  This permission is global-only: a project
  config must never be able to authorize its own executable preset content.
- **Manual generators never run from implicit status or upgrade paths.**
  `generator.auto = false` leaves `list`/`info`/`update` local-only and
  causes upgrade to preserve the manifest snapshot. Only install (including `--replace-managed`) or
  `shine app refresh` may run it; refresh must target manifest-owned files and
  preserve user modifications unless `--force` is explicit.
- **Generator failures never destroy the last-known-good managed file.** Status
  and upgrade warn and retain an existing manifest-owned destination. An enabled
  generator with no successful installed snapshot fails rather than installing
  empty or partial output. Generator diagnostics must not include source URLs,
  credentials, or raw subscription records.
- **The Surge profile artifact treats `SURGE_PROFILE` as a user-owned file.**
  `profile-artifact.ts` rejects symlinks and invalid UTF-8, computes the full
  desired content before writing, replaces through a same-directory temporary
  file, and preserves mode and per-line endings. A missing patchable
  `#!include` is an error, never a successful no-op. Keep parsing and filesystem
  behavior shared by build and teardown; do not duplicate it in an overlay.
- **Local HTTP resources share one loopback server.** Files that need stable local URLs live under
  `<shine_dir>/http/` and are served by `shine serve start`; `shine serve install` registers one
  global user service for that server. Do not add per-app HTTP daemons, ports, or launchd jobs.

## Personal tasks

- **`tasks.toml` lives under `Config::shine_dir()`**, so it follows `SHINE_CONFIG_DIR` for free.
  Never resolve it against `presets_dir` or `$HOME` directly — that would break test isolation.
- **`shine task run` propagates the child exit code verbatim** and never runs the saved argv
  through a shell. Wrapping the failure in an anyhow error (or defaulting to exit 1) would corrupt
  the task's own exit semantics. Shell syntax is opt-in via an explicit saved `sh -c '...'`.
- **A missing task `cwd` means dynamic caller cwd.** Legacy `tasks.toml` entries have no `cwd`, so
  deserialization must default it to `None`; only an explicit `task save --cwd` fixes the working
  directory. Never reinterpret missing `cwd` as the save-time directory.

## Embedded presets

- **`cli/build.rs` must keep `cargo:rerun-if-changed=presets`.** Without it, preset edits
  don't trigger re-embedding and the binary silently ships stale assets.
- **The generated built-in platform capability blocks must use runtime selector semantics.**
  `preset_meta::tests::built_in_preset_platform_capability_docs_are_current` asks Core to derive App
  category and Shell command visibility from the pristine immutable snapshot for macOS, Linux, and Windows,
  then checks the delimited blocks in both public manual locales. Do not maintain a parallel
  platform map or weaken the test to trust prose labels; a preset metadata change and both generated
  blocks must land together. Regenerate them with
  `SHINE_UPDATE_PRESET_CAPABILITIES=1 cargo test built_in_preset_platform_capability_docs_are_current`.
- **Fallback depends on the selected preset mode.** In built-in mode, an overlay replaces matching
  paths and unmatched paths continue to read embedded assets. A full external preset source is
  authoritative for app and shell category discovery: a missing category or file is not silently
  borrowed from the binary. Sys profile installation has a narrower compatibility fallback to its
  embedded template when a selected external sys preset omits a previously-known profile file
  (commit `5606438`). Do not generalize that compatibility path into cross-source category fallback.

## External shell deployment

- **Shell command activation is command-scoped even though deployment is category-scoped.**
  Embedded extraction and external snapshots may materialize every source file in a category so
  commands can use sibling resources, but source-file presence alone never means that a command is
  installed. `shell-manifest.toml` entries and compatible legacy launchers are the activation
  receipts. Command-scoped install must upsert only its selected receipt; command-scoped uninstall
  must remove only its selected managed launcher/receipt and preserve installed siblings.
- **External source selection and installed state are separate.** Snapshot mode materializes
  effective shell categories below `<shine_dir>/installed/shell/`; launchers must never point at
  the user-owned external tree unless `external_shell_mode = "live"` is explicit.
- **Live transforms are manifest-constrained.** Generated launchers may request only a canonical
  target recorded in `shell-manifest.toml`; the renderer writes only below `rendered_dir`, stores
  no env values in the manifest, uses atomic replacement, and fails rather than executing stale
  output after a transform error.
- **External uninstall never deletes source.** It may remove Shine-owned snapshots, rendered
  files, manifest entries, and managed launchers, including legacy launchers pointing into the
  external tree. The external presets and overlay directories remain untouched.
- **Shell update/upgrade must preserve foreign launchers.** Ownership is checked with the same
  managed-root proof used by uninstall. A regular launcher outside that proof is a structured
  `Conflict`, is not counted as a pending update, is excluded from forced launcher refresh, and
  retains its command receipt. Compatible legacy/stale symlinks keep their existing upgrade repair
  behavior.
- **Preset materialization excludes `node_modules/` at every depth.** External Shell snapshots and
  overlay copies retain `package.json` and `bun.lock`, but never copy a local installation tree into
  Shine-owned state or embedded extraction.

## Secrets

- **Decrypt routing is tag-based only** (`secret::parse_tagged_ciphertext`). `decrypt_secret`
  must never consult `Config::secret_backend` or any other config to pick a backend — only the
  `age:` prefix (or its absence) decides. This lets `secret_backend`/`age_recipients` change
  freely without breaking previously-encrypted secrets (see
  [ADR 0008](../decisions/0008-age-secret-backend-tagged-ciphertext.md)).
- **GPG ciphertext stays untagged.** Adding a tag to existing GPG secrets, or changing the `age:`
  prefix, breaks every secret encrypted before the change.
- **Workspace export decrypts only on explicit request.** `shine env workspace export` omits
  secret-winning keys unless `--include-secrets` is present, never mixes in the caller's process
  environment, and never prints exported values in its status or dry-run output. On Unix, an
  export containing secrets must be created privately (`0600`) before plaintext bytes are written;
  chmod after a wider temporary-file write is not sufficient.

## SSH transfer

- **`ssh::agent` must not trust wire-supplied fields beyond the session token.** The token is the
  only authorization check on a `PutFile`/`GetFile`/`Status` request, but it travels to the remote
  host as plain argv/environ (`env SHINE_SSH_TOKEN=...` in `ssh::mod`), so it can leak to other
  local users there via `ps eww`. Any field documented as constrained (e.g. `PutFile.filename` is
  meant to always be a bare basename) must be validated as such where it's consumed
  (`agent::ensure_single_path_component`), not just produced correctly by the one trusted
  `remote_client` implementation. `dest_hint`/`source_hint` are expanded with `~`-only
  substitution (`home::tilde_expand`), never the full `${VAR}` expansion used for locally-typed
  paths elsewhere, so a forged hint can't pull values out of the local agent's own environment.
- **Per-connection transfer tasks must stay tracked in `agent::ConnectionTasks`, not bare
  `tokio::spawn`.** `agent_handle` (the accept loop's `JoinHandle`) does not cover them —
  `agent_handle.abort()` only stops new connections, never an in-flight transfer. `handle_ssh`
  must drain `ConnectionTasks` before removing the session directory or exiting, so a still-running
  transfer's own error-path cleanup gets to finish instead of being cut off by process exit.

## SSH secret broker

- **The session token is transport correlation, not broker authorization.** A remote peer that
  learns `SHINE_SSH_TOKEN` still must satisfy the session's exact direct allow-list or an exact
  local workspace policy. Every broker wire field is bounded, rejects control characters, and is
  treated as untrusted input.
- **Direct requests always require a local TTY confirmation and decrypt only `KEY_SECRET`.** They
  never use the plaintext fallback accepted by ordinary `env run`, and
  `--trust-remote-session` never suppresses direct-request confirmation.
- **Workspace authorization binds the whole request.** Matching includes SSH target, workspace
  bytes/hash, every source path and byte hash, mode, complete declared-secret set, exact release
  mapping, and exact argv. The local agent must reject rather than partially match any difference.
- **All-declared release is expansion, never a wildcard.** `--release-all-declared` resolves the
  current immutable snapshot into a sorted explicit release list before policy creation or update.
  A future declared secret/source change must invalidate the old policy and require review; runtime
  matching must never reinterpret the stored list as “whatever exists now.”
- **Workspace files are read once per broker run.** The remote snapshot sent for authorization and
  the values later merged into the child environment derive from the same in-memory bytes; never
  re-open a source after the local agent approves its hash.
- **Broker policies are local security state.** `<shine_dir>/ssh-secret-broker.toml` must not be a
  symlink, must be owned by the current Unix user, and must have mode `0600`; writes use an atomic
  same-directory temporary file. Remote enrollment is allowed only in the explicit trusted mode
  and still requires local confirmation.
- **Broker operations are serialized per SSH session.** A local confirmation keeps the SSH child
  paused and the TTY in its pre-SSH canonical/echo state through the subsequent GPG/age operation,
  then an RAII guard restores raw termios and resumes SSH on success or every error path. Dropping
  the guard immediately after the yes/no prompt breaks terminal pinentry and permits concurrent
  requests to corrupt each other's TTY interaction.
- **Broker UI is rendered only inside the local-TTY guard.** OpenSSH raw mode disables normal LF
  cursor handling, so inspect/enrollment details printed before termios restoration form a
  staircase and can interleave with the remote shell. Confirmation accepts plain `y`/`yes` and
  those exact values inside the standard bracketed-paste wrapper; arbitrary ANSI/control-decorated
  input remains a rejection.

## Local HTTP server

- **`serve::handle_start`/`handle_install` have no authentication of their own.** Binding
  loopback-only (`127.0.0.1`) keeps the server off the network, but it does not stop other local
  OS user accounts on a shared/multi-user machine from connecting and reading any file under
  `serve::http_root()` (`~/.shine/http/`), bypassing the filesystem permissions that would
  otherwise keep them out of this user's home directory. Preset authors must never route secrets
  or other sensitive content through a `dest` that resolves under `~/.shine/http`.
- **Persistent serve registrations are per-user and preserve the selected Shine directory.**
  launchd, systemd, and Windows Task Scheduler entries must run without administrator privileges
  and pass the resolved `shine_dir` back through `--config-dir`; background startup cannot depend on
  the installing shell retaining `SHINE_CONFIG_DIR` or its current directory.
- **launchd log paths must stay under the user's own `shine_dir`, never a shared path like
  `/tmp`.** `serve::launchd_log_dir` writes to `shine_dir/run/http/serve.{out,err}.log`, kept out
  of `http_root()` itself so log contents are never servable over HTTP. Two OS user accounts each
  running `shine serve install` would otherwise collide on the same fixed `/tmp/<label>.log` path.

## Update check

- **A failed or rate-limited version check must never fail the user's command** (`main.rs`,
  commits `605fdd8`, `f033a25`). Network errors are tolerated; rate-limit cooldowns are cached.
- **`preview` is not a release baseline.** Version comparisons and release notes must use the
  latest stable `v*` tag (see `conventions.md` § Versioning).
- **A targeted `shine upgrade <TARGET>` must not mutate other targets.** App filtering happens
  before stale-entry handling, shell filtering happens before link/template reconciliation, and
  managed sys apply receives the selected item. Filtering only the report after a global upgrade
  would violate the command's user-visible scope.

## Tests

- **Env-var mutation in tests must hold `crate::test_support::env_lock()`** — a single shared
  mutex used across the lib/bin boundary (it is deliberately *not* `#[cfg(test)]`-gated because
  `cfg(test)` does not cross that boundary).
- **Tests that touch real system paths** (e.g. docker-engine's `/etc/docker/daemon.json`) must
  additionally hold the cross-process admin lock for their full body (commit `fbd9c55`).
