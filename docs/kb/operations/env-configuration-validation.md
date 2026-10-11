# Env configuration: Windows validation and downgrade drill

## Scope and identities

2026-10-11; native execution on the user-authorized `nuc.win.local` Windows instance.
This records the unreleased [env PRD](../../env-configuration-prd.md) and
[ADR 0109](../decisions/0109-env-baseline-and-directed-writes.md), not release approval.

| Item | Observed identity |
| --- | --- |
| OS / target | Windows 11 Pro, 10.0.22631, x86_64-pc-windows-msvc |
| Rust / nextest | rustc 1.96.0 (ac68faa20), cargo-nextest 0.9.143 |
| Shells | Windows PowerShell 5.1.22621.6133; PowerShell 7.6.6 |
| New executable | `shine 2.3.1+env-validation-working-tree (30697c8df 2026-10-11)`; native debug build of the modified working tree, not the stable 2.3.1 release |
| Source snapshot | Current contents of 720 tracked/selected new files; archive SHA-256 `d77660473267cb787332431b7a7bed441785284521891c5f0592ec198be4aa46` |
| Previous executable | Official `shine-v2.3.1-windows-x86_64.tar.gz`; `shine 2.3.1 (2d83e6f68 2026-10-06)` |
| Stable commit | `v2.3.1^{commit}` = `2d83e6f68a393b93480281961afeef78142e2add` |
| Stable archive SHA-256 | `1682a672d19b5807db8e2f9db844e64854bd1a8408a1ef290c6f967371153d54`; matched the GitHub release asset digest before execution |

Source was transferred directly; no commit, push or remote checkout reset was needed. The new
binary's build metadata distinguishes it from the unchanged package version. The source archive
contains the Windows regression tests; the final downgrade script and this record were added
after that snapshot and do not change the compiled implementation.

All state, project, overlay, HOME, APPDATA, LOCALAPPDATA and backup fixtures use fresh temporary
directories with synthetic values. Encryption uses a locally compiled native GPG handshake probe,
without a real keyring. Launcher tests use their synthetic Bun probe. No user configuration,
real secret, source activation, real Preset execution, installed artifact upgrade or publication
was part of this check.

## Native regression results

Executed in the isolated source directory after trusting its inspected, pinned `mise.toml`:

```powershell
mise exec rust@1.96.0 -- cargo nextest run --locked --workspace --target-dir target --all-features --no-fail-fast -E 'test(config::) | test(env::) | test(persist::) | binary(env_configuration) | binary(live_bun_launch) | binary(runtime_boundaries) | test(state::tests::)'
```

**223 tests passed; 1070 tests were excluded by the filter.** This initial run was targeted native
validation; the later full-suite run is recorded below. Compilation took 1m31s; the selected
tests took 4.637s. Windows conditional compilation emitted unused/dead-code warnings in existing
modules; a Windows `clippy -D warnings` run was not part of this check.

| Coverage | Actual result |
| --- | --- |
| Read-only queries, six layers and sources | Real CLI queries preserve absent roots and existing bytes; project config outranks global env; description inheritance and empty descriptions pass |
| Directed writes and retired `--force` | Scope validation, explicit shadowed maintenance, manual overlay writes, inherited deletion and zero-write errors pass |
| A17: encryption preflight | Shadowed implicit write fails before the probe; edits to the target or a different source layer during encryption reject the stale commit and retain the edit |
| A18: two actual Shine processes | Both reach the encryption barrier from the same snapshot; after release exactly one commits and the other reports retry, without losing the source entry |
| A19: Windows replacement failure | A native file handle permits read/write but denies `FILE_SHARE_DELETE`; CLI replacement fails, original bytes remain, and no temporary file remains. Releasing the handle permits retry; comments and unrelated fields survive |
| Existing runtime consumers | Five native live Bun launcher tests pass, including PowerShell launch, literal state paths, argv, project/global layering, template rendering and declared env injection |
| Other save paths | Config save, proxy rollback and runtime schema-save regressions pass without expanding inherited defaults |

The replacement regression is Windows-only in
[`tests/env_configuration.rs`](../../../tests/env_configuration.rs). Encryption and two-process
tests now run on both Windows and Unix. Their fixture supplies existing, isolated Windows data
directories rather than falling back to the developer profile.

## Upgrade and downgrade drill

Reproducible script:
[`scripts/verify-env-downgrade.ps1`](../../../scripts/verify-env-downgrade.ps1).
Run it in PowerShell 7 with explicit, independently verified executables:

```powershell
pwsh -NoProfile -File scripts/verify-env-downgrade.ps1 -CurrentBinary C:/new/shine.exe -PreviousBinary C:/stable/shine.exe
```

The script creates its own temporary root, records each exit code/stdout/stderr in `commands.json`,
and leaves synthetic fixtures, backups and `result.json` for inspection. It also invokes the new
CLI through Windows PowerShell 5.1 and PowerShell 7 using the same isolated environment.

**Final result: PASS, 24 CLI/shell invocations.** Assertions cover:

1. Prime the old binary's historical default expansion, then back up five config/env files.
   Old project lookup returns `overlay-baseline`; after binary upgrade, the new lookup returns
   `project-config` and explain identifies project config. Read-only queries leave all file hashes
   unchanged. Outside the project, new lookup returns `global-env`, old lookup `overlay-baseline`.
2. New `--force` exits 1 without writing and suggests `--overlay`, the old redirection target.
   Explicit global set saves `new-global` while project config still wins. Deleting the project
   entry restores inheritance from the global env file. Deleting a separate custom project entry
   removes only that local definition.
3. Running official v2.3.1 against the edited state restores overlay priority. The deleted custom
   entry remains absent and lookup fails; binary downgrade cannot recover it. New deletion of a
   global builtin entry leaves only the runtime fallback; old lookup subsequently resumes the old
   default expansion into global config. This is default insertion, not recovery of deleted custom
   values. Both public manual locales now state this downgrade boundary.
4. The old `--force` command writes the manual overlay, confirming the new migration diagnostic's
   target. Restore the five private fixture backups; every file's SHA-256 matches its pre-upgrade
   backup. Old lookup again returns the overlay value and the restored custom entry; new lookup
   returns the project value. Both PowerShell generations launch the new CLI successfully without
   changing the restored files.

Raw logs and final JSON were retained under the isolated Windows `%TEMP%` validation root and
copied to the local temporary validation directory. They contain only fixture values. The script's
retained JSON records the verified executable versions, all command results and restored hashes.

## CI follow-up: full native Windows suite

The [CI run for `8e9fb3b`](https://github.com/biulight/shine/actions/runs/38120987713) passed quality,
Ubuntu, macOS, MSRV and documentation checks, but the Windows job reported 1291 passes and two
failures. Both were path-string assertions in legacy-service cleanup and fixed App environment
tests, outside the initial targeted filter. Mixed and native separators identified the same paths.
The fix compares `Path` values while retaining exact command switches, argument count and target
checks; no runtime behavior or platform coverage was removed.

On `nuc.win.local`, the modified source passed the full CI target:

```powershell
cargo nextest run --locked --workspace --target-dir target --all-features --target x86_64-pc-windows-msvc --no-fail-fast
```

**1293 tests passed, 0 skipped, in 19.610s.** The process had fresh existing HOME, USERPROFILE,
APPDATA and LOCALAPPDATA directories; CARGO_HOME/RUSTUP_HOME retained the pinned toolchain and
dependency cache. The first full native attempt inherited the host profile and exposed an embedded
completion assertion influenced by its configured presets. Isolating the test process resolved
that fixture condition without changing or weakening the completion test. Both initial CI failures
also passed on macOS in the 13-test legacy-service/App-script regression selection.

## Remaining boundaries

This closes the PRD's separate native Windows execution/replacement and old-binary downgrade
checks. It does not verify power-loss durability, arbitrary non-cooperating editor races, Windows
ARM64, real encryption devices, or installation-artifact rollback. No version/release decision
was made. Config/binary changes still do not approve or apply Preset lifecycle updates.
