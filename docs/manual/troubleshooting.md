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
shine info proxy
```

If the source script exists but its command entry is missing, `shine list` does not report it as
available. Rebuild managed files and entries with
`shine shell install <CATEGORY> --replace-managed`.

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
source. Check the active source in command output and compare the precedence rules in the
[configuration reference](./reference/configuration.md).

To isolate the check from existing configuration, use a separate directory:

```bash
SHINE_CONFIG_DIR=/tmp/shine-check shine app list
```

Shine uses this directory for configuration and runtime state and does not read the original
`~/.shine/`.

## Installed configuration does not change after updating an environment value

`shine env set` updates the value but does not automatically rewrite installed templates. Run:

```bash
shine update --verbose
shine upgrade --verbose
```

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

The built-in Surge generator requires an HTTPS `SURGE_SUBSCRIPTION_URL` and Bun at runtime. A failed
refresh does not remove the last successfully generated file. If the destination is user-modified,
inspect the difference and use `--force` only when you intend to replace it. Routine `shine update`
and `shine upgrade` runs never access this manual subscription generator.

## `shine preset pull` refuses to update a source

`shine preset pull` performs a fast-forward update only for a clean normal branch with an upstream.
Enter the repository shown in the error and inspect it:

```bash
git status
git branch --show-current
git branch -vv
git pull --ff-only
```

Commit or stash local changes and resolve branch divergence yourself, then rerun `shine preset pull`.
Shine never discards changes or resolves conflicts automatically. If Git is missing, install it and
make sure `git` is in `PATH`. Non-Git preset directories are skipped normally.

## Preview system initialization effects

```bash
shine sys info <ITEM>
shine sys bootstrap --dry-run
shine sys uninstall <ITEM> --dry-run
```

Do not infer available items from planning documents. Use `shine sys list` and `shine sys info` from
the installed version.

PowerShell, bash, or zsh profiles rewritten by Windows or other tools may use CRLF line endings.
System-profile merging matches managed blocks by content and does not repeatedly rewrite files only
because CRLF and LF differ. If an older version left conflict markers, resolve them manually before
rerunning `shine sys bootstrap --dry-run` or the relevant `upgrade`.

## A resource is unavailable through the local HTTP service

Check the service and URL:

```bash
shine serve status
shine serve url app/surge/custom-rules.sgmodule
```

`shine serve install` currently supports a macOS user service only. On other platforms, run
`shine serve start` in the foreground. The service publishes only files under `~/.shine/http/`. If a
resource is missing, first run the relevant `shine app artifact apply <APP_ID>`.

Do not put sensitive files under `~/.shine/http/`. The service binds to `127.0.0.1` but has no
additional authentication.

## A saved task behaves differently from the original command

`shine task` does not invoke a shell; it starts the saved argument array directly. Commands that use
pipes, redirection, variable expansion, or globs must save an explicit shell:

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

## Automatic update checks fail

When the network or GitHub API is unavailable, Shine skips the version check and continues the
original command. After connectivity returns, bypass the 24-hour cache and check again:

```bash
shine update --refresh-release
```
