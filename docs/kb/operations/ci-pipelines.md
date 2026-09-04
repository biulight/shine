# CI Pipelines

All workflows live in `.github/workflows/`.

| Workflow | Trigger | What it does |
|---|---|---|
| `test.yml` | `workflow_call` only (reused by the others) | Ubuntu runs Bun checks, fmt, `cargo check --all`, clippy `-D warnings`, and `cargo audit`; a blocking Ubuntu x86_64 / macOS arm64 / Windows x86_64 matrix runs `cargo nextest run --all-features` natively |
| `msrv.yml` | reusable by releases; push/PR to `release` or `main`, manual | `cargo check --locked --workspace` with Rust 1.88; release asset building and crate publishing wait for it |
| `ci.yml` | push to `release`; PRs to `release`/`main` | calls `test.yml` |
| `release.yml` | push of a `v*` tag | test + MSRV → `package-assets.yml` builds per-platform tarballs and crates.io publish → GitHub Release with git-cliff-generated notes + `install.sh`/`install.ps1` → `open-main-pr` job opens (or reuses) the `release` → `main` sync PR; RC notes are tag-to-tag increments, while stable notes cover the previous stable release through the new stable tag |
| `preview.yml` | daily cron (00:00 UTC) + manual dispatch | if there are new commits since the `preview` tag: test → build assets → force-move `preview` tag → delete and re-publish the `Preview` prerelease |
| `package-assets.yml` | `workflow_call` | builds the release tarballs consumed by `release.yml`/`preview.yml` |
| `docs.yml` | documentation changes pushed to `release`, matching PRs, or manual dispatch | type-checks, checks locale parity, and builds the documentation; non-PR runs deploy to GitHub Pages and, when enabled, the configured documentation server |
| `setup-labels.yml` | (repo maintenance) | syncs GitHub issue labels |

Notes:

- The `open-main-pr` job is `continue-on-error`; if repo settings forbid Actions from creating
  PRs, it logs a warning with a manual compare URL instead of failing the release.
- Stable release-note range selection ignores prerelease tags as section boundaries, consolidates
  their commits into the stable section, and fails when no earlier stable `v*` tag exists.
  `chore(release)` preparation commits are excluded from generated notes.
- The `Preview` release is overwritten daily and marked prerelease with `make_latest: legacy`,
  so it never becomes the "latest" release that `shine update` resolves.
- The Ubuntu quality job in `test.yml` installs `cargo-llvm-cov`/llvm-tools; coverage tooling is
  available there. Rust tests run separately on all three supported OS families with `fail-fast`
  disabled so one platform failure does not hide the others.

## Documentation server deployment

`docs.yml` always keeps GitHub Pages deployment enabled. A second deployment uploads the same
`website/build` artifact to a server over SSH and rsync when the repository variable
`SERVER_DEPLOY_ENABLED` is exactly `true`. Configure the `docs-server` GitHub environment with:

| Kind | Name | Purpose |
|---|---|---|
| Variable | `SERVER_HOST` | SSH hostname or IP address |
| Variable | `SERVER_USER` | SSH login user |
| Variable | `SERVER_PORT` | SSH port; defaults to `22` |
| Variable | `SERVER_PATH` | Absolute destination directory; `/` is rejected |
| Variable | `SERVER_URL` | Optional deployment URL shown by GitHub |
| Secret | `SERVER_SSH_KEY` | Private SSH key used only for this deployment |
| Secret | `SERVER_KNOWN_HOSTS` | Pre-verified `known_hosts` line for the server |

Set the repository-level `SERVER_DEPLOY_ENABLED=true` only after the environment configuration is
complete. The deploy uses `rsync --delete-delay`, so files removed from the documentation build are
also removed from `SERVER_PATH`; the SSH account should be restricted to that destination.
