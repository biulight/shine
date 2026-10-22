# CI Pipelines

All workflows live in `.github/workflows/`.

| Workflow | Trigger | What it does |
|---|---|---|
| `test.yml` | `workflow_call` only (reused by the others) | Ubuntu runs Bun checks, fmt, `cargo check --all`, clippy `-D warnings`, and `cargo audit`; a blocking Ubuntu x86_64 / macOS arm64 / Windows x86_64 matrix runs `cargo nextest run --workspace --all-features` natively |
| `msrv.yml` | reusable by releases; push/PR to `release` or `main`, manual | `cargo check --locked --workspace` with Rust 1.90; release asset building and crate publishing wait for it |
| `ci.yml` | push to `release`; PRs to `release`/`main` | calls `test.yml` |
| `release.yml` | push of a `v*` tag | test + MSRV → per-platform tarballs and crates.io publish → GitHub Release → six-target cargo-binstall verification → stable `release` → `main` sync PR; RC notes are tag-to-tag increments, while stable notes cover the previous stable release through the new stable tag |
| `preview.yml` | daily cron (00:00 UTC) + manual dispatch | if there are new commits since the `preview` tag: test → build assets → force-move `preview` tag → delete and re-publish the `Preview` prerelease |
| `package-assets.yml` | `workflow_call` | builds the release tarballs consumed by `release.yml`/`preview.yml` |
| `docs.yml` | documentation changes pushed to `release`, matching PRs, or manual dispatch | type-checks, checks documentation version retention and resource consistency (including regression tests), checks locale parity, and builds the documentation; non-PR runs deploy to GitHub Pages and, when enabled, the configured documentation server |
| `setup-labels.yml` | (repo maintenance) | syncs GitHub issue labels |

Notes:

- `verify-binstall` uses cargo-binstall 1.12.7 and the exact tagged version from crates.io, with
  `--strategies crate-meta-data` so third-party binaries and source compilation cannot hide a broken
  official download. All six targets download/extract in dry-run mode; Linux x86_64, macOS arm64,
  and Windows x86_64 also install under the runner's temporary directory and check `shine --version`.
  The other targets are download checks only. The job runs after publication, so a failure does not
  roll back published crates or assets; it fails the workflow and blocks the stable sync PR.
- The `open-main-pr` job is `continue-on-error`; if repo settings forbid Actions from creating
  PRs, it logs a warning with a manual compare URL instead of failing the release.
- Stable release-note range selection ignores prerelease tags as section boundaries, consolidates
  their commits into the stable section, and fails when no earlier stable `v*` tag exists.
  `chore(release)` preparation commits are excluded from generated notes.
- The `Preview` release is overwritten daily and marked prerelease with `make_latest: legacy`,
  so it never becomes the "latest" release that `shine update` resolves.
- The Ubuntu quality job in `test.yml` installs `cargo-llvm-cov`/llvm-tools; coverage tooling is
  available there. Rust tests run separately on all three supported OS families with `fail-fast`
  disabled so one platform failure does not hide the others. Each matrix job installs the pinned
  Rust and Bun versions and invokes nextest through `mise exec`, so subprocesses launched from
  temporary directories inherit concrete runtime paths instead of context-dependent mise shims.

## Rust 1.90 and Nextest acceptance (2026-10-22)

Accepted `release` commit:
[`4ece7d6bd9164c551a2e7daf0f9aa23644a94ffd`](https://github.com/biulight/shine/commit/4ece7d6bd9164c551a2e7daf0f9aa23644a94ffd).
The Rust API cleanup replaces `fs2` with standard-library file locks and manual Result flattening
with `Result::flatten()`. The Nextest policy requires 0.9.145 or newer and fails leaked output
handles at the unchanged 100 ms timeout; pre-commit also tests changes to that policy.

The [CI run](https://github.com/biulight/shine/actions/runs/39722135944) and
[MSRV run](https://github.com/biulight/shine/actions/runs/39722135836) both target that exact commit
and finished with `completed / success`. All three native test jobs used Nextest 0.9.148 and
reported zero LEAK results under the committed strict policy.

| Check | Result | Evidence |
| --- | --- | --- |
| Windows `x86_64-pc-windows-msvc` | All-feature workspace nextest: 1,735 passed, 0 skipped | [Windows job](https://github.com/biulight/shine/actions/runs/39722135944/job/119637785774) |
| Linux `x86_64-unknown-linux-gnu` | All-feature workspace nextest: 1,996 passed, 2 skipped | [Linux job](https://github.com/biulight/shine/actions/runs/39722135944/job/119637785642) |
| macOS `aarch64-apple-darwin` | All-feature workspace nextest: 1,998 passed, 2 skipped | [macOS job](https://github.com/biulight/shine/actions/runs/39722135944/job/119637785686) |
| MSRV | Rust 1.90.0 `cargo check --locked --workspace` passed | [MSRV job](https://github.com/biulight/shine/actions/runs/39722135836/job/119637785170) |
| Quality | Bun typecheck and 78 tests, formatting, compilation, Preset authoring examples, strict Clippy and security audit passed | [Quality job](https://github.com/biulight/shine/actions/runs/39722135944/job/119637785471) |

These are native CI results for the committed changes. The earlier local Nextest 0.9.149 run and
deliberately leaking `LEAK-FAIL` probe are separate local validation evidence.

## Let-else cleanup acceptance (2026-10-22)

Accepted `release` commit:
[`e3d1c165f0533044d92a0295dcdc60d8f96eeba3`](https://github.com/biulight/shine/commit/e3d1c165f0533044d92a0295dcdc60d8f96eeba3).
Compared with its parent `2e8e1712`, this replaces 12 simple early-return matches with `let ... else`
across seven Rust files: info diff rendering, self-install synchronization, SSH listener accepts,
Preset fixture loading and packing, permission validation, and sentinel removal. Production code
changed by +34/-55 lines (net -21); tests/support and documentation were unchanged in that commit.
Complex action-enum matches and the existing crate publication workflow remain unchanged.
The Rust 1.90 MSRV remains unchanged.

The [CI run](https://github.com/biulight/shine/actions/runs/39761350589) and
[MSRV run](https://github.com/biulight/shine/actions/runs/39761350622) both target that exact commit
and finished with `completed / success`. All three native Nextest jobs reported zero LEAK results
under the existing strict policy.

| Check | Result | Evidence |
| --- | --- | --- |
| Windows `x86_64-pc-windows-msvc` | All-feature workspace nextest: 1,735 passed, 0 skipped | [Windows job](https://github.com/biulight/shine/actions/runs/39761350589/job/119766024578) |
| Linux `x86_64-unknown-linux-gnu` | All-feature workspace nextest: 1,996 passed, 2 skipped | [Linux job](https://github.com/biulight/shine/actions/runs/39761350589/job/119766024504) |
| macOS `aarch64-apple-darwin` | All-feature workspace nextest: 1,998 passed, 2 skipped | [macOS job](https://github.com/biulight/shine/actions/runs/39761350589/job/119766024925) |
| MSRV | Rust 1.90.0 `cargo check --locked --workspace` passed | [MSRV job](https://github.com/biulight/shine/actions/runs/39761350622/job/119766024123) |
| Quality | Bun typecheck and 78 tests, formatting, compilation, Preset authoring examples, strict Clippy and security audit passed | [Quality job](https://github.com/biulight/shine/actions/runs/39761350589/job/119766024347) |

The code review preserved successful bindings, early-return values, error codes, output text, and
sentinel offset/CRLF handling. Local Rust 1.90 all-target/all-feature compilation and commit hooks
also passed. An initial sandboxed test run could not create local sockets; the permitted local
rerun reported 1,998 passed and 2 skipped. The table records native CI evidence separately from that
local rerun.

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
