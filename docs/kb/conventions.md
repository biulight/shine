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
- GitHub Release notes also skip documentation-only commits written as `docs:`, `docs(scope):`,
  or `fix(docs):`, plus `chore(release)` preparation commits. Feature and bug-fix commits remain
  included even when they update public docs in the same change.
- Pre-commit runs `cargo fmt --check`, `clippy -D warnings`, `cargo deny check`, `typos`, and
  `cargo nextest run --workspace`; it validates `mise.toml`, and changes to Bun tooling or TypeScript sources
  additionally run `mise exec -- bun run --bun check:ts` (strict type-check + Bun tests). All must pass
  locally before a commit lands.
- **Never `git push` without explicit user approval** (`AGENTS.md` § Hard repository rules).

## Versioning

- Baseline for any version-bump decision is the **latest stable `v*` tag** — never the moving
  `preview` tag. Find it with:
  `git tag --list 'v*' --sort=-version:refname | head -1`
- Follow Semantic Versioning for the published CLI and `shine-core`: incompatible public CLI,
  configuration, automation, or library-contract changes → major; backward-compatible
  user-facing features → minor; only backward-compatible user-facing fixes → patch.
- **2.4.0 release exception (2026-10-11):** the maintainer chose a minor release for removing the
  optional HTTP service and changing env precedence and write targets. Keep the current behavior,
  disclose incompatibilities and manual migration in CHANGELOG and both manual locales, and retain
  `clear serve` for legacy service cleanup. This exception does not change the general rule above.
- **2.6.0 release exception (2026-10-22):** the maintainer chose a minor release for the limited
  CLI compatibility change that removes `--purge` and includes internal cache cleanup in default
  uninstall. Most work is internal refactoring and bug fixes; persistent secret-file export and
  cargo-binstall support justify a minor rather than patch release. Keep the implemented behavior
  and disclose the cleanup scope and removal of `--purge` from scripts in CHANGELOG and both manual
  locales. This exception does not change the general versioning rule above.
- **2.7.0 release exception (2026-10-24):** the maintainer chose a minor release for structured
  Preset environment inputs and App code analysis despite incompatible metadata, report, Rust
  library source and admission changes. New fields in public `shine-core` structs require consumers
  to adapt direct initializers and exhaustive patterns. Legacy string `env` arrays require explicit
  migration; definite code differences and analysis failures can block previously executable scripts.
  Plan/approval/review and author Plan schemas become 3; lint and fixture report schemas become 2, while fixture input
  schema 1 stays supported. Source builds require Rust 1.90. Disclose these changes and the manual
  migration path in CHANGELOG and both manual locales; preserve existing runtime state, trust
  requirements and non-blocking coverage gaps. This exception does not change the general rule above.
- **2.8.0 release exception (2026-11-01):** the maintainer chose a minor release for managed
  App secret files, local SSH file authorization, and installer UX improvements despite incompatible
  SSH protocol v4, file-policy format v2, App manifest v2, Plan/approval/review schema v4, and public
  Rust type changes. Upgrade both SSH peers, explicitly migrate file policies, renew approvals,
  and adapt Rust consumers. Older clients cannot read newly written App state; downgrading the
  binary does not reverse migrations. Disclose these boundaries in CHANGELOG and both manual
  locales. This exception does not change the general versioning rule above.
- A versioned release candidate uses `X.Y.Z-rc.N`. It is a reproducible prerelease, does not
  replace the latest stable release, and does not trigger the stable `release` → `main` sync.
- Generated RC notes cover the immediately preceding versioned tag; generated stable notes cover
  the previous stable tag through the new stable tag, consolidating all intervening RC work into
  one stable release section.
- Release commits use `chore(release): ...` (e.g. `chore(release): prepare v0.35.0`).
- Keep `shine-cli` and `shine-core` crate versions in sync (see commit `e14d5f9`).
- The minimum supported Rust version is 1.90. The `mise.toml` toolchain may be newer for
  development, but changes to dependencies must keep the Rust 1.90 CI check passing.

## Testing

- Runner: `cargo nextest run --workspace --all-features` (each test is its own OS process — in-process
  locks do not serialize across tests).
  `.config/nextest.toml` requires nextest 0.9.145 or newer to avoid macOS sibling capture-pipe
  inheritance. Update older runners with `cargo nextest self update`. Leaked output handles fail
  the run at the unchanged 100 ms timeout; do not hide them with longer waits or ignored tests.
- Any test that mutates environment variables or cwd must hold `crate::test_support::env_lock()`;
  the guard restores both before unlocking, including during panic unwinding.
  This serializes participating tests only: unguarded ambient-env reads and PATH-based child
  launches can still race under parallel `cargo test`. Use nextest process isolation for the full
  suite; restoration alone does not make those readers safe.
- Any test that performs privileged (sudo) file operations on real paths must hold the
  cross-process admin lock for its full body (`install_core/file_ops.rs`, commit `fbd9c55`).
- Verify CLI behavior against an isolated config dir:
  `SHINE_CONFIG_DIR=$PWD/.tmp-home/.shine` (details in `AGENTS.md` § Verification boundaries).
- CI additionally runs `cargo audit`; a new dependency with a RUSTSEC advisory fails the build
  (see lessons entry on quinn-proto).
- Rust and Bun versions have a single source of truth in root `mise.toml`; local development and CI
  must use those mise-managed tools. Root-level Bun development dependencies are locked by
  `bun.lock`; CI installs them with `bun install --frozen-lockfile` before running
  `bun run --bun check:ts`. Built-in runtime preset scripts remain self-contained and may import only
  relative modules plus Bun/Node built-ins. External or overlay Bun scripts may use dependencies
  only through a category-local committed `package.json` + `bun.lock` pair; see ADR 0031.

The local Bun hook covers every `presets/**/*.ts` file and its tooling configuration. The
`presets/tooling.test.ts` regression checks the actual hook filter against the maintained files.

Built-in preset contracts live in the single `builtin_presets` integration target, outside the
runtime resource tree. Run it alone with `cargo test --test builtin_presets --target-dir target`,
or through the existing workspace nextest command. Script tests remain colocated and are discovered
by `bun test ./presets`. The existing Rust hook also matches all preset files and Cargo/toolchain
configuration; the Bun tooling regression checks both hook filters. Source packages include the
integration target and its support modules. Root-anchor include paths and exclude `node_modules`
even under allowed trees; `source_package` tests this with Cargo against synthetic file trees. Public-manual parity checks retain their source-package
skip when the manuals are absent. Native CI remains necessary for platform-gated cases.

## Refactoring

Every refactor must answer: compared with keeping the current implementation, does this make
future understanding, modification and verification easier, and what evidence supports that?

- Confirm the actual release tag, commit and branch before a version-based review; Cargo versions
  and moving preview tags are not release baselines. If the release cannot be confirmed, agree on
  the range first. Include subsequent fixes, tests, docs and related uncommitted work; compare both
  each change's before/after and the release baseline with the final state.
- Review read-only first. For each item recommend retain, simplify, partially revert, revert or
  defer for insufficient evidence. Record the original problem, actual benefit, added cost, simpler
  alternative, required coverage and implementation order. Obtain authorization for implementation.
- Report additions, deletions and net changes separately for production, tests/fixtures/support and
  docs, by actual purpose (including inline `cfg(test)`). Count modules, types, forwarding functions,
  policy parameters, call layers and locations changed for one rule. Moves, macros, generated code
  and externalized implementations do not remove complexity; formatting compression is not savings.
- Prefer dead-code removal, narrower visibility and existing interfaces. Verify platform, dynamic,
  public API, old-format and recovery responsibilities before declaring code unused. Share a stable
  rule only when its consumers have the same semantics and evolution needs. Local duplication is
  preferable to extra adapters, switches or transport types with no demonstrated benefit.
- Map tests by entry point, scenario, risk and assertions. Remove only redundant coverage; record
  where every necessary assertion remains. Preserve distinct consumers and failure boundaries.
  Reuse existing fixtures and fault injection, use small local setup helpers, and avoid large
  sequential tests, copied legacy implementations as permanent oracles, or generic test frameworks.
  Temporary differential tools need a separate justification before becoming permanent fixtures.
- Preserve command/output/error contracts, snapshot-bound approvals, lock order/scope and rechecks,
  receipt/manifest/journal formats and persistence order, ownership/user edits, dry-run/force/admin
  differences, backups/failure/cancellation/retry, raw bytes versus normalized values, platform and
  recovery compatibility. Separate introduced regressions, pre-existing defects and evidence gaps;
  do not mix independent behavior changes into cleanup.
- Implement high-confidence small changes first. Reassess if actual cost exceeds expected benefit.
  Net growth needs a concrete explanation and rejected simpler alternatives; net reduction must not
  lose behavior or coverage. There is no mandatory negative-line-count threshold.
- Validate according to risk and repository checks, using matching before/after scenarios for key
  boundaries. Distinguish local tests, cross-target compilation, native platforms and external
  systems. Report failures and unverified boundaries, then finish with `git diff --check` and
  `git status --short`. Do not repeat passing suites without a changed input or unresolved concern.
- Keep lasting decisions in existing KB/ADRs, not repeated execution diaries. Preserve unrelated
  work and honor the separate authorization boundaries for edits, commits and pushes.

The accepted `let ... else` cleanup at `e3d1c165` illustrates this scope: simplify direct
`Some`/`Ok` bindings with unchanged early returns, retain complex action-enum matches, and review
crate publication changes separately.

## File size

File size is a navigation guideline, not a reason by itself to split code. Test/support code still
counts toward total maintenance cost under the refactoring review above.

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

Follow [`preset-authoring.md`](preset-authoring.md). Key rules: prefer `shine.toml` metadata over
legacy `shine-dest:` annotations and declare transforms in order. Sys manifests declare `version =
2`; every init item uses standard detection/provider metadata or one item-local script with a
normal exit code. Do not add platform-wide dispatch or status/update wire protocols.

## Installation, migration, and release documentation

Keep `installation.md` focused on installing and updating the binary. Place
`guides/upgrading.md` and the dedicated 1.x migration guide under Guides, leaving installation
and quick start adjacent in the top-level navigation. Link existing users to migration guidance
from the installation page. Preserve the migration page's `/upgrading` slug when moving its source.

Reserve the migration guide for concrete preparation that runtime prompts cannot replace, such
as updating scripts, converting external Preset inputs, or adapting changed configuration
precedence. Group by starting version and affected feature; requirements accumulate when skipping
releases. A safe runtime rejection alone does not justify a mandatory pre-upgrade checklist.
Keep SSH peer mismatches in the SSH guide, saved approval/report compatibility in the authoring
guide, and App downgrade limits in the App guide. Coordinate automation upgrades only when
availability requires it. Omit routine prompts, automatic cache rebuilding, and no-migration notices.

Do not prescribe backups merely because a version or state format changes. A backup recommendation
must identify the specific information that would otherwise be lost, the operation that loses it,
and how restoration works. Prefer existing automatic metadata backups or a verified reversible
workflow where applicable; do not require an extra copy by default. Conversely, do not equate
undoing managed effects or resealing ciphertext with downgrading installation records, trust, or
recovery state. Explain verified compatibility limits without inventing a rollback recipe or
turning an unverified downgrade path into an upgrade prerequisite.

Feature announcements and fix summaries belong in `CHANGELOG.md` and GitHub Releases; link to
release notes instead of creating a manual section for every release. Keep both locales aligned
and preserve existing installation fragment links when moving migration guidance. Editorial
corrections to released behavior should also reach the latest stable documentation snapshot;
leave historical major versions unchanged.

### Command examples

Keep explanations outside commands intended for interactive shells. Default interactive Zsh passes
`#` and subsequent words as arguments. Use prose for sequences and a purpose/command table for
alternatives. Explain prerequisites and destructive effects before the command.
Do not repeat the same short explanation in a paragraph below the block, or annotate obvious
single commands merely for uniformity.

Label alternatives as choices before the block, including install scopes, update channels, trust
modes, and set/delete or enable/disable operations. Do not present equivalent commands as mandatory
consecutive steps. For real workflows, preserve the order and explicit review boundaries between
preview and mutation. Retain local/remote and shell-specific labels where execution context changes.
Keep comments short and readable in both locales. Never append a comment after a shell continuation
backslash, inside a quoted argument, or to a format that has no comment syntax; use a preceding
explanation outside the code block for multiline commands. Check that edits preserve executable arguments and do not
turn examples into automatic approval or execution instructions.
