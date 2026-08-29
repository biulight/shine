# Lessons Learned

Dated entries mined from real bugs. Format: **symptom → root cause → fix → rule**.
Newest first. Cite the fixing commit. Add an entry whenever a bug's cause was non-obvious.

## 2026-08-30 — Executor-side choices expanded reviewed lifecycle work

- **Symptom**: a reviewed App upgrade could still ask to remove stale files, and aggregate upgrade
  could synchronize the composed Sys profile even though neither mutation was fixed by the Plan.
- **Root cause**: lifecycle options and shared-state convergence were partly decided inside CLI
  execution adapters after planning instead of being bound to the reviewed request and steps.
- **Fix**: make `--prune-stale` the only stale-removal input, disable executor-side stale prompts,
  batch and prevalidate aggregate lifecycle Plans, and remove implicit Sys profile sync from
  untargeted upgrade.
- **Rule**: after Plan review, execution may request administrator authorization but may not add a
  target, resource, permission, or cleanup action; every such choice belongs in the reviewed Plan.

## 2026-08-29 — In-memory runtime tests still need host-native captured paths

- **Symptom**: the full Windows `shine-core` suite failed three in-memory App, Shell, and Sys
  lifecycle tests that passed on Unix hosts.
- **Root cause**: `InMemoryHost` abstracts filesystem I/O, but captured `PathBuf` values, native
  shell defaults, and Shell launcher names still follow the compiled host. The tests used
  Unix-rooted homes, inherited the Windows PowerShell default while simulating Zsh, and assumed a
  suffix-free launcher.
- **Fix**: derive isolated homes from the host-native temporary directory and resolve the expected
  launcher through the same platform helper as production code; explicitly select Zsh in the Sys
  test whose requested behavior is Zsh profile composition.
- **Rule**: in-memory host tests must use host-native absolute paths for captured runtime context
  and shared platform helpers for host-specific artifact names. Tests that simulate a target shell
  or platform must set it explicitly instead of inheriting the compiled host's default.

## 2026-08-29 — Cross-platform validation must not use host-native path parsing

- **Symptom**: Windows CI rejected every built-in App preset destination and reported a missing-file
  fixture as invalid metadata before it reached the missing reference.
- **Root cause**: static validation simulated macOS, Linux, and Windows metadata with a fixed Unix
  `/validation-home`, then used the Windows host's `Path::is_absolute()` and component parser.
  Target-platform paths were therefore interpreted using the runner's path grammar.
- **Fix**: use a native absolute synthetic home derived from the canonical preset root for runtime
  simulation, and validate declared absolute, relative, and parent-path syntax lexically across both
  separator styles.
- **Rule**: validators that evaluate multiple target platforms on one host must separate target
  path grammar from host filesystem paths; use host-native paths only for captured runtime context.

## 2026-08-29 — Platform-selected preset tests must follow the selected source

- **Symptom**: the Windows Rust test job failed while checking that Shell info rendered expected
  content from the embedded proxy preset instead of a stale extracted source.
- **Root cause**: after inspection moved to `CoreRuntime`, the test selected the native Shell preset
  at runtime but still wrote `set_proxy.sh` and asserted its Unix assignment syntax; Windows
  correctly selected `set_proxy.ps1` and rendered PowerShell syntax.
- **Fix**: discover the selected source path from runtime inspection, seed the stale file there,
  and assert the cross-platform rendered proxy value rather than one shell's assignment syntax.
- **Rule**: when production metadata selects a platform variant, tests must seed and inspect that
  selected variant and assert shared semantics unless syntax itself is the behavior under test.

## 2026-08-29 — A host boundary must remove ambient compatibility entry points

- **Symptom**: the Phase 2 migration routed normal CLI commands through `CoreRuntime`, but Sys
  preflight and preset validation could still observe the real filesystem, while public legacy
  launcher and manifest methods could mutate state without a host.
- **Root cause**: boundary tests checked that selected CLI files were deleted or mentioned Core,
  but did not reject no-host public APIs or direct `Path`/`std::fs` calls inside Core-owned flows.
- **Fix**: require hosts for validation and manifest persistence, capture cwd explicitly, move
  external/overlay discovery into shared host-backed runtime bootstrap, materialize Sys inputs only
  for authorized execution, hide test-only launcher helpers, and add behavioral boundary tests.
- **Rule**: dependency inversion does not forbid Core infrastructure from observing a real host. It
  requires shared bootstrap and resource operations to observe through explicit ports, while domain
  execution consumes captured inputs and exposes no ambient or no-host compatibility entry point.

## 2026-08-28 — Native-shell integration tests must assert native syntax

- **Symptom**: the Windows Rust test job rejected the profile written by `shell completion install`
  even though it contained the correct PowerShell completion registration.
- **Root cause**: the test let `ShellType::default()` select the native shell but always searched for
  Bash/Zsh's `COMPLETE=<shell> shine` assignment; PowerShell intentionally uses
  `$env:COMPLETE = 'powershell'`.
- **Fix**: select the expected completion marker from the native shell type before inspecting the
  installed managed profile.
- **Rule**: a test that deliberately exercises a platform-selected implementation must assert that
  implementation's syntax, not interpolate its name into one platform's syntax.

## 2026-08-27 — Generated documentation checks must ignore checkout line endings

- **Symptom**: the Windows Rust test job reported both generated preset-capability blocks as stale
  even though every row matched the runtime-derived output.
- **Root cause**: Git's Windows checkout presented the Markdown blocks with CRLF endings while the
  in-memory generator used LF, and the conformance test compared the strings byte-for-byte.
- **Fix**: normalize the checked block before comparison and preserve its existing CRLF or LF style
  when the opt-in update mode replaces it.
- **Rule**: generated documentation conformance checks must treat checkout-only line-ending changes
  as equivalent, and generators that edit a block must not introduce mixed endings into its file.

## 2026-08-27 — Serialized path assertions must compare decoded values

- **Symptom**: the Windows Rust test job failed after `shine preset link` correctly persisted an
  external preset directory in `config.toml`.
- **Root cause**: the test searched the raw TOML text for the native Windows path, but TOML string
  syntax escapes each backslash, so the serialized text intentionally differs from `Path::to_str()`.
- **Fix**: parse the saved TOML and compare the decoded `presets_dir` as a `PathBuf` against the
  canonical linked directory.
- **Rule**: cross-platform persistence tests must compare decoded semantic values; inspect raw text
  only when the serialization format itself is the behavior under test.

## 2026-08-27 — Background services must pin the resolved Shine directory

- **Symptom**: a service installed while using `--config-dir` or `SHINE_CONFIG_DIR` could later
  serve the default `~/.shine/http/` tree instead of the selected tree.
- **Root cause**: the service manager starts Shine outside the installing shell and does not retain
  its working directory or ad-hoc environment.
- **Fix**: every launchd, systemd, and Windows scheduled-task command records the resolved
  `shine_dir` as an explicit global `--config-dir` argument.
- **Rule**: persistent registrations must serialize all state-location inputs needed to reproduce
  the installed behavior; never rely on the installer process environment surviving a login or
  reboot.

## 2026-08-27 — Unix grouping exposed macOS-only app presets on Linux

- **Symptom**: the built-in Surge preset appeared in App listings on Linux and Windows and could
  install files beneath a meaningless Surge-style destination before its macOS-only reload hook
  failed.
- **Root cause**: runtime selection collapsed every non-Windows host into `unix`, so metadata could
  not distinguish macOS from Linux and relied on destination paths or external commands to fail.
- **Fix**: add exact `macos`, `linux`, and `windows` selectors, retain `unix` as a compatibility
  group, validate all exact OS branches, and declare Surge's destination only for macOS.
- **Rule**: encode OS availability in metadata and filter before application lifecycle effects; never use a
  platform-specific path or hook failure as the availability boundary.

## 2026-08-22 — Bootstrap sudo prompts lost their owning item

- **Symptom**: `shine sys bootstrap` could show a sudo password prompt without identifying which
  selected software caused it.
- **Root cause**: installer output is buffered until completion, including output from install
  scripts that invoke `sudo` themselves.
- **Fix**: announce each missing item from the bootstrap runner before starting its installer.
- **Rule**: a bootstrap action must identify its owning item before any authorization interaction;
  do not rely on installer output that the runner buffers until completion.

## 2026-08-21 — External-code errors hid the active trust boundary

- **Symptom**: `shine sys bootstrap` reported external profile or legacy bootstrap code without
  explaining which external preset layers created the trust boundary, which file would execute, or
  where the global-only permission had to be configured.
- **Root cause**: the permission check correctly treated the overlay as an external trust boundary,
  but its one-line diagnostic described only the blocked item and flag.
- **Fix**: identify the executable integration kind, every active external preset layer, and the
  resolved global config path; present permission and keep-blocked remediation as separate actions,
  and state that bootstrap preflight made no system changes.
- **Rule**: permission errors must name both the blocked capability and the active trust boundary,
  then point to the configuration layer that can actually grant access. Alternative remediation
  must remove every active trust boundary, not imply that removing only one of several layers is
  sufficient.

## 2026-08-20 — Redundant detail flags should remain composable

- **Symptom**: `shine update <TARGET> --verbose` failed during argument parsing even though the
  target form was already detailed and accepting the flag would not make the request ambiguous.
- **Root cause**: the targeted update implementation intentionally bypassed the global verbose
  listing, and that internal no-op was exposed as a Clap argument conflict.
- **Fix**: accept `--verbose` with a target as a compatibility no-op while retaining the meaningful
  conflict between targeted checks and `--refresh-release`.
- **Rule**: when a shared CLI option is redundant in a more specific mode, accept it if its meaning
  is already satisfied; reserve argument conflicts for combinations with incompatible semantics.

## 2026-08-20 — Shell source presence was mistaken for command installation

- **Symptom**: `shine shell install utils/shine-env-export` was parsed as a nonexistent category,
  completed successfully with `0 linked`, and could not express the user's intent to activate only
  one independent command from `utils`.
- **Root cause**: lifecycle parsing stopped at categories, while status treated extracted source
  files and full external snapshots as installation evidence even though the manifest and launchers
  were already command-granular.
- **Fix**: accept explicit `category/command` install and uninstall targets, update manifest entries
  incrementally at that scope, treat category source material as a shared cache rather than an
  activation receipt, filter info through command installation status, and retain shared rendered
  files until the final referencing command is uninstalled.
- **Rule**: when deployment material is broader than the user-selected lifecycle unit, installed
  state must come from the unit's receipt or managed entry—not incidental source presence.

## 2026-08-20 — Structural preset updates were rendered as content replacements

- **Symptom**: renaming a live overlay root, for example `shineOverlay` to `shineOverlayTest`,
  correctly required command entries to be repointed but `shine update <TARGET>` presented the
  change as a whole-file diff. New files and destination moves had the same misleading path.
- **Root cause**: status collapsed content, manifest, path, and launcher differences into one
  `UpdateAvail` value; targeted and `--diff` output then unconditionally invoked the text renderer.
- **Fix**: retain structured update causes through status collection and render relocations and
  deployment fields directly. Generate a unified diff only for an actual content change, and omit
  inline output for binary, invalid UTF-8, NUL-containing, or over-256-KiB inputs. Keep command-entry
  absence, command-entry mismatch, and missing manifest state distinct; for a structural-only update,
  say explicitly that content is unchanged.
- **Rule**: update availability and content difference are not synonyms. Preserve the reason for a
  pending reconciliation step at the most actionable safe granularity, and never feed arbitrary
  bytes or unbounded files to a terminal diff.

## 2026-08-20 — Refreshed Clash rules left browser connections on their old route

- **Symptom**: after `shine app artifact apply clash-verge` refreshed every rule-provider, browser
  traffic did not follow the new rules until the browser was closed and reopened.
- **Root cause**: a provider refresh updates matching for new connections but does not reroute
  already-established HTTP/2, QUIC, WebSocket, or other long-lived mihomo connections.
- **Fix**: after every declared provider refresh succeeds, call the controller's
  `DELETE /connections` endpoint so applications reconnect and rematch; surface a close failure as
  an artifact failure and document the brief disruption to active proxied sessions.
- **Rule**: when a network-policy update promises immediate effect, distinguish refreshing policy
  data from rematching existing flows; explicitly drain old flows when the public controller API
  supports it, and disclose the disruption.

## 2026-08-20 — App update and upgrade exposed different reporting identities

- **Symptom**: `shine update` could list only one changed file while `shine upgrade` installed
  additional files, printed every physical destination and successful hook line, then counted files
  rather than the app category the user had selected.
- **Root cause**: status treated a file without its own manifest entry as not installed even when its
  category was installed, while upgrade intentionally adds new category files. Both commands then
  rendered the underlying file operations instead of sharing a category-level reporting identity.
- **Fix**: classify installable new files and destination moves as available updates, render default
  targets through the shared horizontal column presentation with one action hint, and reserve file
  destinations and successful hook details for `--diff` or `--verbose`. Keep failures, conflicts,
  user-modified warnings, and permission blocks visible. Exclude manual-generator destination moves
  because implicit upgrade intentionally preserves those manifest snapshots.
- **Rule**: update discovery and upgrade execution must agree on the full pending change set and use
  the same user-selected target as their default reporting and counting unit.

## 2026-08-19 — Shell upgrade counted deployment operations instead of updated targets

- **Symptom**: `shine update` listed two Shell targets with available updates, but `shine upgrade`
  printed only `Bin Links 1 updated` and finished with `1 updated`, even though both targets were
  brought current.
- **Root cause**: embedded/overlay preset extraction discarded its report, while the global footer
  summed template, snapshot, launcher, and profile operations. A raw source rewrite was therefore
  invisible, and one target could conversely be counted more than once when several deployment
  layers changed together.
- **Fix**: carry the pending target identities from the shared status model through upgrade,
  confirm which targets converged, print those target names by default, and reserve Bin Link and
  other deployment-layer counts for `--verbose`.
- **Rule**: status and mutation reports must use the same user-facing identity. Count each updated
  target once; expose the lower-level operations that implemented it only as diagnostic detail.

## 2026-08-19 — Ubuntu manual update guidance pointed to no-op bootstrap reruns

- **Symptom**: manual results from `shine sys update --verbose` told users to rerun bootstrap for
  `mise` and several other Ubuntu items, suggesting that doing so would update them.
- **Root cause**: the update checker reused a generic remediation even though each corresponding
  installer deliberately returns `already-installed` as soon as it finds an existing installation.
  The same audit found a `git pull` suggestion for Shine's AstroNvim clone even though bootstrap
  removes that clone's `.git` directory.
- **Fix**: keep the check manual because bootstrap state does not record installation provenance,
  remove remediation that cannot work, and give conditional source-specific guidance only where
  it is valid, such as `mise self-update` for standalone `mise.run` installs.
- **Rule**: validate every remediation against the implementation it invokes. Never recommend
  rerunning an idempotent installer as an upgrade path when its existing-install guard is a no-op.

## 2026-08-19 — Clash Verge provider refresh drifted from its composite source

- **Symptom**: renaming, adding, or removing a `rule-providers` entry in an overlay `merge.yaml`
  left the artifact refreshing the original three provider names, causing missed refreshes or 404s.
- **Root cause**: rendering parsed the effective composite source, but refresh used a separate
  hard-coded provider list copied from the inert example.
- **Fix**: derive refresh targets from the same parsed `rule-providers` mapping, encode each name as
  one URL path segment, and distinguish skipped refreshes from successful ones.
- **Rule**: when an artifact already owns and parses a declarative source, downstream actions must
  derive their targets from that parse rather than maintaining a parallel list.

## 2026-08-09 — Migration and authorization snapshots crossed lifecycle boundaries

- **Symptom**: a broker policy could hash one workspace revision but execute another revision's
  settings; separately, a legacy project config stayed unmigrated once the global schema was
  already current.
- **Root cause**: broker setup parsed the workspace through a second filesystem read, and state
  migration treated the global schema version as proof that every independently discovered
  project config had already been migrated.
- **Fix**: parse the captured workspace text used by the broker snapshot, and inspect the active
  project config for retired keys even when no global schema steps remain.
- **Rule**: immutable authorization snapshots must parse their captured bytes, and a global
  migration marker must not suppress migrations of project-local state discovered later.

## 2026-08-09 — SSH broker enrollment printed in raw mode and rejected pasted approval

- **Symptom**: a trusted broker enrollment candidate rendered as a diagonal staircase, duplicated
  its command/secret summary in a second confirmation block, and could display `y` while still
  returning `secret request rejected by the local user`.
- **Root cause**: candidate details were written to stderr before the broker paused OpenSSH and
  restored the pre-SSH canonical termios, so raw-mode LF did not return the cursor to column zero.
  The confirmation parser also compared the complete input line only with `y`/`yes`; a terminal
  with bracketed paste enabled wrapped a pasted answer in invisible control sequences.
- **Fix**: perform inspect display and enrollment display/confirmation only inside the guarded
  local-TTY window, render long argv/secret/source collections as vertical lists, avoid the
  duplicate generic prompt, and accept only `y`/`yes` after removing the exact bracketed-paste
  wrapper while continuing to reject every other decorated input.
- **Rule**: every local message emitted during an interactive SSH session must be written only
  after restoring local termios (or use raw-safe CRLF deliberately). Confirmation parsing must
  account for terminal protocol wrappers without broadly stripping arbitrary control sequences.

## 2026-08-07 — ccenv called a retired env decrypt command

- **Symptom**: selecting a provider with an encrypted credential always reported that decryption
  failed, without invoking the configured secret backend.
- **Root cause**: `ccenv` retained the former `shine env decrypt` command shape after secret
  operations moved under `shine env secret`, and suppressed the child command's stderr.
- **Fix**: invoke `shine env secret decrypt`, inherit stdin/stderr for that interactive child so
  GPG can obtain a card PIN/touch confirmation, and assert the complete argv in the credential test.
- **Rule**: command-wrapper tests must assert every CLI argument when invoking nested subcommands;
  otherwise a valid-looking wrapper can silently call a retired command path.

## 2026-08-04 — External shell sources exposed two incompatible apply models

- **Symptom**: editing external `cc.ts` took effect immediately, while editing an external
  template or app preset required `shine update` followed by `shine upgrade`.
- **Root cause**: external preset discovery also selected the deployment location. Raw shell
  launchers referenced the working tree, but transformed and app files used managed outputs.
- **Fix**: separate desired source from deployment. External shell commands default to managed
  snapshots; explicit live mode directly consumes raw content and lazily renders transforms.
- **Rule**: source selection must not implicitly change lifecycle semantics. Keep the default
  update/upgrade boundary uniform and make live execution an explicit, visibly reported mode.

## 2026-07-25 — Subscription values crossed the generated-config line boundary

- **Symptom**: a VMess subscription field containing CR/LF could add a second Surge configuration
  line instead of being counted as an invalid node.
- **Root cause**: quoting a value escaped quotes and backslashes but preserved control characters;
  Surge's configuration remains line-oriented even when a field is quoted.
- **Fix**: reject control characters in every emitted remote value, reject configuration
  delimiters in unquoted positional values, and keep node-name sanitization control-safe.
- **Rule**: quoting is not validation for generated line-oriented configuration. Validate every
  untrusted field before interpolation, and reject line/control characters even inside quotes.

## 2026-07-25 — Generator output limits were checked after unbounded capture

- **Symptom**: a faulty generator could make the parent buffer arbitrary stdout/stderr for the
  full timeout despite advertised 8 MiB/64 KiB limits.
- **Root cause**: `Command::output()` collected both pipes completely before their lengths were
  checked.
- **Fix**: drain stdout and stderr concurrently in bounded chunks, retain at most each configured
  limit, and terminate/reap the child immediately when either stream exceeds it.
- **Rule**: a post-capture size check is not a memory bound. Enforce subprocess output limits while
  draining both pipes concurrently so neither pipe deadlocks the child.

## 2026-07-19 — Interactive Windows SSH skipped the managed PowerShell profile

- **Symptom**: `shine ssh --remote-shell windows <host>` opened PowerShell 7, but Shine-installed
  source commands such as `setproxy` were missing.
- **Root cause**: the encoded-command wrapper launched the final interactive `pwsh.exe` child with
  `-NoProfile`. `setproxy` is intentionally a wrapper function registered by Shine's managed
  PowerShell profile rather than a standalone executable, so the command could not exist in that
  session.
- **Fix**: keep the outer selection bootstrap profile-free, but let the final interactive child
  load its normal profile; retain `-NoProfile` for explicit non-interactive remote commands.
- **Rule**: an interactive shell wrapper must preserve normal startup-file semantics unless the
  user explicitly requests isolation. Bootstrap isolation does not justify suppressing the final
  user's profile.

## 2026-07-19 — Windows split-DNS upgraded an already-current NRPT rule

- **Symptom**: `shine upgrade` always requested elevation, recreated the shine-owned Windows
  split-DNS NRPT rule, and counted it as updated even when its namespace and name servers already
  matched the active configuration.
- **Root cause**: the Windows branch of `split_dns_up_to_date` unconditionally returned false, and
  `apply_split_dns` unconditionally ran the elevated remove-and-create operation. The NRPT cmdlet
  exposes `Namespace` as an array and `NameServers` as `IPAddress` objects, not the scalar strings
  shown in its formatted table output.
- **Fix**: query `Get-DnsClientNrptRule` without elevation, project those properties into stable
  string arrays, parse only rules with shine's exact comment marker, and treat the resource as
  current only when exactly one rule has the desired namespace and ordered name-server list. Query
  or parse failures fail closed to the existing elevated repair path.
- **Rule**: convergence must validate the live managed resource before assuming a platform needs a
  write; a managed receipt alone does not prove current system state.

## 2026-07-19 — POSIX SSH wrapper failed on Windows OpenSSH remotes

- **Symptom**: `shine ssh --with-secret GH_TOKEN <windows-host>` sent `env ... sh -c` to the
  Windows remote and failed with `'env' is not recognized as an internal or external command`.
- **Root cause**: the command wrapper was selected from the local transfer architecture instead of
  the target command shell; Windows OpenSSH normally routes remote commands through `cmd.exe`.
- **Fix**: add explicit `--remote-shell windows`, which uses an UTF-16LE Base64-encoded PowerShell
  wrapper and intentionally does not create a POSIX transfer channel.
- **Rule**: remote command syntax must be chosen by the target shell. Never send a POSIX wrapper to
  Windows CMD, and never insert secret values into a cross-shell command-line syntax layer.

## 2026-07-18 — CVR Global Extend Config replaced subscription arrays and broke its own rules

- **Symptom**: `shine app reinstall clash-verge` wrote all three rule-providers but their refreshes
  returned HTTP 404. Saving Global Extend Config then failed validation because an original
  subscription rule targeted `Proxies`, which no longer existed.
- **Root cause**: CVR 2.5.1 treats global `proxies` / `proxy-groups` as whole-key replacements and
  removed global prepend/append support. The preset wrote fixed `profiles/Merge.yaml`, erasing the
  subscription groups at synthesis time; CVR also did not hot-reload that externally written file,
  so the earlier provider refresh ran against stale runtime config.
- **Fix**: split the composite source across the active subscription's bound Merge/Rules/Proxies/
  Groups files; render arrays into the editor-native `{ prepend, append, delete }` shape; never fall
  back to Global Extend Config; after a changed write, wait for profile reselection before refresh.
- **Rule**: third-party "merge" scopes are not interchangeable. Verify array semantics and reload
  behavior on the supported app version; resolve opaque per-profile filenames through the app's
  binding index, but never mutate that index or guess a global filename. Compare app-reformatted
  YAML semantically, not byte-for-byte, or every in-app save looks like a new change. Providers
  fetching an internal URL must declare `proxy: DIRECT`; otherwise a valid direct URL can fail as
  EOF when fetched through the subscription proxy. If that hostname relies on split DNS, mirror
  the suffix either in Merge's `dns.nameserver-policy` with CVR DNS Override disabled, or in
  Override's own Advanced Nameserver Policy when it remains enabled. Mihomo does not consume
  Windows NRPT, and CVR applies its generated `dns_config.yaml` after Merge, replacing the entire
  `dns` map. A Windows-side HTTP 200 does not prove mihomo resolves the same address; verify with
  the controller's `/dns/query` endpoint.

## 2026-07-18 — `sys init --proxy` had no effect on Windows (winget ignores proxy env vars)

- **Symptom**: `shine sys init --proxy` routed macOS/Ubuntu downloads through the local proxy but
  did nothing on Windows — the platform the feature was requested for — so installs still failed
  behind a firewall.
- **Root cause**: the first cut (commit `6a0ce96`) only injected the standard proxy env vars
  (`http_proxy`/`https_proxy`/`all_proxy`) into the init-script subprocess. `curl`/`apt`/`brew`/
  `rustup` honor those, but Windows `init.ps1` installs everything via **winget, which ignores
  `http_proxy`/`https_proxy` entirely** — it only accepts `winget install --proxy <uri>`, and that
  CLI option is *disabled by default* (needs a one-time admin `winget settings --enable
  ProxyCommandLineOptions`). Compounding it, `Install-WinGetPackage` never checked winget's exit
  code (a native exe's nonzero exit does not trip PowerShell's `ErrorActionPreference=Stop`), so a
  proxy-rejected install still reported `installed`.
- **Fix**: `build_proxy_env_vars` also exports `SHINE_SYS_PROXY` (the explicit URL signal), and
  `init.ps1` reads it to pass `winget install --proxy`, best-effort enabling the CLI option and
  checking `$LASTEXITCODE` to surface the admin remediation on failure.
- **Rule**: env-var proxying is not universal — verify the *actual downloader* honors it. winget
  needs `--proxy` + an admin-enabled setting, not `http_proxy`. And always check a native exe's
  exit code in PowerShell; `ErrorActionPreference=Stop` does not.

## 2026-07-19 — `winget upgrade --id` is not an update availability check

- **Symptom**: `shine sys update --proxy` installed a ZeroTier update while supposedly checking
  for updates.
- **Root cause**: the first Windows checker used `winget upgrade --id <id>` as though it were a
  scoped query. That command performs the upgrade; the proxy merely made its network access work.
- **Fix**: use the documented read-only `winget list --upgrade-available --exact --id <id>` query
  and only print `winget upgrade --exact --id <id>` for the user to run explicitly.
- **Rule**: never use a package manager's mutation verb to inspect availability, even when its
  output looks list-like. Verify the command's side effects in the upstream documentation first.

## 2026-07-17 — `app list` leaked a whole comment block as a category description

- **Symptom**: `shine app list` on a machine whose base binary predates the `clash-verge` preset
  showed the *entire* multi-paragraph `#` header of the overlay's `merge.yaml` as the category
  description (a one-line wall), while a machine whose presets include `clash-verge/shine.toml`
  showed the clean `description`.
- **Root cause**: with no `shine.toml` for the category, the legacy auto-collect path derives the
  description from each file's leading comment block via `parse_script_description`, and
  `parse_legacy_description` joined the *whole* block with spaces (no first-line truncation, no
  extension filter). A data file like `merge.yaml` uses `#` comments, so its long header was parsed
  and dumped verbatim by `apps/info.rs handle_list`.
- **Fix**: `parse_legacy_description` now returns only the first non-empty comment line (the
  summary), matching how single-line legacy presets (`git`, `starship`) already read.
- **Rule**: an auto-derived one-line description must take only the first comment line — never join
  a whole `#` block. A preset's real description belongs in `shine.toml`; the comment-header
  fallback is a last resort and must stay one line. (The cross-platform *difference* was really a
  stale binary: a machine missing the embedded `clash-verge/shine.toml` falls to this fallback.)

## 2026-07-16 — OSC 11 reply leaks in full: macOS `poll` returns `POLLNVAL` on `/dev/tty`

Follow-up to the 2026-07-14 entry. That fix (total-deadline read in `cli/src/theme/osc.rs`) cured
the *fragmented-tail* leak but left a second, distinct one that reproduces on Ghostty (macOS): the
**whole** reply leaks — `\033]11;rgb:ffff/ffff/ffff\033\\` with the leading `\033]` intact, not
just the tail. It took **two** independent fixes; the first was necessary but not sufficient, and
the second was the real blocker.

- **Symptom**: every fresh Ghostty login shell prints `^[]11;rgb:…^[\` before the first prompt;
  `shine theme sync` also reports "could not determine terminal theme". The full sequence (leading
  `\033]` present) means the read loop consumed **zero** bytes — unlike 2026-07-14, where the
  consumed head made `\033]` absent.
- **Cause 1 (necessary): canonical mode.** `EchoGuard::disable` cleared `ECHO` but not `ICANON`,
  so the tty stayed in canonical (line) mode. An OSC 11 reply has **no newline**, and the
  canonical line discipline never marks a newline-less line readable — so `select`/`poll`/`read`
  can't see it. Fixed by `EchoGuard` → `TtyQueryGuard`, which clears `ECHO | ICANON` and sets
  `VMIN=1`/`VTIME=0` (in non-canonical mode those `c_cc` slots alias `VEOF`/`VEOL`; leaving them
  inherits `VEOF`=4 as `VMIN` and stalls fragmented reads until 4 bytes accrue).
- **Cause 2 (the real blocker): macOS `poll(2)` on `/dev/tty`.** Even with `ICANON` off the leak
  persisted, because the read loop waited for readability with `poll(2)` and required
  `revents & POLLIN`. On macOS, `poll` on `/dev/tty` returns **`POLLNVAL`** (revents `= 32`,
  `POLLIN` unset) *even though the fd is readable* — a long-standing Darwin bug. So the loop broke
  on its first iteration and read nothing. Fixed by waiting with `select(2)` instead
  (`wait_readable`), which reports tty readability correctly on both macOS and Linux.
- **How it was measured** (not inferred — cf. the 2026-07-14 meta-rule): a `tty.setcbreak` probe
  against the live Ghostty tty read the full 25-byte reply in ~0.3 ms with `select`; a `poll` probe
  that *checked revents* showed `revents=POLLNVAL, POLLIN=False`. The mistake that cost a round:
  an earlier `poll` probe read the fd whenever poll returned *any* event and so "passed",
  masking the missing `POLLIN`. Always inspect `revents`, don't just test "poll returned".
- **Why the tests missed both**: the `matrix_*` tests drive the loop over a `UnixStream` socket
  pair — no line discipline (so canonical mode never applied) and no tty (so the macOS
  `poll`/`POLLNVAL` behavior never applied). The `openpty` regression test
  (`read_loop_reads_newline_free_response_through_pty`) exercises canonical mode on a real pty, but
  even a pty **slave** does not reproduce the `/dev/tty`-specific `POLLNVAL`; that path is only
  observable against a real controlling terminal, so it stays covered by this lesson, not a test.
- **Rule**: reading a terminal-control reply needs **both** (a) non-canonical mode (`ICANON` off —
  control replies carry no newline) and (b) `select(2)`, not `poll(2)`, to wait for readability —
  macOS `poll` is unreliable on `/dev/tty`. And when probing a syscall's behavior, assert on its
  actual output flags (`revents`), never on "the call returned something".

## 2026-07-26 — Removed global state needs a fail-fast recovery tombstone

- **Risk**: silently ignoring `~/.shine/env.toml` after removing its automatic migration would
  make users believe their environment values were still active, while automatically deleting
  or rewriting it would destroy the easiest recovery path.
- **Fix**: normal config initialization now detects the removed file before saving anything and
  stops with v0.39 migration or explicit move/merge instructions. It never parses, modifies, or
  deletes the file.
- **Exception**: the read-only global loader used by dry runs and `theme sync` deliberately
  ignores the tombstone so shell startup remains non-fatal and side-effect free.
- **Rule**: when retiring user-owned state, fail before mutation and preserve the original for
  recovery; keep explicitly non-fatal/read-only startup paths outside that guard.

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
  description-preserving via the same `shine_core::migration::sync_table` `Config::save()` uses),
  and warns loudly when the winning file is the shine-managed overlay mirror, since that
  write is discarded on the next `shine preset pull`.
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

## 2026-07-26 — Global upgrade no-op sections must honor `--verbose`

- **Symptom**: a default `shine upgrade` printed a full `Managed System Configs` section for an
  already-current split-DNS resource, even though the only result was counted as skipped.
- **Root cause**: the global upgrade's `verbose` flag was not passed to the managed-system
  handler, whose output path printed every selected item and outcome unconditionally.
- **Fix**: hide `already installed` and ordinary `skipped` managed outcomes by default, lazily
  create the section only for a visible result, and retain the full listing under `--verbose`.
- **Rule**: optional upgrade sections must not print a header for no-op rows hidden by the
  command's verbosity policy; aggregate counters may still include those rows.
- **Follow-up (2026-08-01)**: shell and app upgrade paths still printed inventory-only section
  headers during a no-op run, and the footer collapsed unlike outcomes into an incomplete
  `skipped` count. Global upgrade now lazily prints all three subsystem sections, reserves preset
  source paths and ordinary no-op rows for `--verbose`, and limits the default footer to actionable
  outcomes. The global heading is lazy too, so a fully converged default run matches `shine update`'s
  compact empty-state style with the single line `Nothing to upgrade.`.

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

## 2026-07-31 — Embedded shell sources need identity and content status

- **Symptom**: after moving `agent/ccenv` from sourced `cc.sh`/`cc.ps1` to Bun `cc.ts`, a freshly
  installed binary reported `Nothing to update` and kept running the old source wrapper.
- **Root cause**: shell status compared effective bytes only for template-rendered scripts,
  treated a missing new embedded source as generic missing state, and checked bin entries only
  for existence. In external-presets mode the new source already existed, so the stale native
  link was incorrectly considered current without validating its source or runtime.
- **Fix**: `status::shell_source_status` reports an update when an installed command's expected
  embedded source is absent and compares raw extracted bytes with rust-embed. Shell rows also use
  `bin_links`' install-equivalent current-ness check for source, runtime, and runtime env.
- **Rule**: shell status must compare expected source identity, content, and launcher runtime;
  existence of some command at the target name does not prove the current preset is installed.

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
- **Fix**: `a5aed62` (inheritance) + `0936f05` (scheduled cleanup of the legacy project file);
  v0.40.0 completed that cleanup after the deprecation window.
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
