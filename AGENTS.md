# AGENTS.md

This file is the repository entry point for AI coding agents. Keep it concise: mandatory workflow,
high-frequency commands, and pointers to authoritative detail belong here; architecture narratives
and feature-specific authoring guidance belong in `docs/kb/`.

`shine` is a self-contained Rust CLI that bundles shell scripts, app config presets, and OS
bootstrap presets into one binary (`rust-embed`), installs them under `~/.shine/`, and supports
safe, manifest-tracked uninstall. The workspace root is the publishable `shine-cli` package
(binary plus the `cli` library); `utils/` is the reusable `shine-core` package.

## Required workflow

1. Before any non-trivial change, read [`architecture/invariants.md`](docs/kb/architecture/invariants.md)
   and grep [`lessons.md`](docs/kb/lessons.md) for the modules or behavior you will touch.
2. Before proposing a design, check [`decisions/`](docs/kb/decisions/) for an existing ADR.
3. Preserve unrelated working-tree changes. Do not overwrite or clean files you do not own.
4. Update the KB in the same change whenever code makes it stale:
   - non-obvious bug cause → `lessons.md`
   - design choice → numbered ADR under `decisions/`
   - changed data flow or invariant → the matching file under `architecture/`
   - moved or renamed modules → `architecture/module-map.md`
5. User-visible behavior changes must update both public manual locales in the same release change:
   - English source: `docs/manual/`
   - Simplified Chinese: `website/i18n/zh-Hans/docusaurus-plugin-content-docs/current/`
   Keep doc IDs, page sets, commands, identifiers, examples, and warnings semantically aligned.
6. Do not publish `docs/kb/`, PRDs, release runbooks, private paths, or internal procedures through
   the public site. Root READMEs are summaries, not a second command/configuration reference.

Full KB maintenance protocol: [`docs/kb/README.md`](docs/kb/README.md).

## Knowledge map

| Need | Authoritative source |
|---|---|
| Build, test, lint, and common verification | this file |
| Module ownership and command routing | [`docs/kb/architecture/module-map.md`](docs/kb/architecture/module-map.md) |
| Cross-module data flows | [`docs/kb/architecture/data-flows.md`](docs/kb/architecture/data-flows.md) |
| Safety and behavioral invariants | [`docs/kb/architecture/invariants.md`](docs/kb/architecture/invariants.md) |
| Shell, app, and sys preset authoring | [`docs/kb/preset-authoring.md`](docs/kb/preset-authoring.md) |
| Design rationale | [`docs/kb/decisions/`](docs/kb/decisions/) |
| Commit, versioning, and testing conventions | [`docs/kb/conventions.md`](docs/kb/conventions.md) |
| Release, CI, and troubleshooting | [`docs/kb/operations/`](docs/kb/operations/) |
| Past bugs and derived rules | [`docs/kb/lessons.md`](docs/kb/lessons.md) |
| Public user manual | [`docs/manual/`](docs/manual/), [`website/i18n/zh-Hans/`](website/i18n/zh-Hans/) |

## Hard repository rules

- **Never run `git push` without explicit user approval.** This includes branch pushes, tag pushes,
  and force-pushes. A request to commit is not permission to push.
- `CHANGELOG.md` is hand-written. Never generate it with `git cliff`; the `git cliff` invocation in
  `release.yml` generates only the GitHub Release body.
- Version decisions use the latest stable `v*` tag, never the moving `preview` tag:
  `git tag --list 'v*' --sort=-version:refname | head -1`.
- Work lands on `release`; `main` receives only automated post-release sync PRs. See
  [ADR 0001](docs/kb/decisions/0001-release-branch-model.md).
- Internal-only fixes caused by code in the same release use the git-cliff-skipped scopes documented
  in [`conventions.md`](docs/kb/conventions.md). Do not hide real user-facing fixes in those scopes.
- Uninstall and upgrade safety, external-code permissions, secret handling, and user-file ownership
  are governed by [`invariants.md`](docs/kb/architecture/invariants.md); read the relevant section
  before touching those paths.

## Commands

Versions are pinned in `mise.toml`. Run `mise install` once and activate mise before using the
repository toolchain. Install Bun dependencies before editing TypeScript presets.

```bash
# Setup
mise install
bun install --frozen-lockfile

# Build and run
cargo build
cargo build --release
cargo run -- shell list
cargo run -- app list
cargo run -- sys list
cargo run -- sys bootstrap --dry-run
cargo run -- env list
cargo run -- self upgrade --channel preview

# Rust tests
cargo nextest run --all-features
cargo test
cargo test shells::tests::install_then_uninstall_roundtrip
cargo nextest run -E 'test(install_then_uninstall)'

# Bun preset checks
bun run test:ts
bun run typecheck
bun run check:ts

# Lint and policy
cargo fmt
cargo clippy --all-targets --all-features --tests --benches -- -D warnings
cargo deny check bans licenses sources
typos

# Public documentation
cd website
pnpm install --frozen-lockfile
pnpm check:locales
pnpm typecheck
pnpm build
```

Pre-commit validates `mise.toml` and runs `cargo fmt --check`, clippy with warnings denied,
`cargo deny`, `typos`, and `cargo nextest run`. Changes to Bun tooling or TypeScript sources also
run `mise exec -- bun run check:ts`. All applicable checks must pass before committing.

## Verification boundaries

- In sandboxed environments, use `cargo ... --target-dir target` so build artifacts remain in the
  repository-local ignored directory.
- Most commands call `Config::load_or_init()` and can create state even when they appear read-only.
  Isolate ad-hoc checks:

  ```bash
  mkdir -p .tmp-home/.shine
  env SHINE_CONFIG_DIR=$PWD/.tmp-home/.shine cargo run --target-dir target -- app list
  ```

- `SHINE_CONFIG_DIR` overrides both the shine directory and presets directory; its runtime presets
  live at `$SHINE_CONFIG_DIR/presets/`. `SHINE_PRESETS` overrides only the presets directory.
- Built-in `app list`/`app info` reads embedded presets only when external preset mode is inactive.
  If `SHINE_CONFIG_DIR` is set, copy the preset under test to
  `.tmp-home/.shine/presets/app/<category>/` before list/info/install dry-runs, or unset it when
  verifying embedded metadata.
- For metadata-driven app presets, verify destination/metadata logic with a targeted unit test plus:

  ```bash
  cargo run --target-dir target -- app list
  cargo run --target-dir target -- app info <category>
  cargo run --target-dir target -- app install <category> --dry-run
  ```

More diagnostic cases: [`operations/troubleshooting.md`](docs/kb/operations/troubleshooting.md).

## Architecture at a glance

| Area | Primary location |
|---|---|
| CLI definition and top-level dispatch | `cli/src/commands/`, `cli/src/main.rs` |
| App install, upgrade, hooks, generators, artifacts | `cli/src/apps/` |
| Shared app/sys install primitives and manifest | `cli/src/install_core/` |
| Shell deployment and launcher activation | `cli/src/shells/`, `cli/src/bin_links.rs` |
| System bootstrap and managed resources | `cli/src/sys/` |
| Config discovery, layering, and save | `cli/src/config/` |
| Env, secrets, workspaces, and proxy injection | `cli/src/env/`, `cli/src/secret/` |
| SSH wrapper, transfer, and secret broker | `cli/src/ssh/` |
| Preset source/overlay operations | `cli/src/presets.rs`, `cli/src/preset_commands.rs`, `cli/src/git_pull.rs` |
| Update checks and self-install | `cli/src/update_check/`, `cli/src/self_install.rs` |
| Personal task registry | `cli/src/task/` |
| Embedded assets | `presets/` |
| Shared library | `utils/` |

Use the detailed [module map](docs/kb/architecture/module-map.md) for per-file ownership and command
routing. Use [data flows](docs/kb/architecture/data-flows.md) before changing behavior that spans
multiple modules.

## Finishing a change

1. Run checks proportional to the files and risk involved; follow any area-specific verification
   documented in the KB.
2. For public documentation changes, run locale parity, type checking, and the production docs build.
3. Run `git diff --check` and inspect `git status --short`.
4. Report checks actually run, distinguish pre-existing failures, and call out any unverified
   platform or external-system behavior.
