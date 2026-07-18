# 0009 — App artifact build scripts run only via an explicit `shine app build` command

- **Status**: accepted (the "not auto-reversed" consequence is superseded by
  [ADR 0012](0012-app-lifecycle-post-install-and-teardown.md), which adds an optional
  `[artifact].teardown` script; the build-is-explicit-only stance still holds)
- **Evidence**: `cli/src/apps/build.rs`, `cli/src/apps/metadata.rs` (`[artifact]`/`AppArtifact`),
  `shine app build <app-id>`, `presets/app/surge/build.sh`

## Context

Some app presets need an explicit, provider-specific action that Shine core must not implement
itself. For Surge, `shine app install surge` copies user-maintained `local-proxies.conf` /
`local-rules.conf` next to the active profile (a plain `Copy` install to a `dest`), but the *active
profile itself* — a user-owned `.conf` under `~/Library/Application Support/Surge/Profiles/`, whose
`[Proxy]`/`[Rule]` sections `#!include` the Surge-managed subscription — must be patched so those
`#!include` lines also pull in the local files (`#!include <provider>, local-proxies.conf`).
Knowing what a Surge `#!include` is, which section maps to which local file, and where the profile
lives is provider-specific logic that must stay out of Shine core — the same restraint behind
[ADR 0007](0007-explicit-safe-git-pull.md). `shine upgrade` must stay a predictable install pass
with no implicit external edits.

(The subscription is left entirely to Surge's own `#!MANAGED-CONFIG` refresh; Shine never fetches
it. An earlier iteration had `build.sh` fetch/split/serve the subscription over `shine serve` — that
was dropped as it duplicated Surge's job and fought the subscription's expiry.)

## Decision

Add a generic `[artifact].script` metadata field and a `shine app build <app-id>` command that:

- Resolves the category exactly like `app info`/`app install` (`metadata::load_active_categories`).
- Resolves the script's location with the overlay directory winning over the built-in/external
  source directory *as a whole* when the overlay's copy of the category exists — one decision per
  category, not per file, since a build script's sibling files are conceptually one package with
  the script.
- Injects a fixed environment-variable contract (`SHINE_APP_ID`, `SHINE_APP_DIR`,
  `SHINE_APP_SOURCE_DIR`, `SHINE_APP_OVERLAY_DIR` when an overlay copy exists, `SHINE_APP_HTTP_DIR`,
  `SHINE_CONFIG_DIR`, `SHINE_CACHE_DIR`, `SHINE_STATE_DIR`) **plus the active `[env]` table passed
  as stored** (via `EnvConfig::as_map` — plaintext keys as-is, `_SECRET` keys as ciphertext; **no
  decryption**, exactly like the `template` transform), so a script can read user-configured values
  such as `SURGE_PROFILE` without any build ever triggering a secret-decryption prompt (Touch ID /
  GPG) for unrelated `_SECRET` keys. The `SHINE_APP_*` contract vars are applied after the `[env]`
  values so they win on any name collision. The script runs with inherited stdio, so build
  output/failures are visible live rather than captured.
- Propagates a nonzero script exit as a real `anyhow::Error` — unlike `post_upgrade` hooks
  (`apps/upgrade.rs::run_post_upgrade_hooks`), which are a background side effect of `shine upgrade`
  and deliberately swallow a failing hook's error so one broken hook can't abort the whole upgrade.
  `shine app build` is a single, explicit, user-invoked action, so its failure should be loud.
- Never runs implicitly from `shine upgrade` or `shine app install`.

Shine core deliberately does not know what the script does with those inputs — there are no
`cache`/`publish`/`run_on_upgrade` fields in the metadata model. Patching a profile, caching, or
anything else is entirely the script's own responsibility.

## Consequences

- `shine upgrade` stays side-effect-predictable: it never touches the active Surge profile and never
  fetches a subscription. It only re-copies the tracked `local-*.conf` files.
- Real provider-specific logic (which sections to patch, the profile's path from `SURGE_PROFILE`,
  the idempotent `#!include` append) lives entirely outside this repo, in the `shineOverlay`
  project's own `build.sh`; the built-in `presets/app/surge/build.sh` is a placeholder only.
- Adding an artifact script to a new app preset requires no Shine core changes — just an
  `[artifact]` table and a script that honors the environment contract.
- A category with no `[artifact]` section behaves exactly as before; `shine app build` on such a
  category is a clear, immediate error rather than a silent no-op.
- **The patch is not auto-reversed.** `shine app uninstall surge` removes the copied `local-*.conf`
  but does not un-patch the profile's `#!include` lines; the overlay `build.sh` is idempotent
  (add-only) and un-patching is a documented manual step. This is an accepted tradeoff for keeping
  the patch logic in the overlay rather than teaching Shine core a reversible profile-edit strategy.

## Update (2026-07-18): cross-platform `runtime = "bun"`

The original artifact runner execs the script directly (`Command::new(script)`), which relies on a
shebang and is therefore **Unix-only**. That is fine for surge (macOS-only), but not for a
cross-platform preset like `clash-verge` (Clash Verge Rev runs on Windows/macOS/Linux).

`[artifact]` now accepts an optional `runtime` field (`native` default, or `bun`). `runtime = "bun"`
launches the script via `bun <script>` — the same cross-platform runtime the bun **shell** presets
use — so a `.ts` artifact works on all three platforms. `bun` is an external prerequisite; a missing
`bun` fails with a clear "not installed" error (via `proc::ensure_command`) instead of a raw spawn
error. A `bun` artifact's `script`/`teardown` must be a `.ts`/`.js`/`.mts`/`.mjs` file (validated at
metadata-load time). The `native` path is unchanged.

`clash-verge` uses `runtime = "bun"` with `build.ts`/`unbuild.ts`. Its refresh logic (a mihomo
external-controller `PUT /providers/rules/<name>`) is generic and secret-free, so — unlike surge's
provider-specific patch — it can ship in shine core; the user-specific pieces (real `merge.yaml`
values, `CLASH_CONTROLLER_URL/TOKEN`) still live in the overlay, preserving this ADR's principle.
