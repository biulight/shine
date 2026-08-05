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
2. **Decide the bump.** User-facing features since the tag → minor; only user-facing fixes →
   patch. Ignore `fix(lint|clippy|fmt|typo|build|ci|internal)` commits.
3. **Bump versions.** Update the workspace version in root `Cargo.toml`, which keeps
   `shine-cli` and `shine-core` in sync; refresh `Cargo.lock` (`cargo check`).
4. **Write CHANGELOG.md by hand.** New `## [x.y.z] — YYYY-MM-DD` section, entries grouped under
   Features / Bug Fixes / Internal / Docs, plain English, user-facing. Do **not** use git-cliff
   for this file.
5. **Commit** as `chore(release): prepare vX.Y.Z` (pre-commit gates must pass).
6. **Get explicit user approval before pushing anything** (Git Push Policy). Then push the
   branch, tag `vX.Y.Z`, and push the tag.
7. **`release.yml` takes over**: tests → asset build → GitHub Release (git-cliff notes) →
   automatic `release` → `main` sync PR.

## Post-release checks

- GitHub Release exists with tarballs for all platforms plus `install.sh`/`install.ps1`.
- The `release` → `main` sync PR was opened (the job is `continue-on-error`; if Actions cannot
  create PRs, open it manually — the workflow log prints the compare URL).
- `shine update` on an installed binary detects the new version (cache TTL is 24 h; use
  the force-refresh path when verifying).
