# Knowledge Base

This directory is the **shared engineering knowledge base for maintainers and AI agents** working
on `shine`. Every entry must be understandable and reviewable by a human. It records the
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
- [`operations/maintainability-audit-2026-10-27.md`](operations/maintainability-audit-2026-10-27.md) — closed Rust maintenance audit; baseline evidence and frozen historical batches
- [`operations/architecture-necessity-review-2026-10-27.md`](operations/architecture-necessity-review-2026-10-27.md) — closed review and historical evidence; subsequent split-DNS verification and authorized D1 implementation
- [`operations/env-configuration-validation.md`](operations/env-configuration-validation.md) — native Windows env layering, replacement and stable-binary downgrade evidence
- [`lessons.md`](lessons.md) — dated symptom → root cause → rule entries mined from real bugs

## How to use this KB

Follow the [root workflow](../../AGENTS.md#workflow-and-authorization), then read the references
for the affected area. For release or CI work, use `operations/`; historical ADRs explain decisions
and do not grant permission to execute their example commands.

Read conclusions and their limits in Markdown before consulting supporting artifacts. Historical
reviews describe their stated baseline; check the owning source before treating them as current
behavior. Humans and agents use the same authoritative explanation, organized by topic and purpose.

## How to update this KB (maintenance protocol)

Update the KB in the same PR as the change it documents:

- Fixed a bug caused by non-obvious behavior → add an entry to `lessons.md`.
- Made a design/architecture choice → add a numbered ADR in `decisions/`.
- Changed a data flow or invariant → update the matching file in `architecture/`.
- Structural refactor (modules moved/renamed) → sync `architecture/module-map.md`.

Keep entries short and factual. Cite commits by hash where relevant. Delete entries that become
wrong rather than letting them rot.

Routine test results belong in delivery reports or PRs. Keep validation evidence in the KB only
when it explains a design choice, compatibility boundary, risk, or reproducible failure. Prefer a
short conclusion with a direct commit/CI link or a maintained reproduction entry point. Do not
append per-commit pass tables, suite counts, timings, temporary archive hashes, environment retry
logs, or validation of commits whose only purpose was to record earlier validation. Historical
execution details remain available in Git history, PRs, and CI.

### Explanations and supporting artifacts

- Keep decisions, rationale, alternatives, compatibility boundaries and verification limits in the
  owning Markdown page. Readers must understand the conclusion without opening a data attachment.
  Use short prose and tables; JSON is not needed merely because an AI agent may read the material.
- Organize by responsibility, not by reader: use `architecture/`, `decisions/` and `operations/`
  for shared knowledge. Do not create parallel human/AI trees or manually maintain the same
  conclusion in Markdown and JSON.
- Keep structured data under a topic's `evidence/` directory only when a named tool consumes it
  or it is necessary to reproduce a durable finding. The owning page must explain its purpose,
  baseline, consumer or reproduction command, and limitations. Generated data should have a
  reproducible generation entry point; a standalone hash or line-number dump is not a reproduction.
- Reusable reproduction scripts may remain with their owning audit when they explain a real
  failure or safety boundary. Document their inputs, isolation and expected result. Do not run
  them merely to tidy documentation; normal execution and authorization rules still apply.
- Before retaining an artifact, ask whether it changes a decision, explains a boundary, or enables
  useful reproduction. Remove duplicate narratives and transient logs, counters and probe hashes
  that serve none of those purposes. Keep routine run output in PRs or CI.
- When consolidating existing evidence, move unique explanations and necessary provenance into
  the owning page, retain uncertainty and historical status, remove stale links, and use Git history
  for retired detail. Do not convert an entire JSON dump into a Markdown dump or add another ledger
  solely to document the cleanup.
