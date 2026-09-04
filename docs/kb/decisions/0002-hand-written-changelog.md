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

Release-candidate notes are incremental from the immediately preceding versioned tag. Stable
release notes instead start at the previous stable tag, so users upgrading between stable releases
receive a complete feature and fix inventory even when the work passed through one or more RCs.
Intervening RC tags are ignored as section boundaries so the stable notes remain one release entry.
Release-preparation commits are excluded from both forms.

## Consequences

- Never run `git cliff` against `CHANGELOG.md`.
- Commit messages still matter: git-cliff builds release notes from them, and the
  `fix(lint|clippy|fmt|typo|build|ci|internal)` scopes plus `chore(release)` are auto-skipped (see
  `conventions.md`).
- Documentation-only `docs`, scoped `docs(...)`, and `fix(docs)` commits are also skipped from
  GitHub Release notes; user-facing feature and fix commits remain eligible even when they include
  documentation changes.
