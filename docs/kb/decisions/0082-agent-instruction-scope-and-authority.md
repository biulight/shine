# 0082 — Agent instructions share authority and load task-specific detail

- **Status**: accepted
- **Date**: 2026-09-05
- **Evidence**: `AGENTS.md`, `.github/copilot-instructions.md`, `skills/shine-preset-author/`,
  `cli/src/main.rs`, `docs/kb/preset-authoring.md`

## Context

The instruction audit found a second repository rule set in Copilot instructions, including
obsolete `utils/` paths and unconditional pre-commit claims. The authoring skill required schema
output for every task, left outcome confirmation ambiguous, forced exactly one preset kind, and
isolated only its final dry-run even though `preset new`/`copy` initialize config. Its App reference still prescribed
nested artifact `--yes`, contradicting [ADR 0076](0076-script-hooks-share-the-parent-app-plan.md).

The [GPT-6 Astra official guidance](https://developers.openai.com/api/docs/guides/latest-model?model=gpt-6-astra#prompting-best-practices)
motivates this audit: conflicting file instructions can stall work, routine choices need not
require clarification, and verification should match the change. Model advice does not grant
repository or runtime permissions.

## Decision

Extend [ADR 0029](0029-agents-entrypoint-context-budget.md) and clarify the workflow in
[ADR 0033](0033-skill-first-ai-preset-authoring.md):

- `AGENTS.md` owns shared workflow and repository rules; Claude and Copilot route there.
- The portable skill owns isolated authoring. Read a reference for each affected kind, and load
  fixture/bundle/dry-run details when needed. Fetch generated schema only for contracts that need it;
  the installed templates and runtime validator remain authoritative for preset TOML.
- Authorized authoring proceeds from available context. Ask for material missing choices or
  missing authorization, identify the exact blocking rule, and preserve prior scoped approval.
- Isolate scaffolding and runtime dry-runs. Authoring never grants trust, activates presets,
  executes preset code, or turns a hypothetical report into an approved runtime Plan.
- Instruction-only checks cover structure, references, consistency, and changed command guidance.
  Source changes retain their area checks; commit hooks retain their configured file filters.

## Consequences

Push approval, release routing, hand-written changelogs, bilingual manual parity, public/private
documentation boundaries, and runtime ownership/trust/approval invariants remain intact. Optional
procedures no longer force unrelated work. No model selection, global skill installation, client
configuration, or product runtime behavior changes are part of this instruction maintenance.
