# Conventions

Repository-specific conventions. Build/test/lint commands live in [`AGENTS.md`](../../AGENTS.md).

## Commits

- Conventional commits: `type(scope): description` — types seen in history: feat, fix, refactor,
  docs, chore.
- Fixes that only exist because of new code **in the same release** (lint noise, formatting,
  typos, build breakage) must use one of the git-cliff-skipped scopes below, so they are
  excluded from generated release notes. Real user-facing fixes use the feature area scope
  instead (`fix(apps)`, `fix(sys)`, `fix(install)`, …).

  | Scope | Example |
  |-------|---------|
  | `fix(lint): ...` | clippy allow/deny rule adjustment |
  | `fix(clippy): ...` | clippy suggestion |
  | `fix(fmt): ...` | rustfmt formatting |
  | `fix(typo): ...` | spell-check fix in new code |
  | `fix(build): ...` | build/compile error in new code |
  | `fix(ci): ...` | CI pipeline fix |
  | `fix(internal): ...` | any other non-user-facing cleanup |
- Pre-commit runs `cargo fmt --check`, `clippy -D warnings`, `cargo deny check`, `typos`, and
  `cargo nextest run`; it validates `mise.toml`, and changes to Bun tooling or TypeScript sources
  additionally run `mise exec -- bun run check:ts` (strict type-check + Bun tests). All must pass
  locally before a commit lands.
- **Never `git push` without explicit user approval** (`AGENTS.md` § Git Push Policy).

## Versioning

- Baseline for any version-bump decision is the **latest stable `v*` tag** — never the moving
  `preview` tag. Find it with:
  `git tag --list 'v*' --sort=-version:refname | head -1`
- Count/inspect the commits since that tag to decide the bump: user-facing features → minor;
  only user-facing fixes → patch.
- Release commits use `chore(release): ...` (e.g. `chore(release): prepare v0.35.0`).
- Keep `cli` and `utils` crate versions in sync (see commit `e14d5f9`).

## Testing

- Runner: `cargo nextest run --all-features` (each test is its own OS process — in-process
  locks do not serialize across tests).
- Any test that mutates environment variables must hold `crate::test_support::env_lock()`.
- Any test that performs privileged (sudo) file operations on real paths must hold the
  cross-process admin lock for its full body (`install_core/file_ops.rs`, commit `fbd9c55`).
- Verify CLI behavior against an isolated config dir:
  `SHINE_CONFIG_DIR=$PWD/.tmp-home/.shine` (details in `AGENTS.md` § Verification Notes).
- CI additionally runs `cargo audit`; a new dependency with a RUSTSEC advisory fails the build
  (see lessons entry on quinn-proto).
- Rust and Bun versions have a single source of truth in root `mise.toml`; local development and CI
  must use those mise-managed tools. Root-level Bun development dependencies are locked by
  `bun.lock`; CI installs them with `bun install --frozen-lockfile` before running
  `bun run check:ts`. Runtime preset scripts remain self-contained and must not import packages
  from `node_modules`.

## File size

- The ~800-line file-size guideline is measured by **production code only** — everything before
  the file's `#[cfg(test)] mod tests { ... }` block. Inline `#[cfg(test)]` unit tests (the Rust
  convention, and this repo's own testing rules) do not count toward it; a file can have any
  number of test lines below that boundary without needing a split on size grounds alone.
- Files that are pure test modules (e.g. `cli/src/ssh/integration_tests.rs`) are exempt entirely.
- Decided 2026-07-11 when auditing files over 800 total lines after the Phase 3 module-split
  refactor (commits `63a8f180`..`1ebfbec9`, continued in `5a28dd67`/`9601f93a`/`5eb58d43`): most
  of the remaining "oversized" files turned out to be well under 800 lines of actual production
  code, with the excess being inline tests. Only `sys/commands.rs`, `sys/profile.rs`, and
  `update_check.rs` had genuinely oversized production code, and those were split further.

## Preset authoring

Follow `AGENTS.md` § "Adding a new preset category". Key rules: prefer `shine.toml` metadata over
legacy `shine-dest:` annotations; declare `transforms` in order; sys scripts report progress via
`SHINE_SYS_STATUS` tab-separated events.
