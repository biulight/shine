# 0155 — Pair Shell profile file contracts with execution bytes

- **Status**: Accepted
- **Date**: 2026-10-29
- **Extends**: [ADR 0117](0117-paired-shell-preparation-and-app-assessment-states.md)

## Context

Shell preparation already pairs each Action and payload in a `PreparedShellStep`. Inside a profile
step, however, it independently appended journal file contracts and executable file records, then
zipped those lists during apply. Paths, ownership and modes had multiple initialization sites.
Candidate C1 removes that local correspondence without generalizing other resource preparation.

## Decision

The private `PreparedShellProfileFile` stores a `ShellProfileFileV1` contract and optional desired
bytes. Preparation appends one record per file after the existing observations and checks. At the
original Action construction point it projects complete contract clones into the journal Action.
Apply iterates those same prepared records; paths, previous identity, ownership, sentinel hashes
and desired mode come from the contract, while write content comes from the private bytes.

Keep the Action variant check, permission validation, journal publication, observation order,
locks, receipt checks and applied-marker saves. The payload remains memory-only; journal formats,
Plan fingerprints, public inputs and recovery behavior do not change. Cache/removal pairing and
all other resource preparation remain outside this decision.

## Consequences

The profile branch no longer relies on two lists having the same length and order. Existing types
and modules remain; contract cloning still produces a second physical copy, read-only during apply.
Preparation must still bind desired bytes to their hash and mode. Pairing is not proof of byte
equality and does not replace live-state or recovery checks. Mixed-resource interruption tests
retain journal order, payload exclusion, rollback and retry coverage, with profile content/mode
checks; profile-only removal and preservation of unrelated edits retain separate regressions.
