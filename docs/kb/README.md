# Knowledge Base

This directory is the **AI-consumable knowledge base** for the `shine` repository. It records the
knowledge that cannot be re-derived from the code alone: why decisions were made, invariants that
must not be broken, operational runbooks, and lessons learned from past bugs.

It complements — and never duplicates — the existing docs:

| Document | What it covers |
|---|---|
| [`AGENTS.md`](../../AGENTS.md) | Shared workflow, authorization, commands, and verification; `CLAUDE.md` and Copilot instructions route here |
| [`README.md`](../../README.md) | Product and installation summary; detailed command usage belongs in the bilingual manual |
| [`CHANGELOG.md`](../../CHANGELOG.md) | Hand-written, user-facing release history |
| **`docs/kb/`** (this directory) | Decisions, invariants, data flows, runbooks, lessons — the non-derivable knowledge |

## Map

- [`architecture/data-flows.md`](architecture/data-flows.md) — end-to-end flows that span multiple modules
- [`architecture/invariants.md`](architecture/invariants.md) — non-obvious invariants that must hold
- [`architecture/module-map.md`](architecture/module-map.md) — module ownership and command routing
- [`architecture/platform-support.md`](architecture/platform-support.md) — macOS, Ubuntu, and Windows capability matrix, gaps, and implementation priorities
- [`preset-authoring.md`](preset-authoring.md) — shell, app, and sys preset authoring rules
- [`executable-preset-inventory.md`](executable-preset-inventory.md) — Phase 4 execution, privilege, provenance, and rollback classification
- [`decisions/`](decisions/) — ADR-lite records (one decision per file, numbered)
- [`conventions.md`](conventions.md) — commit, versioning, and testing conventions
- [`operations/release-runbook.md`](operations/release-runbook.md) — how to cut a release
- [`operations/ci-pipelines.md`](operations/ci-pipelines.md) — what each GitHub Actions workflow does
- [`operations/troubleshooting.md`](operations/troubleshooting.md) — common failures and diagnosis
- [`lessons.md`](lessons.md) — dated symptom → root cause → rule entries mined from real bugs

## How to use this KB (for AI agents)

Follow the [root workflow](../../AGENTS.md#workflow-and-authorization), then read the references
for the affected area. For release or CI work, use `operations/`; historical ADRs explain decisions
and do not grant permission to execute their example commands.

## How to update this KB (maintenance protocol)

Update the KB in the same PR as the change it documents:

- Fixed a bug caused by non-obvious behavior → add an entry to `lessons.md`.
- Made a design/architecture choice → add a numbered ADR in `decisions/`.
- Changed a data flow or invariant → update the matching file in `architecture/`.
- Structural refactor (modules moved/renamed) → sync `architecture/module-map.md`.

Keep entries short and factual. Cite commits by hash where relevant. Delete entries that become
wrong rather than letting them rot.
