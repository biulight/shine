# 0112 — Explicit file-group export retains user-managed plaintext

- **Status**: accepted
- **Evidence**: `cli/src/env/workspace/file_groups.rs`, `tests/workspace_secret_files.rs`

## Decision

Extend `env workspace export` with `--format files`, mandatory `--mode` and explicit repeatable
`--file-group`. Export resolves existing version 3 mappings in place, deduplicates selected groups,
and preserves source ciphertext and workspace configuration. It never compiles `env.files`.
The dotenv interface remains compatible; files rejects output, force and include-secrets options.
No workspace or ciphertext schema changes are needed.

Share selection, path/Git preflight, snapshot capture, binary decryption, local hybrid preference
and seal/run locks. Capture every source before decryption, freeze hybrid backend choice, decrypt
all files before creating targets, and recheck inputs before and throughout writes. Use private
exclusive target creation and sync before completion, with the existing byte limits and native
permission policy. Final creation rejects concurrent writers rather than relying on preflight.

Keep an owned-object rollback guard through final snapshot/output identity checks. Failure or
controlled cancellation attempts removal of newly owned files and empty directories; replacements
remain protected and cleanup errors identify possible residue. Success explicitly relinquishes
cleanup responsibility. Files are ordinary user-managed plaintext thereafter. Run still refuses
existing targets, and export never skips, overwrites, adopts, or later deletes them automatically.

## Consequences

Users can prepare ordinary files once and use tools without a Shine wrapper. Successful exports
stay until manually managed or removed, and command output warns about persistent plaintext.
Export is not a cross-file atomic transaction: partially written targets may be observed during
writing, and abrupt termination/power loss may leave files. Controlled rollback is not secure erasure.
No alternate output root, force mode, broker protocol, plaintext cache, receipt, automatic resealing
or recovery command is added.
