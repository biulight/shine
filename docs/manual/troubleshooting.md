---
title: Troubleshooting
sidebar_position: 6
---

# Troubleshooting

Start by running `shine --version` to record the version. Then use `shine list`,
`shine info <TARGET>`, or the relevant command's `--dry-run` option to collect state.

## A command is missing after installation

Shell preset commands are exposed through `~/.shine/bin/`. Open a new terminal after installation
or reload the shell profile:

```bash
source ~/.zshrc
# Or: source ~/.bashrc
```

Then inspect the preset:

```bash
shine list
shine info shell/proxy
```

If a command entry is missing, use `shine info shell/<CATEGORY>` to inspect it. Rebuild the
managed files and command entries with `shine shell install <CATEGORY> --replace-managed` after
reviewing any local changes.

## Application configuration is reported as user-modified

Shine preserves files changed after installation by default. Inspect the difference first:

```bash
shine info app/starship --diff
```

Run `shine app install starship --replace-managed` to use the preset version. During uninstall, use
`shine app uninstall starship --force` only when you explicitly want to delete those local changes;
preview the same command with `--dry-run` first.

## Shine is using an unexpected preset source

External directories, project configuration, and environment variables can change the active
source. Check `Preset Source` and `Presets Overlay` in command output and compare the precedence rules in the
[configuration reference](./reference/configuration.md).

To inspect account configuration alone, move outside projects containing `shine.config.toml`
before checking the source. `SHINE_CONFIG_DIR` changes the account configuration and state directory;
it does not disable project configuration discovery.

## Installed configuration does not change after updating an environment value

`shine env set` updates the value but does not automatically rewrite installed templates. Run:

| Operation | Command |
| --- | --- |
| Inspect pending changes | `shine update --verbose` |
| Apply updates with detailed output | `shine upgrade --verbose` |

If you use a project `shine.config.toml`, project `shine.env.toml`, or overlay, confirm the current
working directory and override precedence.

If a private domain or `192.168.x.x` address is still intercepted by a terminal proxy, continue with
the Chinese Biulight knowledge-base guide
[排查终端代理误拦截 ZeroTier 私有域名](https://blog.biulight.top/timeline/knowledge/terminal-proxy-no-proxy-zerotier).

## Shine reports an old configuration file after upgrading to 0.40

Project `config.toml` and `.env.toml` files are no longer read as Shine configuration. Rename them to
`shine.config.toml` and `shine.env.toml`, respectively.

If Shine reports a global `~/.shine/env.toml`, do not delete values that are still in use. Move it to
`~/.shine/shine.env.toml`; if the destination already exists, merge it manually and check for
duplicate keys. Alternatively, before upgrading, load the configuration once with v0.39 so that the
old version performs its automatic migration.

## Refreshing a generated application file fails

Confirm that the category is installed, the selector is the relative `[[files]].source` path, and
the generator's environment prerequisites are present:

```bash
shine app info surge
shine env list
shine app refresh surge subscription-proxies.conf
```

Check these prerequisites in order:

1. Install Bun and make sure `SURGE_SUBSCRIPTION_URL` or `W_GET_CLOUD_SURGE_SUBSCRIPTION_URL`
   contains an HTTPS subscription URL.
2. For an HTTP error, timeout, oversized response, or no compatible nodes, check the subscription
   service. If it returns a complete Surge managed configuration, use its generic URI/Base64
   subscription URL instead; Shine does not extract proxy sections from a complete configuration.
3. If the destination was modified, inspect the difference. Add `--force` only if you intend to
   replace those local changes.

A failed refresh preserves the last successfully generated file. Routine `shine update` and
`shine upgrade` do not refresh this manual subscription; run `shine app refresh` explicitly.
Errors redact subscription URLs, response bodies, and node content. Still check output for
secrets before including it in a bug report.

## `shine preset pull` refuses to update a source

First check the source type. An overlay created with `shine preset overlay link --git` is a
Shine-managed mirror: pulling discards local edits and extra files in that mirror. Edit its
upstream repository instead. See [Git overlays](./guides/custom-presets.md).

For an external source or manually linked overlay that you maintain yourself, `shine preset pull`
only fast-forwards a clean normal branch with an upstream. Enter the repository shown in the error
and inspect it:

```bash
git status
git branch --show-current
git branch -vv
git pull --ff-only
```

Commit or stash local changes and resolve branch divergence yourself, then rerun `shine preset pull`.
For these manually maintained sources, Shine does not discard changes or resolve conflicts automatically. If Git is missing, install it and
make sure `git` is in `PATH`. Non-Git preset directories are skipped normally.

## Preview system initialization effects

```bash
shine sys info <ITEM>
shine sys bootstrap --dry-run
shine sys uninstall <ITEM> --dry-run
```

Use `shine sys list` and `shine sys info` to check the items and prerequisites available in your
installed version.

If an older version left conflict markers in your shell profile, resolve them manually before
rerunning `shine sys bootstrap --dry-run` or the relevant `upgrade`.

## A saved task behaves differently from the original command

For tasks that use pipes, redirection, variable expansion, or globs, include a shell explicitly:

```bash
shine task save kill-port -- sh -c 'lsof -ti :3000 | xargs kill'
```

Additional arguments are appended to the saved command:

```bash
shine task run my-task -- --verbose
```

## SSH file transfer is unavailable

`shine local` works only inside a remote shell opened by `shine ssh`. If
`SHINE_SSH_SESSION`, `SHINE_SSH_TOKEN`, or `SHINE_SSH_REMOTE_SOCK` is missing, exit and reconnect with
`shine ssh <HOST>`.

The remote host must also be able to run a compatible `shine local`. Check:

```bash
shine local status
which shine
shine --version
```

Preview paths and overwrite behavior before transferring:

```bash
shine local download ./remote.log ./remote.log --dry-run
shine local upload ./local.log /tmp/local.log --dry-run
```

Existing files are not overwritten by default. Add `--force` only after confirming the target. For
an existing directory target, `--force` means merge into the directory.

## Maintenance reminders are missing

For manual-maintenance reminders, run `shine state check`. Native service checks normally finish
in the background and appear on a later interactive invocation; redirected or automated commands
skip automatic maintenance checks. If an inspection cannot complete, check access to the current
user's service manager and retry. To remove an old HTTP service, run `shine clear serve`.

## Automatic update checks fail

When the network or GitHub API is unavailable, Shine skips the version check and continues the
original command. After connectivity returns, bypass the 24-hour cache and check again:

```bash
shine update --refresh-release
```
