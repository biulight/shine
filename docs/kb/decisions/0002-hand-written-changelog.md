# 0002 — CHANGELOG.md is hand-written; git-cliff only generates GitHub Release notes

- **Status**: accepted
- **Evidence**: `AGENTS.md` § Hard repository rules, `cliff.toml`, `release.yml` git-cliff step

## Context

Conventional-commit-generated changelogs read like commit logs, not user documentation. But
automated release notes on GitHub are still convenient.

## Decision

`CHANGELOG.md` is written manually per release, in plain English, user-facing, grouped under
Features / Bug Fixes / Internal / Docs. `release.yml` separately runs git-cliff to produce the
GitHub Release body from conventional commits. The two artifacts coexist and may describe the
same release in different words — neither overwrites the other.

## Consequences

- Never run `git cliff` against `CHANGELOG.md`.
- Commit messages still matter: git-cliff builds release notes from them, and the
  `fix(lint|clippy|fmt|typo|build|ci|internal)` scopes are auto-skipped (see
  `conventions.md`).
- Documentation-only `docs`, scoped `docs(...)`, and `fix(docs)` commits are also skipped from
  GitHub Release notes; user-facing feature and fix commits remain eligible even when they include
  documentation changes.
