# Shine

A cross-platform Rust CLI for managing shell commands, application configuration, system resources,
layered environments, and repeatable machine setup.

Shine packages useful presets in one self-contained binary. Managed files are tracked through
manifests so they can be inspected, updated, and safely removed without taking ownership of unrelated
user content. You can also maintain external preset repositories and selective overlays.

**Documentation:** [English](https://biulight.github.io/shine/) ·
[简体中文](https://biulight.github.io/shine/zh-Hans/)

## Highlights

- Install portable shell commands and application configuration with dry-run and modification guards.
- Bootstrap development environments on macOS, Ubuntu, and Windows.
- Manage layered environment values and encrypt workspace secrets with GPG or age.
- Save personal tasks, synchronize terminal themes, and serve managed local resources.
- Forward selected values, broker local secrets, and transfer files through `shine ssh` sessions.
- Extend built-in presets through an external source or path-level overlay.

## Installation

macOS and Linux:

```bash
curl -fsSL https://github.com/biulight/shine/releases/latest/download/install.sh | sh
```

Windows PowerShell:

```powershell
irm https://github.com/biulight/shine/releases/latest/download/install.ps1 | iex
```

From crates.io with Rust 1.88 or later:

```bash
cargo install shine-cli
```

## Quick example

```bash
shine list --available
shine info shell/proxy
shine install shell/proxy
shine update
shine upgrade shell/proxy
```

Resources use canonical targets such as `app/starship`, `shell/proxy`, and `sys/split-dns`. See the
[quick start](https://biulight.github.io/shine/quick-start) and
[command reference](https://biulight.github.io/shine/reference/commands) for the complete workflow.

## Development

Toolchain versions are pinned in `mise.toml`:

```bash
mise install
bun install --frozen-lockfile
cargo nextest run --all-features
cargo clippy --all-targets --all-features --tests --benches -- -D warnings
cargo fmt --check
bun run check:ts
```

The public documentation site is isolated under `website/`:

```bash
cd website
pnpm install --frozen-lockfile
pnpm check:locales
pnpm typecheck
pnpm build
```

Planning uses the issue workflow in [`docs/PLAN.md`](docs/PLAN.md). Contributor architecture,
invariants, and verification rules live in [`AGENTS.md`](AGENTS.md) and [`docs/kb/`](docs/kb/).

Regular development targets `release`. Version tags are created from `release`, and the release
workflow opens the post-release synchronization PR to `main`.

## License

MIT OR Apache-2.0
