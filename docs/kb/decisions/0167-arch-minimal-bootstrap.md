# 0167 — Arch minimal bootstrap with native pacman

- **Status**: accepted
- **Date**: 2026-11-14

## Decision

Add a separate `sys/arch` v2 preset and a fixed pacman provider for the five-item `minimal`
selection. Keep exact distribution IDs: derivatives do not silently inherit Arch support.
The initial support scope is official x86_64 Arch Linux; Linux binary support is not a claim
of preset or managed-resource support on every distribution.

Use `pacman -S --needed --noconfirm PACKAGE` only after snapshot-bound approval, preserving
root/sudo, proxy, bounded output, timeout, post-detection, receipts and fail-stop behavior.
Package names retain the existing single-identifier validation. Never use `-Sy`, implicitly
upgrade the system with `-Syu`, delete database locks, or weaken signature checking. Arch users
must complete full-system maintenance before installing missing packages; avoiding refresh alone
does not prove that an already partially upgraded system is safe.

Share the existing Bash/Zsh startup-file workflow with Ubuntu. Arch owns separate sentinels and
`arch-sys.pre/post.sh` files. Keep existing Ubuntu loader bytes compatible. Planning, bootstrap,
profile enable/disable, shell switching and self-uninstall must agree on both startup files.
No new approval or receipt format is needed. Do not add AUR, package batching, distribution-family
fallback, or managed split DNS as part of this scope.

## Diagnostics

Missing typed pacman installations carry an advisory diagnostic in the observation-only Plan.
CLI review and dry-run explain the existing-database prerequisite without probing freshness.
Only the selected native provider items receive this guidance; custom scripts are not assumed
to follow the native provider policy. The notice must scope its no-refresh/no-upgrade promise to
native pacman installation, even in mixed native/script selections.

A pacman process with a nonzero exit code retains its status and bounded raw logs and adds conditional HTTP-404
recovery guidance. Error wording is not a classifier or authorization source: mirror failures,
locked databases and signature errors remain pacman failures. A successful fallback mirror must
not turn success into failure or trigger a repair. Signal termination, typed timeout and spawn
errors retain their existing diagnostics without download advice. No refresh, upgrade, retry, lock removal or
signature-policy change is introduced.

## Validation boundary

Regression tests cover the embedded preset, exact provider argv and permissions, both startup
files' approval fingerprints, installation/post-detection/rerun, shell execution, profile toggling,
switching and conservative loader detach. Native pacman/mirror/signature behavior needs separate
Arch acceptance; harmless package-manager fixtures do not establish that external behavior.
