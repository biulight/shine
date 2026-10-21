# ADR 0137: Test production sealing, decryption and profile owners

- **Date**: 2026-10-17
- **Status**: Accepted

## Decision

Delete the uncalled `plain_values_from_broker_snapshot` entry and its public re-export. Keep
`merge_broker_values`, complete captured-source validation and effective binding resolution.
An older broker may return a secret shadowed by later plain data; filtering and zeroization of
that response remain unchanged, as do release policy, confirmation and snapshot identities.

The plain-only sealing regression calls `handle_seal` with an explicit isolated workspace and
source file, exercising production configuration resolution, locking and source capture. Preserve
legacy version-1 source parsing, the plain value and the empty payload without recipients. Remove
the test-only `seal_file` orchestration; keep shared validation, preparation, snapshot checks and
atomic replacement, including link rejection and multi-file failure behavior.

The controlled GPG/age process regression calls `decrypt_with_config` with configured identities.
Set the default encryption backend opposite to the ciphertext backend to prove tag-based routing.
Assert backend process arguments so legacy `age_identity` and additional `age_identities` reach
age in order, with duplicates removed and paths containing spaces preserved as single arguments.
Retain exact untagged GPG/age ciphertext, trailing plaintext newlines, no external base64 command
and legacy recipient compatibility assertions. Delete `decrypt_secret`, which had no production
caller; keep the shared decryption dispatcher and hybrid preference semantics.

Move eleven profile rendering, completion, source-wrapper, PowerShell path and sentinel regressions
from CLI tests into Core's profile module, calling the same production functions directly. Remove
the five CLI test wrappers and the unused test sentinel alias. Keep `managed_profile_snippet`,
`shell_config_snippet` and `remove_shell_sentinel` visible inside runtime only; make
`powershell_bin_assignment` and `powershell_quote` private. Remove their public runtime re-exports.
Production CLI activation hints, completion detection and profile install/removal adapters remain.

## Boundaries and validation

Validate migrated tests before deleting old wrappers. After cleanup, run workspace tests,
approval/read-only capability doctests, formatting, lint, dependency and documentation checks.
Only test ownership, unused code and internal visibility change. Profile bytes, legacy handling,
secret formats, approvals, locks, receipts, journals and recovery ordering remain unchanged.
Native Windows/Linux execution and real encryption hardware/provider behavior are not established
by local tests and controlled backend stand-ins. There is no public manual or portal behavior change.
