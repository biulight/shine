# AGENTS.md

`shine` bundles shell, app, and OS bootstrap presets into a Rust CLI with manifest-tracked
installation and uninstall. The root package is `shine-cli` (sources in `cli/`); `core/` is
`shine-core`. Keep this entry point focused on repository rules; route details to the KB.

## Workflow and authorization

- Complete authorized edits and verification using context for routine choices. Ask only when
  missing information changes the outcome or an action needs authorization not already given;
  continue independent work while resolving it.
- User instructions take precedence over skill workflow preferences. Skills and command examples
  do not grant permission to push, activate presets, execute external code, or bypass Shine's
  ownership, trust, and snapshot-bound approval checks. If an instruction blocks work, cite its
  file and exact requirement and explain what remains blocked.
- Preserve unrelated working-tree changes; do not overwrite or clean files you do not own.
- Before a non-trivial change, read the relevant sections of
  [invariants](docs/kb/architecture/invariants.md) and search [lessons](docs/kb/lessons.md) for the
  affected behavior. Check [ADRs](docs/kb/decisions/) before proposing a design; consult
  [data flows](docs/kb/architecture/data-flows.md) for changes spanning modules.
- Update stale KB material in the same change using the
  [maintenance protocol](docs/kb/README.md#how-to-update-this-kb-maintenance-protocol).

## Hard repository rules

- **Never run `git push` without explicit user approval**, including branch, tag, and force pushes.
  A request to commit is not permission to push; retain approval already given for the same action.
- Work lands on `release`; `main` receives only automated post-release sync PRs
  ([ADR 0001](docs/kb/decisions/0001-release-branch-model.md)).
- `CHANGELOG.md` is hand-written. Never generate it with `git cliff`; the release workflow uses
  git-cliff only for the GitHub Release body.
- Version decisions use the latest stable `v*` tag, never the moving `preview` tag; follow
  [versioning conventions](docs/kb/conventions.md#versioning).
- Internal fixes caused by new code in the same release use the git-cliff-skipped scopes in
  [commit conventions](docs/kb/conventions.md#commits). Real user-facing fixes use their feature area.
- User-visible behavior changes must update both public manual locales in the same release change:
  `docs/manual/` and `website/i18n/zh-Hans/docusaurus-plugin-content-docs/current/`. Keep doc IDs,
  page sets, commands, identifiers, examples, and warnings semantically aligned.
- Never publish `docs/kb/`, PRDs, release runbooks, private paths, or internal procedures through
  the public site. Root READMEs are summaries, not a second command/configuration reference.

## Read when relevant

| Task | Authoritative reference |
|---|---|
| Locate modules or command handlers | [Module map](docs/kb/architecture/module-map.md) |
| Change cross-module behavior or safety contracts | [Data flows](docs/kb/architecture/data-flows.md), [invariants](docs/kb/architecture/invariants.md) |
| Author or review presets | [Preset authoring](docs/kb/preset-authoring.md); [portable authoring skill](skills/shine-preset-author/SKILL.md) for isolated authoring |
| Change declarative actions or recovery | [Design](docs/declarative-action-recovery-prd.md), [executable inventory](docs/kb/executable-preset-inventory.md) |
| Assess platform coverage | [Platform support](docs/kb/architecture/platform-support.md) |
| Commit, version, or test conventions | [Conventions](docs/kb/conventions.md) |
| Release, CI, or diagnostics | [Operations](docs/kb/operations/), [troubleshooting](docs/kb/operations/troubleshooting.md) |
| Maintain internal documentation | [KB protocol](docs/kb/README.md) |

## Commands

Use the versions pinned in `mise.toml` (`mise exec -- <command>` or activate mise). Run
`mise install` if required tools are missing. Install Bun dependencies with
`bun install --frozen-lockfile` before editing TypeScript presets.

```bash
cargo build --target-dir target
cargo nextest run --target-dir target --all-features
cargo test --target-dir target                         # fallback or targeted test filter
cargo nextest run --target-dir target -E 'test(install_then_uninstall)'
cargo fmt --check
cargo clippy --target-dir target --all-targets --all-features --tests --benches -- -D warnings
cargo deny check bans licenses sources
typos
bun run check:ts                                      # typecheck + preset tests
```

For public manual or website changes, run in `website/`:

```bash
pnpm install --frozen-lockfile
pnpm check:locales
pnpm typecheck
pnpm build
```

## Verification boundaries

- Select checks for the changed behavior and risk. Instruction-only or internal Markdown edits
  need link, consistency, and applicable format checks; skill edits also need frontmatter and
  reference validation. They do not require Rust/Bun suites or a public-site build by themselves.
  Applicable area checks and configured pre-commit hooks must pass before committing;
  [`.pre-commit-config.yaml`](.pre-commit-config.yaml) defines the hook file filters.
- Add tests for meaningful behavior or safety regressions. Once required checks pass, repeat or
  broaden them only for changed code, failures, or unresolved concerns.
- In a sandbox, add `--target-dir target` to Cargo commands that build artifacts.
- Most CLI commands, including `preset new` and `preset copy`, can initialize config. Keep
  ad-hoc checks under a fresh temporary `SHINE_CONFIG_DIR`, set per command. `preset validate`,
  `lint`, `plan`, `test`, and `schema` route before config initialization and do not execute presets.
- `SHINE_CONFIG_DIR` overrides both Shine state and presets; runtime presets live under
  `$SHINE_CONFIG_DIR/presets/`. `SHINE_PRESETS` overrides only presets. Copy the category under test
  into the isolated tree before runtime list/info/dry-run checks. For pristine embedded metadata,
  use a snapshot-based test; do not drop isolation to recover built-in discovery.
- Metadata-driven App changes need a targeted metadata/destination test plus isolated `app list`,
  `app info <category>`, and `app install <category> --dry-run`. See the
  [authoring guide](docs/kb/preset-authoring.md#app-verification) for setup.

Finish with `git diff --check` and `git status --short`. Report the changes, checks actually run,
pre-existing failures, and unverified platform or external behavior concisely in the user's language.
