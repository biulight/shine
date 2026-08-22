# Troubleshooting

Common failures and where to look. Add new entries as they occur; pair persistent root causes
with a [`lessons.md`](../lessons.md) entry.

## CI failures

| Symptom | Likely cause / fix |
|---|---|
| clippy step fails, code builds locally | CI uses `-D warnings` across `--all-targets --all-features --tests --benches`. Run the exact command from `AGENTS.md` § Commands locally. |
| `cargo audit` fails on an untouched PR | A new RUSTSEC advisory landed for an existing dependency. Upgrade the affected transitive dep (precedent: `e146b08` quinn-proto for RUSTSEC-2026-0185) as `fix(deps)`. |
| Intermittent test failures under nextest only | Cross-process race: nextest runs each test in its own OS process, so in-process mutexes don't serialize them. Real-path (sudo) tests must hold the cross-process admin lock; env-var tests must hold `test_support::env_lock()`. See commits `fbd9c55`, `3f7ac41`. |
| `install_then_uninstall_roundtrip` fails on privileged paths | Check that `requires_admin` survives the manifest round-trip and that uninstall routes through the sudo path (`70ee910`). |

## Local development

| Symptom | Likely cause / fix |
|---|---|
| Edited a preset but the binary still shows old content | Re-embed didn't trigger. Confirm `cli/build.rs` still has `cargo:rerun-if-changed=presets`; touch a file under `presets/` and rebuild. |
| `app list` / `app info` shows unexpected presets | An external presets mode is active. Check `SHINE_CONFIG_DIR`, `SHINE_PRESETS`, and `presets_dir`/`presets_overlay_dir` in `config.toml` (priority chain: ADR 0005). |
| Commands create state in `~/.shine` during ad-hoc verification | Most commands call `Config::load_or_init()`. Isolate with `SHINE_CONFIG_DIR=$PWD/.tmp-home/.shine` (see `AGENTS.md` § Verification boundaries). |
| Sandbox build permission failures | Use `cargo ... --target-dir target` to keep artifacts repo-local. |

## Update / self-upgrade

| Symptom | Likely cause / fix |
|---|---|
| `shine update` says up-to-date right after a release | 24 h version cache (`UPDATE_CACHE_TTL`). Run `shine update --refresh-release` to bypass it. |
| Version check errors or GitHub rate limits | Non-fatal by design (`605fdd8`); rate-limit cooldowns are cached per auth mode (`f033a25`). The user's primary command must still succeed. |
| Upgrade comparing against the wrong version | The moving `preview` tag was used as baseline. Always compare with the latest stable `v*` tag. |
