# 0157 — Pair Shell cache-replacement contracts with execution bytes

- **Status**: Accepted
- **Date**: 2026-10-29
- **Extends**: [ADR 0117](0117-paired-shell-preparation-and-app-assessment-states.md)

## Context

Cache replacement independently appended Action file contracts and prepared execution records.
Paths, previous-file presence and desired mode were initialized in both representations. Removal
can use only the Action list (ADR 0156), but replacement must retain private write bytes.

## Decision

Keep the existing private `PreparedShellCacheFile`, storing one `ShellCacheFileReplacementV1`
contract and its write bytes. Preparation appends each complete record after the same observations
and checks. At the existing Action construction point, project complete contract clones into the
journal list. Apply reads paths, previous presence and desired mode from the paired contract.

Keep validation, permissions, journal publication, file order, rename/write/mode order, locks,
receipt boundaries and recovery unchanged. Bytes stay in memory; no public input, persisted schema,
error text or ownership scope changes. Cache ownership remains a per-file patch, not a whole tree.

## Consequences

One independently built list and four mirrored fields disappear without new types or helpers.
The journal still contains contract copies; these are read-only during apply. Preparation must
still bind bytes to the desired hash and mode. Pairing is not proof of byte equality or authority.
Existing mixed-action and receipt-failure coverage remains. A focused regression interrupts either
write in a mixed overwrite/create batch, checks the journal contracts and partial state, preserves
user edits, restores previous bytes/mode and retries while retaining an unrelated file.
