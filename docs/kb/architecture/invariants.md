# Invariants

Non-obvious properties that must hold. Breaking any of these has caused (or would cause) real
bugs. Check this list before changing the modules named in each entry.

## Install / uninstall safety

- **Uninstall never touches user files.** `presets::remove_prefix` removes only embedded-asset
  files; `bin_links::unlink_managed` removes only symlinks pointing into the managed presets dir;
  app uninstall is driven by `~/.shine/app-manifest.toml` entries only.
- **Backups use the `<name>.shine.bak` suffix** (`apps/file_ops.rs::backup_path`). Uninstall
  restores from that exact name; changing the suffix orphans existing backups.
- **`requires_admin` must persist on every manifest entry** (`apps/manifest.rs::AppEntry`).
  Uninstall routes to the sudo removal path based on the stored flag, not by re-checking the
  path. Dropping it during (de)serialization silently breaks privileged uninstall (commit
  `70ee910`).
- **Privileged filesystem mutations must hold the cross-process admin lock**
  (`apps/file_ops.rs::admin_lock`, `$TMPDIR/shine-admin.lock`). In-process mutexes are not
  enough: nextest runs each test in its own OS process, and real concurrent shine invocations
  exist too (commit `fbd9c55`).

## Shell profile editing

- **Sentinel blocks are the only thing shine writes to user shell configs**
  (`# >>> shine >>>` … `# <<< shine <<<`, `shells/profile.rs`). Removal must delete the block
  precisely, including the preceding blank-line separator.
- **Paths under `$HOME` are written as `$HOME/...`**, not absolute, for portability.
- **PowerShell profiles: preserve a leading BOM** when rewriting the file (`cli/src/sys/mod.rs`,
  commit `81244f8`), and update **both** `Documents/PowerShell/` and
  `Documents/WindowsPowerShell/` profile files so pwsh and Windows PowerShell stay in sync.

## Config files

- **All `config.toml` writes go through `utils::sync_table`**, which preserves user comments.
  Never serialize the whole file from a struct — that destroys comments.
- **Config discovery priority is fixed**: `SHINE_CONFIG_DIR` > `SHINE_PRESETS` > `presets_dir`
  key > `~/.shine/` default. Code and docs (AGENTS.md § Config) must agree.

## Personal tasks

- **`tasks.toml` lives under `Config::shine_dir()`**, so it follows `SHINE_CONFIG_DIR` for free.
  Never resolve it against `presets_dir` or `$HOME` directly — that would break test isolation.
- **`shine task run` propagates the child exit code verbatim** and never runs the saved argv
  through a shell. Wrapping the failure in an anyhow error (or defaulting to exit 1) would corrupt
  the task's own exit semantics. Shell syntax is opt-in via an explicit saved `sh -c '...'`.

## Embedded presets

- **`cli/build.rs` must keep `cargo:rerun-if-changed=../presets`.** Without it, preset edits
  don't trigger re-embedding and the binary silently ships stale assets.
- **Embedded templates are the fallback** when an external/overlay presets dir lacks a file
  (commit `5606438`). External presets mode must degrade to embedded content, not error.

## Secrets

- **Decrypt routing is tag-based only** (`secret::parse_tagged_ciphertext`). `decrypt_secret`
  must never consult `Config::secret_backend` or any other config to pick a backend — only the
  `age:` prefix (or its absence) decides. This lets `secret_backend`/`age_recipients` change
  freely without breaking previously-encrypted secrets (see
  [ADR 0008](decisions/0008-age-secret-backend-tagged-ciphertext.md)).
- **GPG ciphertext stays untagged.** Adding a tag to existing GPG secrets, or changing the `age:`
  prefix, breaks every secret encrypted before the change.

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

## Update check

- **A failed or rate-limited version check must never fail the user's command** (`main.rs`,
  commits `605fdd8`, `f033a25`). Network errors are tolerated; rate-limit cooldowns are cached.
- **`preview` is not a release baseline.** Version comparisons and release notes must use the
  latest stable `v*` tag (see `conventions.md` § Versioning).

## Tests

- **Env-var mutation in tests must hold `crate::test_support::env_lock()`** — a single shared
  mutex used across the lib/bin boundary (it is deliberately *not* `#[cfg(test)]`-gated because
  `cfg(test)` does not cross that boundary).
- **Tests that touch real system paths** (e.g. docker-engine's `/etc/docker/daemon.json`) must
  additionally hold the cross-process admin lock for their full body (commit `fbd9c55`).
