# Lessons Learned

Dated entries mined from real bugs. Format: **symptom → root cause → fix → rule**.
Newest first. Cite the fixing commit. Add an entry whenever a bug's cause was non-obvious.

## 2026-07-16 — OSC 11 reply leaks in full because the tty stayed in canonical mode

Follow-up to the 2026-07-14 entry. That fix (total-deadline `poll(2)` read in
`cli/src/theme/osc.rs`) cured the *fragmented-tail* leak but left a second, distinct one that
reproduces on Ghostty: the **whole** reply leaks — `\033]11;rgb:ffff/ffff/ffff\033\\` with the
leading `\033]` intact, not just the tail.

- **Symptom**: every fresh Ghostty login shell prints `^[]11;rgb:ffff/ffff/ffff^[\` before the
  first prompt. The full sequence (leading `\033]` present) means the read loop consumed **zero**
  bytes of it, unlike 2026-07-14 where the consumed head made `\033]` absent.
- **Root cause**: `EchoGuard::disable` cleared `ECHO` but **not `ICANON`**, so the tty stayed in
  canonical (line) mode during the query. An OSC 11 reply contains **no newline**, and in
  canonical mode the line discipline withholds a newline-less line from `read`/`poll` until a
  newline arrives — so `poll` never signals readable, the loop times out at 200 ms having read
  nothing, restores the tty, and the buffered reply flushes into the next prompt. Disabling echo
  is irrelevant to *this* leak: the bytes are withheld regardless of echo.
- **Why it wasn't caught**: the `matrix_*` tests drive the read loop over a `UnixStream` socket
  pair, which has **no line discipline**, so canonical mode never applied in test. Measured with a
  real terminal: a cbreak probe (`tty.setcbreak`) returned the full 25-byte reply in-window with
  no leak; the same probe leaving `ICANON` on returned 0 bytes and leaked. Slower terminals (Apple
  Terminal/iTerm2) happened to mask it during the PRD's manual check — the reply raced in before
  the timeout on those, but Ghostty's timing exposed it every time.
- **Fix**: `EchoGuard` → `TtyQueryGuard`, which clears `ECHO | ICANON` and sets `VMIN=1`/`VTIME=0`
  (required because in non-canonical mode those `c_cc` slots alias `VEOF`/`VEOL`; leaving them
  inherits `VEOF`=4 as `VMIN` and stalls fragmented reads until 4 bytes accrue). Regression test
  `read_loop_reads_newline_free_response_through_pty` uses a real `openpty` pair (which *does* have
  a line discipline) so canonical mode is exercised — the exact gap the socket-pair tests had.
- **Rule**: to read a terminal-control reply you must put the tty in **non-canonical** mode
  (`ICANON` off), not merely turn echo off — control replies carry no newline, and canonical mode
  will hold them hostage until one appears. And a fixture without a line discipline (socket pair,
  pipe) cannot test tty-mode behavior; use a real pty (`openpty`).

## 2026-07-15 — `env set`/`encrypt` silently wrote a value an override file kept shadowing

- **Symptom**: `shine env encrypt --from KEY` (or `env set KEY value`) reported success and
  wrote into `config.toml [env]`, but the effective value of `KEY` never changed when an
  env override file (`shine.env.toml` — global, active overlay, or project) already defined
  it.
- **Root cause**: override files are merged strictly above **both** `config.toml [env]`
  layers by design (`config/load.rs`: *"Environment override files deliberately sit above
  both TOML layers"*), but `env set`/`encrypt`/`delete` had no awareness of that and always
  wrote into whichever `config.toml` was active for the cwd. The write succeeded but was
  dead on arrival — the override file kept winning at read time.
- **Why precedence itself was not changed**: per
  [ADR 0010](decisions/0010-git-managed-overlay.md), a shine-managed Git overlay exists
  specifically so a value authored once on the maintaining device reliably wins on every
  consuming machine, overriding whatever stale value a local `config.toml` holds. Flipping
  overlay-vs-config.toml precedence would defeat that guarantee for every other key.
- **Fix**: `Config` now tracks, per env key, which override file (if any) currently supplies
  its effective value (`Config::env_override_sources` / `env_override_source()`, populated
  in `apply_*_env_override`). `env set`/`encrypt`/`delete` consult it before writing: refuse
  with a clear "this write would have no effect, X currently wins" error by default; `--force`
  writes directly into that override file instead (`write_env_override_entry`, comment- and
  description-preserving via the same `utils::migration::sync_table` `Config::save()` uses),
  and warns loudly when the winning file is the shine-managed overlay mirror, since that
  write is discarded on the next `shine pull`.
- **Rule**: a `set`/`delete`-shaped command must never report success for a write that a
  higher-precedence layer will keep shadowing — either make the write land where it's
  actually effective, or refuse and say why.

## 2026-07-14 — OSC 11 response tail leaks when the reply arrives fragmented

Supersedes the 2026-07-13 entry that blamed tty echo. That diagnosis was wrong and its fix
(`6f23c6b9`) does not work — the bug still reproduces on the latest build. The cause below is
measured, not inferred.

- **Symptom**: after `shine sys init` on Ubuntu, opening an SSH session displays a string such as
  `11;rgb:0f0f/1616/1010` at the prompt. Note the missing leading `\033]` — that omission is the
  clue, not a typo.
- **Root cause**: the managed Unix profile's OSC 11 read loop
  (`presets/sys/{ubuntu,macos}/profile.pre.sh`) gives the first byte 150 ms but drops the
  **inter-byte** timeout to 10 ms (`read_timeout="0.01"`). When the reply arrives fragmented — a
  normal outcome over SSH — the loop times out mid-response, `break`s holding only the 2 bytes it
  read (`\033]`), and restores the tty. The remaining bytes land **after** echo is back on and are
  echoed verbatim. The visible text is the response's *tail*, which is exactly why `\033]` is
  absent: the loop consumed it.
- **Why the 2026-07-13 fix failed**: `stty -echo` only covers the loop's *duration*, but the leak
  happens *after* the loop restores the tty. (Bash's `read -s` already disabled echo per-read, so
  that patch's delta was near zero to begin with.)
- **Measured** with `pty.fork()` driving the real loop against synthetic OSC replies, on **both**
  affected platforms — Ubuntu/bash 5.3.9 (the `read -n` branch) and macOS/zsh 5.9 (the `read -k`
  branch). Both reproduce identically; the only difference is the timeout return code
  (bash `142` = 128+SIGALRM, zsh `1`):

  | reply arrival | consumed | elapsed | tail leaked |
  |---|---|---|---|
  | whole packet | 25 B (full) | 1 ms | no |
  | fragmented, 50 ms gap | **2 B = `\033]`** | 10 ms | **yes** |
  | fragmented, 5 ms gap | 25 B | 6 ms | no |
  | no reply at all | 0 | 150 ms | no (silent skip — correct) |

  Only the fragmented-beyond-10 ms case is broken; the no-reply path is healthy. `read -k -t`
  (zsh) and `read -n -t` (bash) show no semantic difference here — this is the loop's timeout
  policy, not a shell quirk.
- **Fix**: landed per [`docs/terminal-theme-sync-prd.md`](../terminal-theme-sync-prd.md) §6.2 — the
  shell-only OSC read loop was replaced entirely by `shine theme sync` (`cli/src/theme/osc.rs`),
  which reads against a single total deadline via `poll(2)` with no inter-byte timeout to violate.
  `presets/sys/{ubuntu,macos}/profile.pre.sh` now only decides whether to call the binary; it
  contains no OSC/PTY/RGB parsing of its own.
- **Rule**: read terminal-control responses against a **total deadline until the terminator**,
  never with a tight inter-byte timeout — a partial read that then restores the tty converts the
  remainder into user input. Disabling echo is necessary but nowhere near sufficient. The only way
  to remove the race entirely is to not query from the remote at all (pass the value in over the
  session instead).
- **Meta-rule**: this entry was wrong for a day because the cause was *inferred from a plausible
  story* rather than measured. When a terminal/pty bug's cause is not obvious, reproduce it in a
  pty on the affected platform before writing the lesson down.

## 2026-07-13 — `shine local` now runs external tools with wire-derived argv

- **Symptom / risk**: rewriting `shine local` to spawn `rsync`/`scp` locally (ADR 0011) moved a
  process-execution boundary into the transfer agent. The two path strings in a `Transfer` frame
  (`remote_spec`, `local_spec`) come from the remote over the tunnel and are **untrusted** — the
  session token that gates them leaks via `ps eww`/`/proc/<pid>/environ` on the remote (see the
  2026-07-09 token-leak lesson), so a co-tenant on the remote can forge a `Transfer`. A naive
  implementation lets a `remote_spec` of `-oProxyCommand=evil` or an rsync `-e …`/`--rsync-path=…`
  reach an *option* position and achieve arbitrary command execution on the **local** machine.
- **Root cause**: rsync/scp interpret leading-`-` operands as options, and rsync's `-e`/`--rsh`
  (and `-o ProxyCommand`) are command-execution vectors. Any wire string that lands in an option
  slot — or is word-split by a shell — is a local RCE.
- **Fix** (`cli/src/ssh/agent.rs`): spawn argv-only, never `sh -c`; emit the remote path only as
  the single token `<host>:<remote_spec>` after a `--` separator (a `-`-leading spec becomes an
  inert `host:-…`); anchor local operands to the session dir and `./`-prefix any dash-leading one;
  build the ssh reconnection `-e`/`-o` string **solely** from the local `SessionContext` (never the
  wire); reject control chars in `remote_spec`; expand wire paths tilde-only (not `${VAR}`); expand
  local globs with the `glob` crate, not a shell. ControlMaster reuse also avoids an *invisible*
  local password prompt (a second connection prompting on the local terminal while the user watches
  the remote one). Covered by `build_transfer_argv` injection unit tests.
- **Rule**: when an agent shells out with any wire-supplied value, keep that value out of every
  option position — argv only, `--` before operands, protocol prefixes (`host:`) that can't be read
  as flags, and connection/transport options sourced only from local, trusted state.

## 2026-07-11 — Surge's `external-resource update` never covers URL-based Modules

- **Symptom**: after `shine upgrade` correctly rewrote the installed `custom-rules.sgmodule`
  (served via `shine serve` at `~/.shine/http/app/surge/`), Surge kept applying the old rules —
  the `post_upgrade` hook (`surge-cli external-resource update all` + `surge-cli reload`) appeared
  to run successfully but had no effect.
- **Root cause**: `surge-cli external-resource list` only enumerates rule-sets and MITM hostname
  lists (`type = ruleset`) fetched via `RULE-SET,https://...` — a Module added via URL is not
  tracked as an "external resource" at all, so `external-resource update all` is a silent no-op
  for it. `surge-cli reload` only re-parses the module content Surge already has cached from its
  last fetch; there is no `surge-cli` command that forces a URL-based Module to re-fetch on
  demand (confirmed against `surge-cli --help`).
- **Fix**: dropped the ineffective `external-resource update all` step from
  `presets/app/surge/shine.toml`'s `post_upgrade` (kept `reload`, which is still needed to apply
  other profile changes). No shine-side workaround exists to force a Module URL re-fetch; on
  macOS the practical fix is for the user to reference the module by local file path
  (e.g. `~/.shine/http/app/surge/custom-rules.sgmodule`) in Surge's own profile instead of a
  `shine serve` HTTP URL, since `reload` re-reads local files immediately with no caching layer.
- **Rule**: a post-upgrade hook exiting 0 does not mean it had any effect — verify what a
  third-party CLI's subcommand actually covers (e.g. via its own `list`/`--help` output) before
  assuming it refreshes the specific resource an app preset just changed.

## 2026-07-11 — CRLF↔LF differences made `shine sys` re-install report spurious updates

- **Symptom**: tracing a user question — *install via `shine sys init`, edit config on a Windows
  host (CRLF-prone), edit the preset on macOS, re-install* — revealed that a pure line-ending
  difference is detected as a real change on the whole sys path. A CRLF copy of a loader file
  under `~/.shine/profile/` fails `active == template` (raw `&[u8]` compare in `sys/profile.rs`)
  and drops into the three-way merge, where `split_profile_lines` keeps each trailing `\r`, so
  every line "differs" → **updated / needs-action / conflict** every run. Separately the sentinel
  idempotency check in `sys/profile_blocks.rs` compares the extracted block against an LF
  `pre_block`, so a CRLF profile misses the short-circuit and gets rewritten (silently normalizing
  the user-owned file to LF).
- **Root cause**: every comparison on the sys path was byte-exact with **no** line-ending
  normalization (and `hash_content` is FNV over raw bytes), so `\r` flips every `==`. Compounding
  it, nothing pinned the embedded template's endings — a build host with `core.autocrlf=true`
  could embed CRLF, making even a clean checkout diff against an installed LF copy.
- **Fix**: (1) `.gitattributes` pins `presets/** text eol=lf` so the rust-embed'd template is
  byte-deterministic. (2) new `install_core::normalize_eol`/`eol_eq`; the sys reconciliation
  (`sys/profile.rs`) normalizes `active`/`base`/`template` at read and gates `git merge-file` off
  when any on-disk input was non-LF (falling back to the pure-Rust merge over normalized bytes);
  the sentinel idempotency check (`sys/profile_blocks.rs`) compares blocks ending-agnostically
  (trimming the trailing break, since `extract_block_with_newline` only reattaches `\n`). When only
  endings differ, both paths now report no change and leave the file's bytes untouched. The
  invariant-protected `sentinel::remove_block_*` styles were left unchanged — only the comparison
  layer normalizes.
- **Rule**: when reconciling installed files that a user (or their editor) may re-save, byte-exact
  `==`/content-hash compares silently conflate a formatting-only difference with a real edit —
  normalize line endings before comparing, and pin embedded/template endings so the baseline is
  deterministic across build and checkout environments.

## 2026-07-09 — `agent_handle.abort()` didn't actually protect in-flight `shine ssh` transfers

- **Symptom**: a follow-up review of the Ctrl-C cleanup path in `ssh/mod.rs` found that
  `agent_handle.abort()` — called right before `remove_dir_all(&session_dir)` and (for a nonzero
  `ssh` exit status) `std::process::exit(code)` — looked like it protected an in-flight `PutFile`/
  `GetFile` transfer from being cut off mid-copy. It doesn't: `agent_handle` is the `JoinHandle`
  for `LocalListener::serve`'s *accept loop* only. Each individual connection is handed off to its
  own detached `tokio::spawn` inside `spawn_connection`, whose `JoinHandle` was discarded and never
  tracked anywhere. So `abort()` only ever stopped new connections from being accepted — an
  already-running transfer kept executing as an orphaned background task with nothing in
  `handle_ssh` aware of it, and `std::process::exit(code)` (a hard process termination that skips
  Rust's normal unwind/drop machinery entirely) could cut it off before its own error-path cleanup
  (removing a partial temp file) ever ran.
- **Root cause**: "spawn a task and drop the handle" is a common, usually-harmless Tokio pattern
  for fire-and-forget work, but it silently breaks any later code that assumes it can wait for or
  cancel that work — there was no data structure connecting `handle_ssh`'s shutdown sequence to the
  connection tasks `spawn_connection` created.
- **Fix**: added `agent::ConnectionTasks` (`Arc<tokio::sync::Mutex<tokio::task::JoinSet<()>>>`),
  threaded through `LocalListener::serve` and `spawn_connection` so every connection task is
  tracked instead of detached. `handle_ssh` now calls `agent::drain_connection_tasks(&connection_tasks,
  CONNECTION_DRAIN_GRACE_PERIOD)` (a 5s bounded wait) after `ssh_run` resolves and before removing
  the session directory — since the SSH tunnel is already gone by that point, any genuinely
  in-flight transfer's next socket read/write fails almost immediately and its existing error-path
  cleanup gets to run to completion inside that grace period, rather than being abandoned.
- **Rule**: a bare `tokio::spawn(...)` with the `JoinHandle` immediately dropped is a red flag
  during review whenever the surrounding code later does anything time-sensitive (shutdown,
  cleanup, `process::exit`) — trace whether "this task is running" is ever visible to the code that
  assumes it can wait for or interrupt it.

## 2026-07-09 — `shine local` upload's `PutFile.filename` allowed a path-traversal write

- **Symptom**: a Rust code review of the SSH transfer feature found that `agent::resolve_target_path`
  (`ssh/agent.rs`) joined the wire-supplied `PutFile.filename` directly onto the resolved
  destination directory with `candidate.join(filename)` and no validation. The only gate on a
  `PutFile` request is a per-session token, but that token is exposed as plaintext in the wrapped
  remote command's argv/environ (`env SHINE_SSH_TOKEN=<token> sh -c ...` in `ssh/mod.rs`), which
  any other local user on the remote host can read via `ps eww`/`/proc/<pid>/environ`. A forged
  client that reads the token could dial the forwarded socket directly and send
  `filename: "../../.ssh/authorized_keys"`, writing outside `session_local_dir` on the machine
  running `shine ssh` — an arbitrary local file write gated only by OS file permissions.
- **Root cause**: `filename` is documented as "basename of the remote source path," and the
  honest client (`remote_client.rs`) does derive it via `Path::file_name()` — but the server side
  never re-validated that invariant on the wire. `dir_transfer.rs`'s tar-extraction path had an
  equivalent, well-tested symlink/traversal check (`symlink_target_is_safe`); the newer
  single-file `PutFile` path did not get the same treatment.
- **Fix**: added `ensure_single_path_component` in `ssh/agent.rs`, called at the top of
  `resolve_target_path`, that rejects any `filename` which isn't exactly one
  `std::path::Component::Normal` — so an absolute path, a `..` component, or an embedded
  separator is rejected with a clear error before it ever reaches `Path::join`. `dest_hint` is
  intentionally left unrestricted (it's a legitimate explicit destination the caller chose), only
  the auto-derived `filename` is constrained. Covered by unit tests in `ssh/agent.rs` and two new
  integration tests in `ssh/integration_tests.rs` that send a raw forged `PutFile` (bypassing
  `remote_client`) with a traversal/separator filename and assert it's rejected.
- **Rule**: any field in a wire protocol that's *documented* as "always a basename" or similar
  must be *enforced* as such at the point it's consumed, not just produced correctly by the one
  trusted client implementation — a session token alone is not sufficient authorization once it
  can leak through the host's own process table.

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
