# Env configuration: Windows validation and downgrade drill

## Scope and identities

The 2026-10-11 native Windows x86_64 check validated the env precedence and directed-write change
in [ADR 0109](../decisions/0109-env-baseline-and-directed-writes.md) and the
[env PRD](../../env-configuration-prd.md). The candidate was the modified working-tree build
`2.3.1+env-validation-working-tree (30697c8df)`, not the stable 2.3.1 executable. The downgrade
baseline was the official v2.3.1 Windows binary at `2d83e6f68`. Windows PowerShell 5.1 and
PowerShell 7 were exercised.

State, project, overlay, HOME, APPDATA, LOCALAPPDATA and backup fixtures use fresh temporary
directories with synthetic values. Encryption uses a native GPG handshake probe without a real
keyring; launcher tests use a synthetic Bun probe. Supply existing isolated Windows data
directories so config discovery and completion cannot inherit the developer's profile.

## Native regression boundaries

[`tests/env_configuration.rs`](../../../tests/env_configuration.rs) retains these independent
scenarios, including the PRD's Windows-specific replacement check:

| Boundary | Assertion |
| --- | --- |
| Read-only queries and six-layer resolution | Queries preserve absent roots and existing bytes; project config outranks global env; value/description inheritance and empty descriptions are independent |
| Directed writes and retired `--force` | Scope validation, explicit shadowed maintenance, manual overlay writes, inherited deletion and zero-write errors retain their contracts |
| A17: encryption preflight | A shadowed implicit write fails before encryption; edits to the target or another source layer reject the stale commit and retain the edit |
| A18: concurrent Shine processes | Two processes reach the encryption barrier from the same snapshot; exactly one commits and the other reports retry without losing the source entry |
| A19: Windows replacement failure | A native handle permits read/write but denies `FILE_SHARE_DELETE`; replacement fails with original bytes and no temporary file left; releasing the handle permits retry while preserving comments and unrelated fields |
| Runtime and save consumers | PowerShell launch, literal paths/argv, env layering, template rendering, config save, proxy rollback and schema save retain declared injection without persisting inherited defaults |

Encryption and two-process tests cover Windows and Unix; the deny-delete handle regression is
Windows-only. Native path assertions compare `Path` values while retaining exact switches,
argument counts and targets, so separator spelling does not weaken command verification.

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

The native drill confirmed the following upgrade/downgrade boundaries:

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

## Remaining boundaries

The historical check covered native Windows execution/replacement and the old-binary downgrade
path. It did not verify power-loss durability, arbitrary non-cooperating editor races, Windows
ARM64, real encryption devices, or installation-artifact rollback. Config/binary changes do not
approve or apply Preset lifecycle updates. The retained reproduction script is the entry point
for checking this compatibility boundary again with independently verified binaries.
