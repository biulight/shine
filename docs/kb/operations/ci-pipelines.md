# CI Pipelines

All workflows live in `.github/workflows/`.

| Workflow | Trigger | What it does |
|---|---|---|
| `test.yml` | `workflow_call` only (reused by the others) | fmt check → `cargo check --all` → clippy `-D warnings` → `cargo audit` → `cargo nextest run --all-features` |
| `ci.yml` | push to `release`; PRs to `release`/`main` | calls `test.yml` |
| `release.yml` | push of a `v*` tag | test → `package-assets.yml` builds per-platform tarballs → GitHub Release with git-cliff-generated notes + `install.sh`/`install.ps1` → `open-main-pr` job opens (or reuses) the `release` → `main` sync PR |
| `preview.yml` | daily cron (00:00 UTC) + manual dispatch | if there are new commits since the `preview` tag: test → build assets → force-move `preview` tag → delete and re-publish the `Preview` prerelease |
| `package-assets.yml` | `workflow_call` | builds the release tarballs consumed by `release.yml`/`preview.yml` |
| `setup-labels.yml` | (repo maintenance) | syncs GitHub issue labels |

Notes:

- The `open-main-pr` job is `continue-on-error`; if repo settings forbid Actions from creating
  PRs, it logs a warning with a manual compare URL instead of failing the release.
- The `Preview` release is overwritten daily and marked prerelease with `make_latest: legacy`,
  so it never becomes the "latest" release that `shine update` resolves.
- `test.yml` installs `cargo-llvm-cov`/llvm-tools; coverage tooling is available in CI runs.
