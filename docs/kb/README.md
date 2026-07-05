# Knowledge Base

This directory is the **AI-consumable knowledge base** for the `shine` repository. It records the
knowledge that cannot be re-derived from the code alone: why decisions were made, invariants that
must not be broken, operational runbooks, and lessons learned from past bugs.

It complements — and never duplicates — the existing docs:

| Document | What it covers |
|---|---|
| [`AGENTS.md`](../../AGENTS.md) | Build/test/lint commands, module map, command routing, preset authoring, hard rules (push policy, release rules) — the authoritative agent entry point (`CLAUDE.md` imports it) |
| [`README.md`](../../README.md) | User-facing documentation (features, installation, command usage) |
| [`CHANGELOG.md`](../../CHANGELOG.md) | Hand-written, user-facing release history |
| **`docs/kb/`** (this directory) | Decisions, invariants, data flows, runbooks, lessons — the non-derivable knowledge |

## Map

- [`architecture/data-flows.md`](architecture/data-flows.md) — end-to-end flows that span multiple modules
- [`architecture/invariants.md`](architecture/invariants.md) — non-obvious invariants that must hold
- [`decisions/`](decisions/) — ADR-lite records (one decision per file, numbered)
- [`conventions.md`](conventions.md) — commit, versioning, and testing conventions
- [`operations/release-runbook.md`](operations/release-runbook.md) — how to cut a release
- [`operations/ci-pipelines.md`](operations/ci-pipelines.md) — what each GitHub Actions workflow does
- [`operations/troubleshooting.md`](operations/troubleshooting.md) — common failures and diagnosis
- [`lessons.md`](lessons.md) — dated symptom → root cause → rule entries mined from real bugs

## How to use this KB (for AI agents)

1. **Before changing behavior** in an area, check `architecture/invariants.md` and grep
   `lessons.md` for the module you are touching.
2. **Before proposing a design**, check `decisions/` — the choice may already have been made,
   along with its rationale.
3. **For release or CI work**, follow `operations/`.

## How to update this KB (maintenance protocol)

Update the KB in the same PR as the change it documents:

- Fixed a bug caused by non-obvious behavior → add an entry to `lessons.md`.
- Made a design/architecture choice → add a numbered ADR in `decisions/`.
- Changed a data flow or invariant → update the matching file in `architecture/`.
- Structural refactor (modules moved/renamed) → sync `AGENTS.md` (existing convention).

Keep entries short and factual. Cite commits by hash where relevant. Delete entries that become
wrong rather than letting them rot.
