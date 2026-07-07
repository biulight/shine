# Lessons Learned

Dated entries mined from real bugs. Format: **symptom → root cause → fix → rule**.
Newest first. Cite the fixing commit. Add an entry whenever a bug's cause was non-obvious.

## 2026-07-07 — `shine ssh` could leak its local session directory on Ctrl-C

- **Symptom**: a PRD audit of `docs/ssh-local-transfer-prd.md` against the implementation found
  that `handle_ssh` (`ssh/mod.rs`) only ran its cleanup (`agent_handle.abort()` +
  `remove_dir_all(session_dir)`) after `cmd.status().await` on the spawned `ssh` child resolved.
  No `tokio::signal` handler existed anywhere in the crate, so a local Ctrl-C during an active
  session was delivered under SIGINT's default disposition and could terminate the `shine`
  process before that cleanup line ever ran, leaking `~/.shine/run/ssh/<session-id>` (containing
  the local transfer socket). The remote side was unaffected — its `trap ... EXIT` in the wrapped
  remote command is robust — only the local side was at risk.
- **Root cause**: nothing in the process intercepted SIGINT, so the OS's default disposition
  (immediate termination, no unwinding) applied. Cleanup code placed *after* an `.await` only runs
  if that `.await` resolves normally; it is never reached if the process is killed while still
  awaiting.
- **Fix**: raced `cmd.status()` against `tokio::signal::ctrl_c()` via `tokio::select!`. Installing
  the `ctrl_c()` listener itself overrides SIGINT's default disposition for the process, so once
  polled, a Ctrl-C resolves the listener future instead of killing the process outright; the `ssh`
  child (same foreground process group) receives SIGINT independently and exits on its own, and
  the parent awaits that exit before falling through to the existing cleanup. Verified against a
  stub `ssh` child (no real remote host available in this sandbox): sending `SIGINT` to the whole
  process group (`kill -INT -$pgid`, mirroring what a terminal does on real Ctrl-C) left the
  session directory removed and the process log clean, versus leaking it before the fix.
- **Rule**: any cleanup that must run "no matter how this async command exits" needs an explicit
  signal listener raced against the awaited operation — placing cleanup code after a bare
  `.await` only covers the success path, not process-level interrupts.

## 2026-07-07 — Windows CI failed on a module that "obviously" only runs on the remote host

- **Symptom**: the `build-preview-assets` Windows job failed with
  `error[E0432]: unresolved import tokio::net::UnixStream` in `ssh/remote_client.rs`, even though
  the preceding commit had already added `#[cfg(unix)]`/`#[cfg(windows)]` gating for the *local*
  agent side (`ssh::bind_local_listener`, `agent::LocalListener`) and passed local verification.
- **Root cause**: `remote_client.rs` implements the *remote* side of a session (it dials the
  forwarded socket via `UnixStream`), and the remote host is always assumed Linux/macOS by design
  — so it seemed safe to leave unconditional. But the `shine` binary is one cross-compiled artifact
  that must *compile* for every target it ships on, regardless of which side of a session that
  particular binary instance will ever actually play. A Windows build still needs
  `shine local download/upload/status` to type-check even though nothing will call it as a remote
  in practice yet. This repo's sandboxed dev environment cannot run `cargo check --target
  x86_64-pc-windows-msvc` to completion at all (an unrelated transitive dependency, `aws-lc-sys` via
  `reqwest`, needs the real MSVC/Windows SDK), so this gap wasn't caught before pushing — only real
  Windows CI surfaced it.
- **Fix**: gated `mod remote_client;` itself behind `#[cfg(unix)]`, and gave
  `handle_local_download`/`handle_local_upload`/`handle_local_status` in `ssh/mod.rs` a
  `#[cfg(not(unix))]` stub returning a clear "Windows is local-side only" error, so the binary
  still compiles (and fails loudly at runtime, not compile time) on Windows.
- **Rule**: when adding platform-specific code to a binary that ships cross-platform, gate by
  *what the code assumes about the runtime host*, not by *which conceptual role the current
  feature work is scoped to* — and treat any target you cannot locally `cargo check` end-to-end
  as unverified until real CI confirms it, even after careful manual reasoning.

## 2026-07-06 — `shine upgrade` prompted for sudo even when nothing needed root

- **Symptom**: every `shine upgrade` run asked for the sudo password for the managed split-DNS
  item, even when the resolved.conf.d file already matched the desired content and the item
  reported `already installed` immediately after.
- **Root cause**: the admin-authorization gate in `run_managed_for_os` decided whether to prompt
  purely from each item's static `requires_admin` manifest flag, before the driver's `apply` ever
  checked whether a write was actually needed. The read-only "already converged" comparison
  already existed inside `apply_split_dns`/`apply_managed_file`, but only ran *after* the prompt.
- **Fix**: added `SystemDriver::is_up_to_date` (read-only, no privilege required) that reuses the
  same desired-vs-current comparison, and call it per admin-required item before `authorize_admin`
  so the prompt is skipped when every such item is already converged.
- **Rule**: a privilege-escalation prompt must be gated on "will this action actually change
  anything," not on "is this category of action normally privileged" — compute the cheap
  read-only diff first.

## 2026-07-06 — Embedded Git progress overwhelms command-level results

- **Symptom**: `shine update --pull` printed Git transfer plumbing, fetch refs, fast-forward
  details, skipped directories, and Shine's update report as one visually noisy stream.
- **Fix**: capture successful pulls and summarize commit range plus short file stats; retain raw
  progress for verbose mode and always surface captured diagnostics on failure.
- **Rule**: wrapped tools should expose task-level outcomes by default and reserve transport-level
  progress for verbose output, without hiding failure diagnostics.

## 2026-07-05 — Managed update detection should explain the pending change

- **Symptom**: split-DNS changes were detected, but update output only said `converge` and did
  not show which recorded values would change.
- **Fix**: derive structured field differences from the recorded and desired receipts and show
  them in both `shine update` and `shine sys info`.
- **Rule**: desired-state checks should return actionable differences, not only a boolean, when
  the manifest already contains enough safe metadata to explain the change.

## 2026-07-05 — Info diff and update must resolve the same effective preset

- **Symptom**: `shine update` reported an embedded shell preset update while
  `shine info proxy/setproxy --diff` said there were no content differences.
- **Root cause**: update rendered the newly embedded template, but info rendered the stale
  extracted copy under `~/.shine/presets/`; info status also omitted template comparison.
- **Fix**: resolve expected shell bytes from embedded assets unless external presets mode is
  active, and reuse update's shell rows for info status.
- **Rule**: status and diff surfaces must share effective-source selection with the operation
  that will apply the update.

## 2026-07-05 — Managed sys resources need desired-state update detection

- **Symptom**: changing split-DNS variables in an overlay `shine.env.toml` was invisible to
  `shine update`, leaving no reliable path from the configuration change to `shine upgrade`.
- **Root cause**: update listing only inspected shell and app content; sys receipts already held
  the applied domain and servers but were never compared with current desired values.
- **Fix**: compare the desired built-in resource receipt with `sys-manifest.toml`, report stale
  managed resources, and let the existing upgrade convergence replace the receipt.
- **Rule**: every manifest-tracked subsystem included in global upgrade must expose an equivalent
  read-only desired-state check to global update.

## 2026-07-05 — Template update checks only see variables used by the template

- **Symptom**: changing `PROXY_NO_PROXY` in an overlay `shine.env.toml` did not make
  `shine update` report the installed Unix `setproxy` command as stale.
- **Root cause**: the Unix proxy template declared template support but hard-coded `no_proxy`, so
  changing `PROXY_NO_PROXY` did not change the rendered output that update detection compares.
- **Fix**: render `PROXY_NO_PROXY` into the Unix proxy script and cover env-only changes in the
  shell update-status tests.
- **Rule**: every documented preset environment setting must occur in the rendered template;
  update detection is content-based and cannot observe unused variables.

## 2026-07-05 — Typed config readers must not silently discard invalid entries

- **Symptom**: `{ value, description }` entries in `shine.env.toml` appeared valid but had no
  effect, while the same shape worked in `config.toml` `[env]`.
- **Root cause**: the override reader parsed a generic TOML table and used `filter_map` to keep
  strings, silently dropping every other value instead of applying the shared env value model.
- **Fix**: parse each override entry as the same string-or-detailed env type, merge optional
  descriptions by layer, and report invalid entries with their key and file path.
- **Rule**: typed configuration readers must reject unsupported value shapes explicitly; never
  use filtering as validation when a dropped entry changes user-visible behavior.

## 2026-07-05 — Filtered overlay lookup must search the merged preset namespace

- **Symptom**: `shine upgrade` failed with `app preset category not found: JetBrains` when the
  category existed in the external presets directory but not in the configured overlay.
- **Root cause**: filtered app-category discovery treated a miss in either the base or overlay
  directory as fatal instead of checking whether the category existed in their union.
- **Fix**: defer the not-found error until base and overlay category names have been merged.
- **Rule**: filtered preset lookup must resolve against the merged namespace; a category only
  needs to exist in one source, while matching overlay paths still take precedence.

## 2026-07-04 — `requires_admin` dropped from manifest entries broke sudo uninstall

- **Symptom**: CI failure in `install_then_uninstall_roundtrip`; uninstall of
  `/etc/docker/daemon.json` went through the unprivileged path and failed.
- **Root cause**: Copy-strategy manifest entries didn't persist `requires_admin`, so
  `uninstall_app_entry` couldn't route to the admin-aware removal path.
- **Fix**: `70ee910` — persist `requires_admin` on `AppEntry`.
- **Rule**: manifest fields are load-bearing across install → uninstall; every flag that affects
  removal must survive the TOML round-trip, with a roundtrip test.

## 2026-07-04 — In-process locks don't serialize nextest tests on real system paths

- **Symptom**: intermittent races between tests touching `/etc/docker/daemon.json` once
  uninstall actually removed it via sudo.
- **Root cause**: nextest runs each test as its own OS process; the in-process `env_lock()`
  mutex can't serialize two processes on one real file.
- **Fix**: `fbd9c55` — cross-process advisory lock (`$TMPDIR/shine-admin.lock`, `create_dir` as
  mutex, stale-lock reclaim) around privileged fs mutations, plus a second lock held for the
  full body of whole-category install/uninstall tests.
- **Rule**: anything shared across test *processes* (real paths, global system state) needs a
  cross-process lock, not a `Mutex`.

## 2026-07-04 — Project config must inherit global settings

- **Symptom**: project-local configs silently lost global settings.
- **Root cause**: project config was read standalone instead of layering over the global one.
- **Fix**: `a5aed62` (inheritance) + `0936f05` (scheduled cleanup of the legacy project file).
- **Rule**: project config is an overlay over global config, not a replacement; removing legacy
  state should be scheduled/graceful, not abrupt.

## 2026-06-21 — PowerShell profile BOM must be preserved

- **Symptom**: rewriting a PowerShell profile corrupted/moved the UTF-8 BOM.
- **Fix**: `81244f8` — detect and preserve a leading BOM when editing profile files.
- **Rule**: on Windows, treat the BOM as part of the file-start invariant when splicing content.

## 2026-06-18 — External presets mode must fall back to embedded templates

- **Symptom**: sys profile installation failed when the external presets dir lacked a template.
- **Fix**: `5606438` — fall back to the embedded copy.
- **Rule**: external/overlay presets extend embedded assets; a missing external file degrades to
  embedded content, never to an error.

## 2026-06-16 — Version checks must be non-fatal and rate-limit aware

- **Symptom**: GitHub API failures/rate limits broke or spammed unrelated commands.
- **Fix**: `605fdd8` (tolerate check failures) + `f033a25` (cache rate-limit cooldown per auth
  mode alongside the 24 h version cache).
- **Rule**: background nicety features must never fail the user's primary command; cache
  negative results (cooldowns), not just positive ones.

## 2026-06-13 — zsh completions need explicit compinit handling

- **Symptom**: installed zsh completions didn't activate reliably.
- **Fix**: `fc410ab` (initialize the zsh completion system) + `f7eac5a` (harden compinit
  registration).
- **Rule**: don't assume the user's `.zshrc` runs `compinit`; shine's completion install must
  ensure registration itself, idempotently.

## 2026-06-13 — Global test state races under parallel test runs

- **Symptom**: two intermittent test failures from an `OVERLAY_DIR` race and scattered env-var
  mutexes.
- **Fix**: `3f7ac41` — unified shared `crate::test_support::env_lock()` across all modules.
- **Rule**: one shared lock for one shared resource; per-module locks over the same global
  resource are a race waiting to happen.

## Release practice — count from the latest stable tag

When cutting a release, always diff against the latest stable `v*` tag
(`git tag --list 'v*' --sort=-version:refname | head -1`), never `preview` and never
`git describe --tags --abbrev=0` alone (it can resolve to `preview`).
