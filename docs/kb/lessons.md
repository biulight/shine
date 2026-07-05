# Lessons Learned

Dated entries mined from real bugs. Format: **symptom → root cause → fix → rule**.
Newest first. Cite the fixing commit. Add an entry whenever a bug's cause was non-obvious.

## 2026-07-05 — Typed config readers must not silently discard invalid entries

- **Symptom**: `{ value, description }` entries in `shine.env.toml` appeared valid but had no
  effect, while the same shape worked in `config.toml` `[env]`.
- **Root cause**: the override reader parsed a generic TOML table and used `filter_map` to keep
  strings, silently dropping every other value instead of applying the shared env value model.
- **Fix**: parse each override entry as the same string-or-detailed env type, merge optional
  descriptions by layer, and report invalid entries with their key and file path.
- **Rule**: typed configuration readers must reject unsupported value shapes explicitly; never
  use filtering as validation when a dropped entry changes user-visible behavior.

## 2026-07-05 — Filtered overlay lookup must search the merged preset namespace

- **Symptom**: `shine upgrade` failed with `app preset category not found: JetBrains` when the
  category existed in the external presets directory but not in the configured overlay.
- **Root cause**: filtered app-category discovery treated a miss in either the base or overlay
  directory as fatal instead of checking whether the category existed in their union.
- **Fix**: defer the not-found error until base and overlay category names have been merged.
- **Rule**: filtered preset lookup must resolve against the merged namespace; a category only
  needs to exist in one source, while matching overlay paths still take precedence.

## 2026-07-04 — `requires_admin` dropped from manifest entries broke sudo uninstall

- **Symptom**: CI failure in `install_then_uninstall_roundtrip`; uninstall of
  `/etc/docker/daemon.json` went through the unprivileged path and failed.
- **Root cause**: Copy-strategy manifest entries didn't persist `requires_admin`, so
  `uninstall_app_entry` couldn't route to the admin-aware removal path.
- **Fix**: `70ee910` — persist `requires_admin` on `AppEntry`.
- **Rule**: manifest fields are load-bearing across install → uninstall; every flag that affects
  removal must survive the TOML round-trip, with a roundtrip test.

## 2026-07-04 — In-process locks don't serialize nextest tests on real system paths

- **Symptom**: intermittent races between tests touching `/etc/docker/daemon.json` once
  uninstall actually removed it via sudo.
- **Root cause**: nextest runs each test as its own OS process; the in-process `env_lock()`
  mutex can't serialize two processes on one real file.
- **Fix**: `fbd9c55` — cross-process advisory lock (`$TMPDIR/shine-admin.lock`, `create_dir` as
  mutex, stale-lock reclaim) around privileged fs mutations, plus a second lock held for the
  full body of whole-category install/uninstall tests.
- **Rule**: anything shared across test *processes* (real paths, global system state) needs a
  cross-process lock, not a `Mutex`.

## 2026-07-04 — Project config must inherit global settings

- **Symptom**: project-local configs silently lost global settings.
- **Root cause**: project config was read standalone instead of layering over the global one.
- **Fix**: `a5aed62` (inheritance) + `0936f05` (scheduled cleanup of the legacy project file).
- **Rule**: project config is an overlay over global config, not a replacement; removing legacy
  state should be scheduled/graceful, not abrupt.

## 2026-06-21 — PowerShell profile BOM must be preserved

- **Symptom**: rewriting a PowerShell profile corrupted/moved the UTF-8 BOM.
- **Fix**: `81244f8` — detect and preserve a leading BOM when editing profile files.
- **Rule**: on Windows, treat the BOM as part of the file-start invariant when splicing content.

## 2026-06-18 — External presets mode must fall back to embedded templates

- **Symptom**: sys profile installation failed when the external presets dir lacked a template.
- **Fix**: `5606438` — fall back to the embedded copy.
- **Rule**: external/overlay presets extend embedded assets; a missing external file degrades to
  embedded content, never to an error.

## 2026-06-16 — Version checks must be non-fatal and rate-limit aware

- **Symptom**: GitHub API failures/rate limits broke or spammed unrelated commands.
- **Fix**: `605fdd8` (tolerate check failures) + `f033a25` (cache rate-limit cooldown per auth
  mode alongside the 24 h version cache).
- **Rule**: background nicety features must never fail the user's primary command; cache
  negative results (cooldowns), not just positive ones.

## 2026-06-13 — zsh completions need explicit compinit handling

- **Symptom**: installed zsh completions didn't activate reliably.
- **Fix**: `fc410ab` (initialize the zsh completion system) + `f7eac5a` (harden compinit
  registration).
- **Rule**: don't assume the user's `.zshrc` runs `compinit`; shine's completion install must
  ensure registration itself, idempotently.

## 2026-06-13 — Global test state races under parallel test runs

- **Symptom**: two intermittent test failures from an `OVERLAY_DIR` race and scattered env-var
  mutexes.
- **Fix**: `3f7ac41` — unified shared `crate::test_support::env_lock()` across all modules.
- **Rule**: one shared lock for one shared resource; per-module locks over the same global
  resource are a race waiting to happen.

## Release practice — count from the latest stable tag

When cutting a release, always diff against the latest stable `v*` tag
(`git tag --list 'v*' --sort=-version:refname | head -1`), never `preview` and never
`git describe --tags --abbrev=0` alone (it can resolve to `preview`).
