---
title: Manage application configuration
sidebar_position: 2
---

# Manage application configuration

Application presets install configuration where the target application expects it and record managed
files in `~/.shine/app-manifest.toml`. If installation encounters an existing unmanaged file, Shine
first creates a `*.shine.bak` backup.

Presets manage configuration only; they do not install, download, or start the application. See
[built-in presets](../reference/built-in-presets.md#application-presets) for destinations, platforms,
permissions, and restart requirements.

## Browse and preview

```bash
shine app list
shine app info starship
shine app install starship --dry-run
```

Presets that target system directories may need additional permissions. Use `--dry-run` first to
confirm destinations and scope.

## Install and update

```bash
shine app install starship
shine install app/starship
shine update
shine upgrade
```

`shine update` compares installed results with presets and reports status without applying it.
`shine upgrade` updates managed shell and application configuration to current preset content.

To replace managed files for one category:

```bash
shine app install starship --replace-managed
```

## Uninstall and restore

```bash
shine app uninstall starship --dry-run
shine app uninstall starship
shine app uninstall starship --purge
```

By default, files modified after installation are preserved and reported as user-modified. A safe
uninstall restores any backup created during installation. `--purge` also removes the category's
preset directory; uninstalling every category also removes the manifest.

## Configuration transforms

Some presets process source files before installation:

- `jsonc-to-json` removes JSONC comments and trailing commas and writes standard JSON.
- `template` replaces `@@VAR_NAME@@` with the current `[env]` value.
- The `json-merge` strategy manages declared top-level keys while preserving other user settings.

`shine update` compares the final transformed result, not the original preset file.

## Generated files and Surge URI subscriptions

An application `[[files]]` entry can declare a generator whose UTF-8 stdout becomes the expected
managed content. Generated results still pass through normal transforms, hashing, manifest tracking,
user-modification protection, and uninstall. A script must not bypass Shine and write the destination
directly.

Generators can be automatic or manual. Automatic generators may participate in installation,
read-only status checks, and upgrades. A manual generator with `auto = false` never runs during
`list`, `info`, `update`, or `upgrade`; refresh its already-installed file explicitly:

```bash
shine app refresh <CATEGORY>
shine app refresh <CATEGORY> <SOURCE_FILE>
```

`SOURCE_FILE` is the relative `[[files]].source` path. A failed refresh preserves the last successful
content. A user-modified destination is also preserved unless you explicitly add `--force`.
Installation, including a repair with `--replace-managed`, runs generators enabled by `when_env`
regardless of their `auto` setting.

Generators supplied by external presets or overlays are executable code and require
`allow_app_hooks = true`. Shine passes only explicitly declared environment values and fixed
`SHINE_APP_*` path variables and limits runtime and output size. Run only presets you have reviewed
and trust.

### Surge URI subscriptions

The built-in `surge` preset can convert an HTTPS Base64 URI subscription into a managed
`subscription-proxies.conf`. It requires Bun and supports compatible `ss://` and `vmess://` records.
VLESS, unsupported transports, plugins, malformed records, and duplicates are skipped; diagnostics
contain no credentials. User-maintained `local-proxies.conf` is not rewritten.

To customize local proxies, policy groups, or rules, first copy the complete preset into a local
overlay:

```bash
mkdir -p ~/dotfiles/shine-overlay
cd ~/dotfiles/shine-overlay
shine preset copy app/surge
shine preset overlay link .
```

Edit `app/surge/local-proxies.conf`, `local-proxy-groups.conf`, or `local-rules.conf`, then install.
If you customize only some files, delete the other copied files so they continue to come from the
built-in preset and receive Shine updates. Do not edit managed copies in the Surge Profiles directory.

Configure the URL and install:

```bash
shine env set SURGE_SUBSCRIPTION_URL 'https://provider.example/subscription?...'
shine app install surge
```

The generator is manual, so routine `shine update` and `shine upgrade` never access the subscription.
Open the provider's access window and refresh explicitly:

```bash
shine app refresh surge subscription-proxies.conf
```

When refreshed content changes, the existing `post_upgrade` hook reloads Surge. Failure preserves the
last successful file. The `Subscription` group in `local-proxy-groups.conf` reads nodes through
`policy-path=subscription-proxies.conf`; other groups can include them with
`include-other-group=Subscription`.

## Build helper resources

Some application presets declare a script in `[artifact]`. Generate or refresh its resources
explicitly:

```bash
shine app artifact apply surge
```

Shine does not implicitly run artifacts, although a preset may call `app artifact apply` from a
lifecycle hook after installation or upgrade actually changes files. A failed manual apply fails the
command. Scripts receive current `[env]` values and path variables such as `SHINE_APP_HTTP_DIR`,
`SHINE_CACHE_DIR`, and `SHINE_STATE_DIR`. They can generate resources under
`~/.shine/http/app/<APP_ID>/`. See [Tasks and the local service](./tasks-and-serve.md) for the complete
variable list.

The built-in `surge` preset installs `local-proxies.conf`, `local-proxy-groups.conf`,
`local-rules.conf`, and the optional subscription file in the Surge Profiles directory. After setting
`SURGE_PROFILE` in `[env]`, `shine app artifact apply surge` uses a built-in Bun artifact to
idempotently patch `[Proxy]`, `[Proxy Group]`, and `[Rule]` `#!include` lines in the active profile.
An overlay supplies only its policy files and does not need its own build script.

The preset includes commented, inert examples for `LAN Network`, `LAN PROXY`, and `Other Direct`.
Each traffic class has three mutually exclusive rule sources in `local-rules.conf`: relative
`rules/*.list` files installed with the profile, loopback HTTP on the same device, or a remote HTTPS
URL whose domain you replace. Enable one source per class. Relative files are usually simplest.
`localhost` always means the device running Surge; on iOS it is not another LAN host.

Undo the patch with:

```bash
shine app artifact remove surge
```

`artifact remove` runs only the declared teardown script. Uninstall also attempts teardown when one
is declared; a cleanup failure warns but does not stop safe removal of managed files.

### Clash Verge Rev

The built-in `clash-verge` preset contains an inert `merge.yaml` example. To add your proxies, groups,
rule providers, and prepended rules, copy the complete preset to a local overlay:

```bash
mkdir -p ~/dotfiles/shine-overlay
cd ~/dotfiles/shine-overlay
shine preset copy app/clash-verge
shine preset overlay link .
```

Edit `app/clash-verge/merge.yaml`. Do not modify `~/.shine/clash-verge/`, which is the managed installed
copy. If only `merge.yaml` is customized, delete the other copied files so they continue to use and
track the built-in versions.

Install after reviewing the content:

```bash
shine app install clash-verge
```

For first use, open and save the current subscription's **Extend Config**, **Edit Rules**,
**Edit Proxies**, and **Edit Groups** editors in Clash Verge Rev, then run:

```bash
shine app artifact apply clash-verge
```

Shine reads `profiles.yaml` only to locate those bound files. It never modifies subscriptions,
creates bindings, or writes remote subscription YAML. After the build writes new content, reselect the
subscription in Clash Verge Rev; running the build again can request an immediate rule-provider
refresh.

The example uses the same three traffic classes and offers three mutually exclusive provider layouts:
a mihomo `type: file` path inside `HomeDir`, loopback HTTP on the same device, or remote HTTPS. Shine
installs three inert reference lists under `HomeDir/ruleset/shine-source/` through ordinary managed
app-file entries; customize those files in an overlay only when choosing the file-provider layout.
The loopback and remote HTTP layouts do not reference these local files, so their URLs, intervals,
and provider cache paths are unchanged. The first upgrade that adds the managed references may run
the preset's existing immediate-refresh hook once, without changing the active provider definitions.
After choosing one complete provider set, enable its matching
policy groups and `prepend-rules`. `proxy: DIRECT` on loopback or private services affects only provider downloads;
remove or change it when the server requires a proxy. Private domains that rely on system split DNS
also require mihomo `dns.nameserver-policy` configuration.

This artifact uses Bun, which must be installed on the machine. Preset hooks rerun the build after
`merge.yaml` or a managed local reference list changes; external presets require `allow_app_hooks`. Optional
`CLASH_CONTROLLER_URL` and `CLASH_CONTROLLER_TOKEN` values can request an immediate refresh. Without
the URL, only that immediate refresh is skipped; providers still update on their own intervals. The
artifact refreshes every name declared by the effective `merge.yaml` `rule-providers` mapping, so
custom provider names need no matching script change. A missing, null, or empty mapping skips the
refresh; a non-mapping value is reported as invalid configuration. Never put controller tokens in an
overlay or documentation.

`shine app artifact remove clash-verge` does not clear subscription bindings stored by Clash Verge
Rev. Clear the four editors manually when removing the integration completely.

## Lifecycle hooks

Preset authors can declare `post_install` and `post_upgrade`. The former runs after installation
actually writes files. The latter runs only when `shine upgrade` updates at least one file in that
category. Unchanged categories do not trigger hooks.

Hooks and generators in external presets require explicit permission:

```toml
allow_app_hooks = true
```

Hooks hide stdout by default. When a preset sets `show_output = true`, successful output is shown
during installation and refresh; `shine upgrade` reserves successful hook completion and output for
`--verbose`. A hook failure or permission block is always shown and does not interrupt installation
or upgrades for other categories.
