# Release Runbook

How to cut a stable release. Prerequisite reading:
[ADR 0001](../decisions/0001-release-branch-model.md) (branch model),
[ADR 0002](../decisions/0002-hand-written-changelog.md) (changelog policy),
[`conventions.md`](../conventions.md) § Versioning.

## Steps

1. **Establish the baseline.** On `release`:
   ```bash
   git tag --list 'v*' --sort=-version:refname | head -1   # latest stable tag — NOT `preview`
   git log <latest-tag>..HEAD --oneline
   ```
2. **Decide the bump.** Incompatible public CLI, configuration, automation, or library contracts
   → major; backward-compatible user-facing features → minor; only backward-compatible fixes →
   patch. Ignore `fix(lint|clippy|fmt|typo|build|ci|internal)` commits.
3. **Bump versions.** Update the workspace version in root `Cargo.toml`, which keeps
   `shine-cli` and `shine-core` in sync; refresh `Cargo.lock` (`cargo check`).
4. **Write CHANGELOG.md by hand.** New `## [x.y.z] — YYYY-MM-DD` section, entries grouped under
   Features / Bug Fixes / Internal / Docs, plain English, user-facing. Do **not** use git-cliff
   for this file.
5. **Verify public documentation** when the release changes user-visible behavior: update both the
   English default manual and Simplified Chinese locale. Publish only the latest stable
   documentation within each major, plus `Next` ([ADR 0089](../decisions/0089-documentation-version-retention.md)).
   For a new major release, add its major.minor snapshot in both locales and retain the latest
   snapshot of each older major; move the previous stable version to its major.minor URL.
   For a new minor release, replace the previous same-major snapshot in `website/versions.json`
   and the Docusaurus version configuration; remove its English and Chinese snapshot directories,
   versioned sidebar, and Chinese version metadata. Patch releases refresh the existing
   major.minor snapshot instead of adding a patch-version entry. Keep the latest stable version
   first and at the root URL, older majors at their version paths, and `Next` at `/next`.
   Remove references to retired routes; they return 404 without redirects. Run
   `pnpm check:versions`, `pnpm check:locales`, `pnpm typecheck`, and `pnpm build` from `website/`.
6. **Commit** as `chore(release): prepare vX.Y.Z` (pre-commit gates must pass).
7. **Get explicit user approval before pushing anything** (Git Push Policy). Then push the
   branch, tag `vX.Y.Z`, and push the tag.
8. **`release.yml` takes over**: tests + MSRV → asset build and crate publication → GitHub Release
   (git-cliff notes) → cargo-binstall verification → automatic `release` → `main` sync PR.
   Stable notes cover the previous stable tag through the
   new stable tag as one consolidated release section, excluding intervening RC boundaries and
   `chore(release)` commits.

For a release candidate, use `X.Y.Z-rc.N` and tag `vX.Y.Z-rc.N`. The workflow publishes crates and
assets as a GitHub prerelease, leaves the latest stable release unchanged, and skips the
`release` → `main` sync PR. RC notes remain incremental from the immediately preceding versioned
tag. The moving `preview` release remains a separate daily channel.

## Post-release checks

- GitHub Release exists with tarballs for all platforms plus `install.sh`/`install.ps1`.
- All six `verify-binstall` matrix entries pass using the published crate metadata, with native
  version checks on Linux x86_64, macOS arm64, and Windows x86_64. These checks run for stable and RC
  tags and allow only the `crate-meta-data` strategy. A failure leaves publication intact but blocks
  the stable sync PR; inspect the download/version failure before rerunning verification.
- The `release` → `main` sync PR was opened (the job is `continue-on-error`; if Actions cannot
  create PRs, open it manually — the workflow log prints the compare URL).
- `shine update` on an installed binary detects the new version (cache TTL is 24 h; use
  the force-refresh path when verifying).

## Checking cargo-binstall metadata before release

The root crate publishes `[package.metadata.binstall]` with exact target overrides for the existing
archive names. Keep them aligned with `package-assets.yml`; do not map musl to GNU archives.
See [ADR 0122](../decisions/0122-cargo-binstall-release-verification.md).

While the manifest still names a version whose assets already exist, test local metadata without
installing a binary (use a fresh temporary root):

```bash
binstall_check_root=$(mktemp -d)
mise exec -- cargo binstall shine-cli --manifest-path "$PWD/Cargo.toml" \
  --target aarch64-apple-darwin --strategies crate-meta-data \
  --root "$binstall_check_root" --dry-run --no-confirm
```

Repeat with each of the six packaged target triples. A native smoke check can omit `--dry-run` and
run the binary from the temporary root with an isolated `SHINE_CONFIG_DIR`. This checks the local
manifest against existing assets, not new unreleased code or the metadata already on crates.io.
After a version bump, the new URLs cannot be checked live until those assets are published.
Published crate metadata is immutable; adding it in Git alone does not retrofit an older release.
Use an absolute manifest path: cargo-binstall 1.12.7 cannot resolve workspace inheritance from the
relative `Cargo.toml` path.
