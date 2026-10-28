# 0154 — Share Sys profile code classification, retain consumer gates

- **Status**: Accepted
- **Date**: 2026-10-29
- **Extends**: [ADR 0091](0091-derived-code-boundaries-and-author-capability-statements.md)

## Context

Trust requirements, profile/bootstrap planning, profile composition and bundle reporting repeated
the same classification of `SysShellIntegration`. Changing that rule required four synchronized
edits. The consumers have different selection and authorization responsibilities; sharing those
flows would hide necessary boundaries. Architecture review candidate D1 is now authorized as a
local change; the other deferred candidates remain separate.

## Decision

The existing model owns a runtime-only `has_executable_code` query: nonempty eval argv, a source,
or a fragment denotes code. PATH, environment and aliases retain their existing classification.
Presence checks remain exact, including `Some("")`; metadata validation still owns admissibility.

All four consumers call that query at their original decision point. Trust and planning retain
their target/category and enabled-item rules; composition filters enabled items and matching
shells before its code gate; pack examines the full declaration and separately includes script
installers. External base profiles, source layers, grants and fresh execution checks remain local.
The query neither performs I/O nor grants authority. No new model, public API or storage format is
introduced.

## Consequences

One existing type now owns the classification, at the cost of one internal method and a lookup
when reading consumers. This reduces rule synchronization, not consumer validation obligations.
The consumer regression covers all six integration forms through trust, planning, composition
preflight and packing, including nonmatching-shell and disabled-item cases. Existing approval,
bootstrap, base-profile and recovery tests remain independent. No command behavior changes.
