# 0166 — Surge General settings use a local native module

- **Status**: accepted
- **Extends**: [ADR 0017](0017-built-in-surge-profile-artifact.md)
- **Evidence**: `presets/app/surge/{shine.toml,shine-general.sgmodule,surge-module.ts,profile-artifact.ts}`,
  `surge-module.test.ts`, `tests/builtin_presets/app.rs`

## Decision

Ship an inert `shine-general.sgmodule` as an ordinary manifest-owned App file. Its per-file
destination is Surge's native macOS Profiles directory even if the category root is overridden.
Keep both filename and `#!name=shine-general` fixed. Personal policy belongs in an overlay;
the built-in module contains commented examples only. Surge owns the effective `[General]`
merge and device-wide enabled state. Do not rewrite General properties or introduce property
ownership receipts, journals or schema extensions in Shine.

The original `[Proxy]`, `[Proxy Group]` and `[Rule]` include algorithm remains. Apply computes
the include change without writing, checks that the regular installed module exactly matches
the effective reviewed snapshot and fixed identity, and queries native discovery. Only then does
it write includes, recheck the installed module, enable the exact fixed module if necessary,
verify the resulting enabled state, and reload. Profile identity/mode/content are rechecked
before replacement; UTF-8 decoding preserves a BOM. These checks narrow external editor races
but do not claim a filesystem/native-controller transaction.

Remove disables the fixed module and reloads before attempting profile cleanup, even if the
profile is missing or malformed. It reloads again after changed includes. Core already invokes
teardown before deleting managed files; its best-effort uninstall failure policy is unchanged.
Artifact remove leaves the module file installed. Install/upgrade only deploy and run their
existing reload hook, never a module-enable command. Surge can remember enabled state across
deletion/reinstallation, so reinstall is not a guarantee of initially disabled behavior.

Explicit artifacts require a running Surge with CLI module support. Commands have a bounded
timeout; nonzero exits, unexpected inventory and failed state verification propagate as errors.
No arbitrary controller output is printed. Only the fixed module is changed. The parser rejects
duplicate identities rather than choosing an ambiguous native module. A missing module cannot be
disabled by native CLI: remove warns and reloads to clear cached effects, but clearing a remembered
enabled setting requires restoring the file first. Never edit Surge's private preference store.

## Failure and retry boundary

File writes and native state changes are separate effects under the existing approved artifact.
If enable/reload fails, includes may already be patched. If later include cleanup fails, the
module may already be disabled. Retry re-reads native state and computes the same include result;
it does not reverse user edits with a saved full-profile backup. Do not report an unsuccessful
reload as successful application. No new Core transaction or automatic rollback is claimed.

Controlled native tests on Surge Mac 6.10 confirmed: local modules start disabled; repeated
enable/reload does not accumulate values; local file updates take effect on reload; disabling
restores the original effective configuration without changing source Profile bytes. `%APPEND%`
does not deduplicate overlapping or repeated input. Deleting an enabled file alone leaves its
effect until reload, and restoring the same filename retains its enabled setting. These facts
require disable-before-delete ordering. The automated adapter tests use an isolated native-command
double to cover errors, state verification, interrupted state changes and retries without affecting
the developer's real Surge instance.

## Alternative

A declarative Profile property patcher would require its own parser, per-property ownership,
conflict handling and durable recovery. Native modules provide the requested list append/scalar
override semantics with less maintenance. The tradeoff is device-wide scope rather than a change
limited to `SURGE_PROFILE`, no append deduplication, and requiring a running native controller.
Modules do not replace existing Proxy/Proxy Group includes.
