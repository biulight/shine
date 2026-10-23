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

## Code-analysis bindings and CLI diff acceptance (2026-10-23)

Accepted `release` commit:
[`c781b1ecf369ba240f5dbde056c2650cf90214cf`](https://github.com/biulight/shine/commit/c781b1ecf369ba240f5dbde056c2650cf90214cf).
The push also included parent commit `3b2a8346` (complete, safely escaped CLI diff output and
public manual updates). The accepted head fixes generator-function shadowing and builtin member
mutation analysis, preserves captured functions and unrelated members, and resolves immutable
aliases with bounded retries. Parser and Core Plan regressions cover both false positives and
missed declaration differences.

The [CI run](https://github.com/biulight/shine/actions/runs/39922126760),
[MSRV run](https://github.com/biulight/shine/actions/runs/39922126713), and
[documentation run](https://github.com/biulight/shine/actions/runs/39922126667) all target that exact
head SHA and finished with `completed / success`.

| Check | Result | Evidence |
| --- | --- | --- |
| Windows `x86_64-pc-windows-msvc` | All-feature workspace nextest: 1,753 passed, 0 skipped | [Windows job](https://github.com/biulight/shine/actions/runs/39922126760/job/120309616598) |
| Linux `x86_64-unknown-linux-gnu` | All-feature workspace nextest: 2,016 passed, 2 skipped | [Linux job](https://github.com/biulight/shine/actions/runs/39922126760/job/120309616693) |
| macOS `aarch64-apple-darwin` | All-feature workspace nextest: 2,018 passed, 2 skipped | [macOS job](https://github.com/biulight/shine/actions/runs/39922126760/job/120309616855) |
| Quality | Bun checks, formatting, compilation, Preset authoring examples, strict Clippy and security audit passed | [Quality job](https://github.com/biulight/shine/actions/runs/39922126760/job/120309616345) |
| MSRV | Rust 1.90 `cargo check --locked --workspace` passed | [MSRV job](https://github.com/biulight/shine/actions/runs/39922126713/job/120309615878) |
| Documentation build | Typecheck, version checks, locale parity and all-locale build passed | [Build job](https://github.com/biulight/shine/actions/runs/39922126667/job/120309616317) |
| GitHub Pages deployment | Succeeded | [Pages job](https://github.com/biulight/shine/actions/runs/39922126667/job/120309981777) |
| Documentation server deployment | Succeeded | [Server job](https://github.com/biulight/shine/actions/runs/39922126667/job/120309981834) |

These are native CI results for the accepted head, separate from the local 48 analysis-related
tests, built-in Preset sample test and six isolated CLI probes. The public manual changes in the
parent commit triggered `docs.yml`; this internal KB evidence update alone does not match that
workflow's path filters and does not deploy documentation.

## Code-analysis P1 acceptance (2026-10-23)

Accepted `release` commit:
[`aa9b81f8445621efb755d23ed11f2666a390c874`](https://github.com/biulight/shine/commit/aa9b81f8445621efb755d23ed11f2666a390c874).
The push contained `14eac507` (four P1 fixes, syntax-boundary review repairs and regressions under
analysis rule version 6) and `aa9b81f8` (isolated native verification evidence). The fixes cover
operand comments, lexical mutation identity, default-parameter scope and loader identity without
changing trust, approval or non-blocking coverage-gap semantics. P2 diagnostic aggregation was
deferred at this checkpoint.

The [CI run](https://github.com/biulight/shine/actions/runs/39962137705) and
[MSRV run](https://github.com/biulight/shine/actions/runs/39962137736) both target that exact SHA and
finished with `completed / success` on attempt 1. No CI repair or rerun was required.

| Check | Result | Evidence |
| --- | --- | --- |
| Windows `x86_64-pc-windows-msvc` | All-feature workspace nextest: 1,766 passed, 0 skipped | [Windows job](https://github.com/biulight/shine/actions/runs/39962137705/job/120442516242) |
| Linux `x86_64-unknown-linux-gnu` | All-feature workspace nextest: 2,029 passed, 2 skipped | [Linux job](https://github.com/biulight/shine/actions/runs/39962137705/job/120442516101) |
| macOS `aarch64-apple-darwin` | All-feature workspace nextest: 2,031 passed, 2 skipped | [macOS job](https://github.com/biulight/shine/actions/runs/39962137705/job/120442516018) |
| Quality | Bun typecheck and 78 tests, formatting, compilation, Preset authoring examples, strict Clippy and security audit passed | [Quality job](https://github.com/biulight/shine/actions/runs/39962137705/job/120442515745) |
| MSRV | Rust 1.90.0 `cargo check --locked --workspace` passed | [MSRV job](https://github.com/biulight/shine/actions/runs/39962137736/job/120442515433) |

All three native test jobs reported zero LEAK results under the existing strict policy. Windows
retained the previously observed unused-item warnings in unchanged CLI code; these did not fail
the test job. Quality Clippy ran on Ubuntu with warnings denied.

These full workspace CI results supplement the earlier
[targeted Windows/Linux verification](../architecture/code-analysis.md#p1-native-verification-2026-10-23)
at `14eac507`; they apply to the accepted head, not to subsequent documentation commits. The push
changed no public manual, website or documentation-workflow paths, so `docs.yml` was not triggered
and no documentation deployment is claimed.

## Code-analysis P2 acceptance (2026-10-23)

Accepted `release` commit:
[`5da9fa466623d8b53661832f08d2d914169227a6`](https://github.com/biulight/shine/commit/5da9fa466623d8b53661832f08d2d914169227a6).
The push included `771f3028` (P1 CI evidence) and `5da9fa46` (P2 diagnostic aggregation, detail-view
review fixes, tests, bilingual manual and KB updates). Ordinary text summarizes repeated advisory
analysis diagnostics; errors remain individual, and existing detailed inspection modes retain all
locations. Core reports, JSON, admission, trust and approval contracts are unchanged.

The [CI run](https://github.com/biulight/shine/actions/runs/39976619157),
[MSRV run](https://github.com/biulight/shine/actions/runs/39976619175), and
[documentation run](https://github.com/biulight/shine/actions/runs/39976619213) all target that exact
SHA and finished with `completed / success` on attempt 1. No CI repair commit or workflow rerun
was required.

| Check | Result | Evidence |
| --- | --- | --- |
| Windows `x86_64-pc-windows-msvc` | All-feature workspace nextest: 1,769 passed, 0 skipped | [Windows job](https://github.com/biulight/shine/actions/runs/39976619157/job/120492772336) |
| Linux `x86_64-unknown-linux-gnu` | All-feature workspace nextest: 2,032 passed, 2 skipped | [Linux job](https://github.com/biulight/shine/actions/runs/39976619157/job/120492772311) |
| macOS `aarch64-apple-darwin` | All-feature workspace nextest: 2,034 passed, 2 skipped | [macOS job](https://github.com/biulight/shine/actions/runs/39976619157/job/120492772513) |
| Quality | Bun typecheck and 78 tests, formatting, compilation, Preset authoring examples, strict Clippy and security audit passed | [Quality job](https://github.com/biulight/shine/actions/runs/39976619157/job/120492772092) |
| MSRV | Rust 1.90.0 `cargo check --locked --workspace` passed | [MSRV job](https://github.com/biulight/shine/actions/runs/39976619175/job/120492771688) |
| Documentation build | Typecheck, version checks, locale parity and all-locale build passed | [Build job](https://github.com/biulight/shine/actions/runs/39976619213/job/120492772674) |
| GitHub Pages deployment | Succeeded | [Pages job](https://github.com/biulight/shine/actions/runs/39976619213/job/120493321128) |
| Documentation server deployment | Succeeded | [Server job](https://github.com/biulight/shine/actions/runs/39976619213/job/120493320389) |

All three native test jobs reported zero LEAK results under the existing strict policy. Windows
retained the known unused-item warnings in unchanged CLI code; Ubuntu quality Clippy passed with
warnings denied. These full workspace CI results supplement the
[local and targeted native P2 evidence](../architecture/code-analysis.md#diagnostic-presentation)
and apply to the accepted head, not subsequent documentation commits. The bilingual manual changes
triggered the documentation workflow and both deployments; this internal KB update alone does not
match that workflow's path filters.

## 2.7.0 local release preparation (2026-10-24)

Prepared on top of `e28f316c`: synchronized crate/dependency/lockfile versions to 2.7.0,
recorded the [compatibility exception](../conventions.md#versioning), completed the hand-written
release notes and bilingual upgrade guidance, and replaced the 2.6 documentation snapshots with
2.7 while retaining 1.8 and Next. Third-party lockfile entries are unchanged.

The only Rust changes gate Unix-only administrator/test helpers and imports by their actual
platform consumers. No runtime logic, analysis rules, trust or approval behavior changed.

| Local/native check | Result |
| --- | --- |
| macOS, Rust 1.96.0 | Workspace all-target/all-feature check and strict Clippy passed; all-feature nextest: 2,034 passed, 2 skipped |
| Windows `x86_64-pc-windows-msvc`, Rust 1.96.0 | `RUSTFLAGS='-D warnings' cargo check --locked --workspace --all-targets --all-features --target-dir target` passed; the previously recorded unused-item compiler warnings are gone |
| MSRV on macOS | Rust 1.90.0 `cargo check --locked --workspace` passed with a separate target directory |
| Formatting and dependency policy | fmt, typos and cargo-deny bans/licenses/sources passed; existing duplicate-dependency and unmatched-license-allowance warnings remain |
| Documentation | Frozen dependency install, version checks (8 tests), locale parity (21 files per locale), typecheck and both locale builds passed |
| Snapshot and generated-page inspection | Both 2.7 snapshots exactly match their current sources; 1.8 is unchanged. Twelve stable/Next English/Chinese pages retain version labels, locale/canonical metadata, migration anchors and full-diagnostic guidance; retired 2.6 routes are absent |

Windows used an isolated working-copy archive with SHA-256
`cbd015a555dc162e416956be8fa1e5e10f13740d7ca396710e934bf32f624ef4`, including the final Rust and
Cargo changes; later edits only changed documentation. The remote checksum matched,
the command exited 0, and its task directory was removed after preserving the log locally.
This is native compilation evidence, not a new Windows runtime test run. Linux was not rerun.

Release-preparation review found that the compatibility notes omitted Rust source breaks from
added public report fields. A compile-only external consumer using the v2.6.0 `PlanV1` field list
failed with `E0063: missing field code_analysis`; adapting the field list compiled successfully.
The exception, ADR 0150, CHANGELOG and both installation guides/snapshots now disclose this
separately from serialized schema changes. An isolated old-env source passed migration preview,
explicit apply, validation and lint without executing its generator. The rebuilt site passed
all 3,748 local links/anchors across 72 stable/Next bilingual pages. Snapshot byte parity, version
checks, locale parity, typecheck and typos also passed after review. All modified Rust/Cargo files
still match the native-verified archive; no further Rust changes or full test reruns were needed.

Local Cargo checks bypassed sandbox-incompatible sccache with `RUSTC_WRAPPER=`; the full test run
had permission to bind its test sockets. Documentation builds retained non-failing notices about
untracked snapshot update dates, the local update-check store and Node localStorage.
These are uncommitted preparation results, not new CI or publication evidence. No tag, push,
crate publication or deployment was performed; a later release candidate must pass its own gates.

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
