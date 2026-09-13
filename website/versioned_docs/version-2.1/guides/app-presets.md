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

### Migrating legacy App metadata to Shine 2

Current App metadata includes this field at the root of `shine.toml`:

```toml
metadata_schema_version = 2
```

This is separate from `[permissions].schema_version`. If an older overlay replaces
`app/<category>/shine.toml` without `metadata_schema_version`, preview the required changes with
`shine preset migrate --dry-run`, then run `shine preset migrate` after reviewing the diff. You can
also pass a repository, category, or manifest path explicitly.

The migration changes metadata only, keeps payload customizations, and backs up files before
writing. Hooks, generators, and artifacts that cannot be migrated safely are reported for manual
authoring. Validate the result with `shine preset validate` and `shine preset plan`; do not grant
external-code trust merely to bypass an incompatibility. See [Customize presets](./custom-presets.md#migrate-a-1x-source)
for the complete author workflow.

## Uninstall and restore

```bash
shine app uninstall starship --dry-run
shine app uninstall starship
shine app uninstall starship --purge
```

Install, upgrade, uninstall, generator refresh, and artifact apply/remove show a Plan before making
changes. The prompt defaults to No. Use command-level `--yes` only after reviewing the same scope in
an attended automation; it does not bypass permission, trust, or safety checks.

By default, Shine preserves files modified after installation and reports a conflict. Safe uninstall
restores the original backup when one was created. A destination move also requires the old managed
file to be unchanged and the new path to be free. If either location has changed, Shine leaves both
untouched for review.

`app uninstall --force` explicitly permits deletion of user-modified managed content. Preview it
with `--dry-run`. `--purge` also removes the category's installed Preset files. During upgrade,
obsolete managed entries are removed only when `--prune-stale` is part of the approved command;
modified entries remain preserved.

## Recover an interrupted App operation

If an App operation is interrupted, Shine may block later changes to protect the unfinished state.
Read-only commands remain available. Review and apply the recovery action with:

```bash
shine app recover
# Non-interactive only after reviewing the same Plan:
shine app recover --yes
```

Recovery either completes or rolls back the interrupted operation when the affected files still
match the state Shine recorded. Files changed afterward are never overwritten; recovery stops and
preserves the evidence for manual review. JSON recovery changes only keys owned by the Preset and
keeps unrelated values.

The recovery Plan lists administrator access only when protected paths actually need changes.
Rollback files can contain configuration data, so treat them as sensitive. Do not edit or delete
recovery state manually; if recovery reports a conflict, inspect the named paths and keep a backup
before resolving it.

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

Generators can be automatic or manual. Neither kind runs during ordinary `list`, `info`, or
`update`. When Shine cannot determine dynamic desired content without execution, info/update shows
a prominent `generator not evaluated` warning and does not claim that the installed file is
current. Use `--run-generators` to execute the selected generators explicitly, apply transforms in
memory, and inspect status or a final diff without writing destinations or manifests:

```bash
shine app info surge --run-generators
shine info app/surge --run-generators --diff
shine update app/surge --run-generators --diff
shine update --run-generators
```

The global form evaluates generators for every installed App category; the targeted forms evaluate
only the selected App. Both automatic and `auto = false` manual generators participate because the
flag is explicit. External or overlay generators still require matching scoped trust. Generator
failures do not stop evaluation of the remaining selection, but the command returns nonzero after
reporting incomplete results.

Automatic generators may also run during an approved install or upgrade. A manual generator with
`auto = false` runs during installation, explicit evaluation, or explicit refresh:

```bash
shine app refresh <CATEGORY>
shine app refresh <CATEGORY> <SOURCE_FILE>
```

`SOURCE_FILE` is the relative `[[files]].source` path. A failed refresh preserves the last successful
content. A user-modified destination is also preserved unless you explicitly add `--force`.
Installation, including a repair with `--replace-managed`, runs generators enabled by `when_env`
regardless of their `auto` setting. Refresh displays and revalidates a security Plan; automation
must add `--yes`.

Generators supplied by external presets or overlays are executable code and require
`shine trust grant app/<CATEGORY>` after review. Shine passes only explicitly declared environment values and fixed
`SHINE_APP_*` path variables and limits runtime and output size. Run only presets you have reviewed
and trust.

The category's `[permissions]` table separately declares review identities for generator, hook,
and artifact commands, network scopes, and environment-name sensitivity. It is statically
validated but does not enable or trust external code; never put a URL token,
environment value, command arguments, or ciphertext in the declaration.

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

Shine does not implicitly run artifact commands. Each manual apply or remove shows its own Plan and
fails the command if the script fails. Automation must add `--yes` after reviewing that operation.
Preset authors should call the underlying script from a lifecycle hook rather than nesting
`app artifact apply` inside the hook.
When an install or upgrade changes managed files for a category that declares an artifact, Shine
prints the explicit apply command. It prints nothing when no managed files changed.
Scripts receive configured `[artifact].env` sources only, and those sources must also be listed in
the category's `[permissions].environment`, plus path variables such as `SHINE_APP_HTTP_DIR`,
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
and provider cache paths are unchanged. When `shine upgrade` changes this category, its approved
post-upgrade script runs automatically. If the subscription bindings are already current, local
reference changes immediately refresh every provider and close existing connections. If the
rendered binding documents change, the script writes them without contacting providers that mihomo
has not loaded yet; reselect the subscription and run `shine app artifact apply clash-verge`.
After choosing one complete provider set, enable its matching
policy groups and `prepend-rules`. `proxy: DIRECT` on loopback or private services affects only provider downloads;
remove or change it when the server requires a proxy. Private domains that rely on system split DNS
also require mihomo `dns.nameserver-policy` configuration.

The artifact and post-upgrade script use Bun, which must be installed on the machine. The automatic
hook runs only when `shine upgrade` actually changes a managed `clash-verge` file; use the explicit
command for first-time setup, after reselecting changed bindings, or to retry a failed refresh.
External script hooks require a current target-scoped trust grant. Optional
`CLASH_CONTROLLER_URL` and `CLASH_CONTROLLER_TOKEN` values can request an immediate refresh. Without
the URL, only that immediate refresh is skipped; providers still update on their own intervals. The
artifact refreshes every name declared by the effective `merge.yaml` `rule-providers` mapping, so
custom provider names need no matching script change. A missing, null, or empty mapping skips the
refresh; a non-mapping value is reported as invalid configuration. After every declared provider
refreshes successfully, the artifact closes all active mihomo connections so browsers and other
applications reconnect under the new rules without being restarted. This can briefly interrupt
downloads or other long-lived proxied sessions. Never put controller tokens in an overlay or
documentation.

`shine app artifact remove clash-verge` does not clear subscription bindings stored by Clash Verge
Rev. Clear the four editors manually when removing the integration completely.

## Lifecycle hooks

Preset authors can declare `post_install` and `post_upgrade`. The former runs after installation
actually writes files. The latter runs only when `shine upgrade` updates at least one file in that
category. Unchanged categories do not trigger hooks.

Bind every environment input a hook consumes with its `env` list and declare the same names under
the category permission declaration. Values are not displayed in the Plan. Missing required command
inputs block approval; optional script inputs are omitted from the child environment.

Each hook declares exactly one action. `command` runs the declared command and arguments. `script`
runs a native or Bun script with its declared `env` values plus the fixed `SHINE_APP_*` variables.
`runtime` is valid only for a script hook; Bun scripts use the same extension and locked-dependency
rules as artifacts and generators.

```toml
post_upgrade = [
  { command = "my-reloader", env = ["API_URL", "API_TOKEN"] },
]

[permissions]
schema_version = 1
environment = [
  { name = "API_URL", sensitivity = "plain" },
  { name = "API_TOKEN", sensitivity = "secret" },
]
commands = ["my-reloader"]
```

```toml
post_upgrade = [{
  script = "refresh.ts",
  runtime = "bun",
  env = ["API_URL", "API_TOKEN"],
}]

[permissions]
schema_version = 1
filesystem = [{ access = ["execute"], base = "preset", path = "refresh.ts" }]
network = [{ scope = "any" }]
commands = ["bun"]
environment = [
  { name = "API_URL", sensitivity = "plain" },
  { name = "API_TOKEN", sensitivity = "secret" },
]
```

Hooks and generators in external presets require target-scoped trust:

```bash
shine trust inspect app/<CATEGORY>
shine trust grant app/<CATEGORY>
```

`trust inspect` is read-only. Before granting trust, resolve every missing permission declaration
shown by the Plan. If an active overlay's `app/<CATEGORY>/shine.toml` is an old full copy that
overrides built-in metadata, remove or migrate that metadata file first; overlay payload files such
as `merge.yaml` and `rules/` remain usable.

Hooks hide stdout by default. When a preset sets `show_output = true`, successful output is shown
during installation and refresh; `shine upgrade` reserves successful hook completion and output for
`--verbose`. A hook failure or permission block is always shown and does not interrupt installation
or upgrades for other categories.
