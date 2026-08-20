# Shine

A cross-platform Rust CLI for keeping a development environment portable, usable, and safe across
machines and remote sessions.

Shine turns shell commands, application configuration, bootstrap steps, environment values, and
remote-session workflows into explicit resources. It tracks what it owns, lets you inspect changes
before applying them, and removes managed content without taking ownership of unrelated user files.

**Documentation:** [English](https://biulight.github.io/shine/) ·
[简体中文](https://biulight.github.io/shine/zh-Hans/)

## What Shine connects

- **Setup across machines:** install and reconcile shell and application presets, then use focused
  system bootstrap scripts for a new macOS, Ubuntu, or Windows environment.
- **Repeatable terminal work:** save argv-based tasks, install portable helper commands, synchronize
  terminal themes, and expose generated local resources.
- **Local–remote continuity:** carry selected environment values into `shine ssh` sessions and move
  files through the authenticated SSH connection without setting up a separate transfer service.
- **Secrets with a boundary:** encrypt workspace values with GPG or age, or let a remote AI/tooling
  workflow request narrowly authorized secrets from a local policy-bound broker.

The built-in presets are both usable defaults and starting points. Opinionated artifacts for Surge
and Clash Verge Rev reduce provider-specific setup work, while `preset copy`, overlays, and external
Git sources let you change only what your environment needs.

`shine sys` has a deliberately narrower boundary than the app and shell lifecycle: its built-in
scripts initialize selected tools, and a small driver set manages reversible system resources such
as split DNS. It does not own third-party runtime versions. For example, a sys preset can install
and activate mise, but mise continues to own its configuration and tool versions.

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

Lifecycle operations use category-level targets such as `app/starship`, `shell/proxy`, and
`sys/split-dns`; file and command identities appear as inspection details. See the
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
