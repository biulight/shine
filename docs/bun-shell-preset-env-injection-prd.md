# Bun Shell Preset Runtime Environment Injection PRD

## 1. Background

The completed [Bun shell preset v1](bun-shell-presets-prd.md) gives a TypeScript or
JavaScript entry point a stable cross-platform command through a shine-managed Bun
launcher. v1 also supports the shared install-time `template` transform, where
`@@VAR@@` is substituted into a rendered script.

That mechanism remains useful for generated configuration files, but it is an
awkward default for TypeScript: Bun already exposes child-process environment
variables as `Bun.env`. It also writes each configured value into the rendered
script, meaning an environment change requires `shine upgrade` before the helper
observes it.

This v2 increment lets a Bun entry explicitly request selected shine environment
values at launch time. Shine resolves those values using its existing encrypted-first
rules and exposes them only to the Bun process tree. The parent terminal is never
modified.

## 2. Goals

- Let a Bun shell helper read explicitly declared shine values through `Bun.env`.
- Keep values out of preset source, rendered scripts, launchers, and normal output.
- Allow explicitly declared secrets, decrypted only for the launched child process.
- Reuse the existing `env run --with` resolution and child-process semantics.
- Preserve Bun v1 launcher ownership, conflict protection, upgrade, and uninstall
  invariants on Unix and Windows.
- Keep install-time `transforms`, including `template`, available and unchanged.

## 3. Non-goals

- Do not automatically export all of `[env]`, workspace values, or inherited shell
  values to a helper.
- Do not modify the caller's shell session. A helper cannot make a later
  `echo "$NAME"` in its parent terminal see an injected value.
- Do not add runtime environment injection to native `.sh` or `.ps1` entries in
  this increment.
- Do not add a general runtime abstraction, automatic Bun/shine installation, or
  dependency/package management.
- Do not remove or redefine `transforms = ["template"]`.

## 4. Preset Interface

Add an optional `env` array to a Bun `[[files]]` entry:

```toml
description = "Cross-platform API helper."

[[files]]
source = "my_tool.ts"
target = "mytool"
runtime = "bun"
platforms = ["unix", "windows"]
env = ["API_URL", "SERVICE_TOKEN=API_TOKEN"]
```

Each array item uses the same grammar as `shine env run --with`:

- `KEY` resolves `KEY` and exposes it to Bun as `KEY`.
- `SOURCE_KEY=TARGET_KEY` resolves `SOURCE_KEY` and exposes it as `TARGET_KEY`.
- Both names must be valid environment identifiers: first character `[A-Za-z_]`,
  remaining characters `[A-Za-z0-9_]*`.
- Two declarations may not write the same target name.
- `env` defaults to an empty list.
- `env` is valid only when `runtime = "bun"`; metadata loading must reject it for
  native entries with a contextual error.

The Bun source consumes the resulting variables normally:

```ts
const apiUrl = Bun.env.API_URL;
const token = Bun.env.API_TOKEN;
```

`env` is an allowlist, not a discovery mechanism. Metadata stores only variable
names and aliases, never values or ciphertext.

## 5. Runtime Behavior

For an entry with no `env` declarations, the v1 launcher behavior is unchanged:

```text
bun <effective-script-path> <args...>
```

For an entry with declarations, its launcher invokes the equivalent of:

```text
shine env run --no-workspace --with API_URL --with SERVICE_TOKEN=API_TOKEN -- bun <effective-script-path> <args...>
```

The actual Unix, PowerShell, and cmd launcher content must preserve the existing
platform-specific quoting and argument-forwarding guarantees. `SHINE_CONFIG_DIR`
and other inherited process settings naturally flow to the nested shine invocation.

The launcher resolves `shine` by **bare name through `PATH`**, not by an absolute
path captured at install time. A bare name is robust to relocating or upgrading the
`shine` binary (an absolute path would silently break when `shine` moves), at the
cost of a runtime dependency on `shine` being discoverable on `PATH` — note that
`shine` itself is not necessarily installed under `~/.shine/bin`.

Before launching, the wrapper verifies both `shine` and `bun` are on `PATH`. A
missing command produces a concise actionable error and exits with `127`; it never
downloads or installs either command. A missing requested value, invalid stored
value, or secret decryption failure prevents Bun from starting and returns the
underlying error without printing the value.

### 5.3 Cost model

Runtime injection is not free, and its cost is paid **only by entries that declare
`env`**:

- **Deeper process chain.** A v1 Bun entry runs `shell → launcher → bun`. A
  declaring entry runs `shell → launcher → shine (Config::load_or_init) → bun`,
  adding one `shine` process (and its config load) per invocation.
- **Per-invocation secret decryption, uncached.** The explicit `--with` path has no
  compiled cache (only the workspace path caches). Each `<KEY>_SECRET` is decrypted
  on **every** run. With an interactive backend — age Secure Enclave (`age-plugin-se`
  Touch ID) or GPG pinentry — this means a biometric/passphrase prompt on every
  invocation. Declare secrets in `env` only for helpers where that per-run prompt is
  acceptable; prefer plaintext `[env]` values for frequently-run helpers.

### 5.1 Workspace isolation

Add `--no-workspace` to `shine env run`. It disables workspace discovery and
loading entirely, so the command uses only explicit `--with` values and inherited
process environment. It is mutually exclusive with `--workspace` and `--mode`.

This flag is required for generated launchers. Without it, invoking a helper from a
directory containing `shine.workspace.toml` could unexpectedly load workspace
variables or fail because a workspace mode was not selected. Normal user-invoked
`shine env run` behavior remains unchanged when the flag is absent.

### 5.2 Process boundary

Injected values are applied to the direct Bun child and its descendants only. They
cannot modify the parent shell:

```text
mytool
echo "$API_TOKEN"  # remains unchanged
```

Users who intentionally need a persistent shell export continue to opt in with
`eval "$(shine env secret export API_TOKEN)"`. That broader exposure is not performed by
Bun launchers.

## 6. Transforms and Upgrade Semantics

`env` and `transforms` are independent:

- `env` resolves values on every helper invocation; changing a configured value is
  observed on the next run and does not require `shine upgrade`.
- `transforms = ["template"]` renders `@@VAR@@` at install/upgrade time; it remains
  appropriate when a downstream file format requires static text.
- A Bun entry may declare both. The launcher targets the rendered file as in v1,
  then injects its declared runtime values before executing Bun.

Because launcher content changes when its environment declaration changes, launcher
currentness must include the ordered `env` specification. Existing marker-based
ownership rules remain the sole authority for overwrite and uninstall decisions.

## 7. Security and Lifecycle Requirements

- Resolve each declaration through the existing `env run --with` path: prefer
  `<KEY>_SECRET` and decrypt it; otherwise read plaintext `KEY`.
- Never include resolved values in launcher content, metadata-derived status output,
  diagnostics, or test snapshots.
- Injected values live in the Bun child's environment (and its descendants), so they
  are readable by same-user processes via `/proc/<pid>/environ` or `ps eww`. This is
  the standard exposure of environment-variable passing — the same trade-off already
  accepted for the SSH session token (see `AGENTS.md`) — not a regression to hide.
- Keep the existing rule that unmarked, unreadable, altered, or foreign launchers
  are user files: never overwrite or delete them without the current explicit
  force behavior.
- Regenerate launchers deterministically and refresh **only when the resolved
  launcher content actually changes**. An entry that gains, drops, or reorders an
  `env` declaration produces different launcher bytes and refreshes on
  install/reinstall/upgrade. An entry with no `env` declaration produces launcher
  bytes byte-identical to v1 and stays current — it must not be needlessly rewritten.
  A user-modified launcher remains a conflict rather than becoming implicitly managed.
- Existing Bun entries that omit `env` retain their v1 launcher behavior and do not
  gain a runtime dependency on a nested `shine` command.

## 8. Implementation Outline

1. Extend shell metadata (`FileToml`, resolved file, and `ShellFile`) with an
   ordered, validated runtime-environment specification. Propagate it through
   `shells::links` into `bin_links::LinkSpec`. Validate declaration names and
   duplicate targets at **metadata load time** (install), so a malformed preset
   fails fast rather than at first run. This means reusing the same rules the
   runtime path uses: lift `env::workspace::validate_env_key` (today a private `fn`)
   and the duplicate-target check out of `resolve_explicit_values` into a shared
   `pub(crate)` location both the metadata parser and the `--with` resolver call.
2. Add `--no-workspace` to `EnvRunCommand` and `env::workspace::handle_run`; bypass
   workspace discovery in that mode and reject incompatible workspace flags.
3. Extend generated Bun launcher content on Unix and Windows only when the entry
   declares environment values. Preserve direct argv execution, platform quoting,
   child exit status, ownership markers, and deterministic stale detection.
4. Update `shine preset new shell`, README, Chinese README, and preset authoring guidance
   to recommend `env` plus `Bun.env` for Bun helper configuration; document
   `template` as the static-rendering alternative.
5. Update the relevant architecture data-flow documentation and launcher invariant
   wording if the new launcher format changes their stated behavior.

## 9. Acceptance Tests

- Metadata parsing accepts direct and aliased declarations, preserves order, and
  rejects invalid names, duplicate target names, and `env` on native entries.
- `env run --no-workspace` injects explicit values without discovering a nearby
  workspace; `--workspace` and `--mode` conflict with it.
- Plaintext and encrypted values reach `Bun.env`; missing values and decryption
  failures do not spawn Bun and do not leak plaintext.
- Unix, PowerShell, and cmd launcher golden tests cover empty and declared
  environments, paths/arguments containing spaces, and exact exit-code forwarding.
- A v1 launcher upgrades deterministically; an unchanged v2 launcher is current;
  a user-modified or unmarked launcher is protected from overwrite and uninstall.
- A Bun entry that uses both `env` and `template` receives runtime values while its
  launcher still targets the rendered script. Existing transform-only Bun entries
  continue to work unchanged.
- A launcher whose entry declares `env` exits `127` without starting Bun when
  `shine` is not resolvable on `PATH` (mirroring the existing missing-`bun` case).
- A declared secret is decrypted at run time and reaches `Bun.env`, and the
  decrypted value never appears in launcher content, status output, diagnostics, or
  test snapshots.
- The nested `shine env run` inherits `SHINE_CONFIG_DIR` from the launcher's
  environment, so runtime resolution stays consistent under test isolation and
  custom config directories.
