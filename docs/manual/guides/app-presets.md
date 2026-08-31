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

Install, upgrade, uninstall, generator refresh, and artifact apply/remove show a snapshot-bound Plan
before mutation. The prompt defaults to No; use command-level `--yes` for non-interactive execution.
`--yes` still renders and revalidates the Plan and cannot bypass missing permissions, blocked
teardown, or external-code gates. App stale files are removed during upgrade only when
`--prune-stale` was part of the reviewed command. Unchanged static Copy and JSON stale entries use
the same receipt-gated journal as uninstall; user-modified stale content remains preserved.

When metadata moves a static Copy file to a different effective destination, upgrade journals the
old receipt and destination, optional fixed backup, rollback path, and absent new destination as one
relocation. The old managed file must be unchanged (or already missing without a backup), and the
new destination must be free. An occupied new path or changed old file is preserved as a conflict.

By default, files modified after installation are preserved and reported as user-modified. A safe
uninstall restores any backup created during installation. Before a supported journaled static Copy
replaces an unowned regular-file destination, Shine requires its fixed `<name>.shine.bak` path to be
absent; an existing backup blocks the Plan and both files are preserved. `--purge` also removes the
category's preset directory; uninstalling every category also removes the manifest.

`app uninstall --force` explicitly authorizes deletion of user-modified managed content. For an
eligible static Copy, the reviewed Plan marks that override and the transaction stages the modified
file at `<name>.shine.rollback` until receipt commit; an optional fixed backup is restored in the
same transaction. Administrator static Copy files use the same journal and recovery contract for
creation, in-place update, and removal while their protected writes, moves, mode restoration, and
cleanup run with administrator access. JSON merge is also journaled for install, in-place update,
ordinary uninstall, and forced uninstall. Other install strategies retain their existing lifecycle
path. Preview destructive intent with `--dry-run`.

## Recover an interrupted App operation

Shine writes an operation journal before the supported App file mutation. If the process stops
after that point, mutating App install, upgrade, uninstall, refresh, and artifact commands remain
blocked so they cannot silently discard recovery state. Read-only status/update inspection does not
recover or remove the journal. Review and apply the dedicated recovery Plan with:

```bash
shine app recover
# Non-interactive only after reviewing the same Plan:
shine app recover --yes
```

For an originally absent destination, recovery removes a transaction-created file only when it is
still byte-for-byte the content Shine wrote. For backup-aware creation, it restores the fixed backup
only when the backup still matches the original bytes and the destination is missing or still
matches the managed bytes; if the backup move never started, it keeps the original destination. If
a receipt-owned static Copy is replaced in place, Shine temporarily moves the previous managed file
to the same-directory `<name>.shine.rollback` path. Before the replacement receipt is durable,
recovery restores it only while the destination and rollback file still match the previous/desired
fingerprints. After the replacement receipt is durable, recovery preserves the destination and
removes only unchanged rollback material plus the stale journal. An ordinary uninstall of an
unchanged static Copy without a persistent backup also moves the managed file to this rollback path
until receipt removal is durable. Recovery restores it while the exact old receipt remains, or
removes it after both receipt removal and the journal's matching commit state are durable, and only
while its kind, mode, and bytes are unchanged. If receipt removal is durable but that journal state
is missing, recovery conservatively recreates the old receipt and restores the unchanged file.
When that static Copy has a fixed persistent backup, uninstall journals both moves: the managed
file goes to `.shine.rollback`, then `.shine.bak` returns to the destination. Before receipt commit,
recovery accepts only the exact three-path states produced before, between, or after those moves;
it returns the restored user file to `.shine.bak` when necessary, then restores the managed file and
old receipt. After receipt commit, recovery keeps the unchanged user file at the destination and
removes only unchanged managed rollback material. The modes and bytes of both files are bound.
For a forced removal of a user-modified static Copy, recovery instead binds the old
receipt hash separately from the modified file's mode and hash. Before receipt commit it restores
that exact modified file and reverses an optional backup restoration; after commit it keeps the
completed uninstall and removes only exact modified rollback material.
For JSON merge, the declared top-level keys are the ownership boundary. An existing whole JSON
object is moved to `.shine.rollback`, but recovery reads it only to restore those keys into the
current object, preserving unrelated values changed after interruption. Creation at an absent path
removes the whole file only when no unrelated keys exist. After uninstall receipt commit, the
current JSON object is user-owned and recovery removes only unchanged rollback material—even if the
user has reintroduced a formerly managed key.
For `upgrade --prune-stale`, unchanged static Copy and JSON entries use the same removal recovery
contract. If receipt removal is interrupted before its positive commit marker, recovery recreates
the old receipt and restores only exact rollback state. A missing destination needs receipt-only
cleanup, and user-modified stale content is never forced through this path.
For a static Copy relocation, recovery before the new receipt removes only an unchanged new file,
returns a restored user file to the old fixed backup when necessary, and restores the exact old
managed file. After the new receipt is durable, it preserves both final destinations and removes
only unchanged old rollback material. JSON relocation retains its existing lifecycle path.
When one of these creation, update, relocation, or removal recovery operations changes an
administrator path,
the recovery Plan includes administrator permission and Shine requests authorization only after
that Plan is approved. Repair that only reconstructs a receipt or clears a journal does not request
administrator access.
A rollback file may contain prior managed configuration and should be treated as sensitive. If any
guarded path changed after interruption, recovery returns nonzero and preserves the paths plus the
journal; replacing a regular file with a symlink or directory also counts as a change. Do not edit
or delete the journal or rollback material manually.

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

Shine does not implicitly run artifacts, although a preset may call `app artifact apply` from a
lifecycle hook after installation or upgrade actually changes files. A failed manual apply fails the
command. Manual apply/remove displays and revalidates a security Plan; automation must add `--yes`.
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
and provider cache paths are unchanged. The first upgrade that adds the managed references may run
the preset's existing immediate-refresh hook once, without changing the active provider definitions.
After choosing one complete provider set, enable its matching
policy groups and `prepend-rules`. `proxy: DIRECT` on loopback or private services affects only provider downloads;
remove or change it when the server requires a proxy. Private domains that rely on system split DNS
also require mihomo `dns.nameserver-policy` configuration.

This artifact uses Bun, which must be installed on the machine. Preset hooks rerun the build after
`merge.yaml` or a managed local reference list changes; external presets require a current
target-scoped trust grant. Optional
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
the category permission declaration. Plan review hashes `plain` values and binds `secret` values by
an opaque revision; neither value is serialized into the Plan. A missing hook input or secret
identity blocks approval.

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

Hooks and generators in external presets require target-scoped trust:

```bash
shine trust inspect app/<CATEGORY>
shine trust grant app/<CATEGORY>
```

Hooks hide stdout by default. When a preset sets `show_output = true`, successful output is shown
during installation and refresh; `shine upgrade` reserves successful hook completion and output for
`--verbose`. A hook failure or permission block is always shown and does not interrupt installation
or upgrades for other categories.
