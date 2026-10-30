---
title: Manage application configuration
sidebar_position: 2
---

# Manage application configuration

Application presets install configuration where the application expects it. Existing unmanaged files
are backed up as `*.shine.bak` before replacement.

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
confirm destinations and scope without writing application files.

## Install and update

```bash
shine app install starship
shine install app/starship
shine update
shine upgrade
```

`shine update` compares installed results with presets and reports status without applying it.
`shine upgrade` updates managed shell and application configuration to current preset content.

If installation fails, some files may already be installed. Check the reported results before retrying.

Reinstalling a category preserves files changed since installation, including managed JSON keys.
Shine also skips the generator for a file that the reviewed Plan marks for preservation. To
explicitly replace modified managed content, preview `shine app install starship --replace-managed --dry-run`
and then review `shine app install starship --replace-managed`.

If another App operation is running, wait for it to finish. Review a new Plan if Shine reports
that the relevant files or settings have changed.

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
```

Install, upgrade, uninstall, generator refresh, and artifact apply/remove show a Plan before making
changes. The prompt defaults to No. Use command-level `--yes` only after reviewing the same scope in
an attended automation; it does not bypass permission, trust, or safety checks.

By default, Shine preserves files modified after installation and reports a conflict. Safe uninstall
restores the original backup when one was created. A destination move also requires the old managed
file to be unchanged and the new path to be free. If either location has changed, Shine leaves both
untouched for review.

If a generated file moves to another destination, check the old file and its backup before approving
the move. Missing or changed backups can require manual repair; generated-file moves are not covered
by `shine app recover`.

`app uninstall --force` permits deleting user-modified managed content; preview it with `--dry-run`.
Uninstall also deletes the selected category's entire Shine cache, including any extra files there.
Without a category, it clears all App caches. External preset sources are preserved. Keep personal
files outside these caches and review the removal Plan before approval.

`shine uninstall app/<category>` uses the same category-limited behavior. The old `--purge` option
has been removed. Resolve protected or failed removals before retrying; do not delete installation
records or backups manually.

During upgrade,
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

A `[[files]]` generator writes the intended file content to UTF-8 standard output; Shine applies
transforms and manages installation and removal. The script must not write the destination directly.
Output is limited to 8 MiB on stdout and 64 KiB on stderr; exceeding either limit fails the generator.

Generators can be automatic or manual. Neither kind runs during ordinary `list`, `info`, or
`update`. When Shine cannot determine dynamic desired content without execution, info/update shows
a prominent `generator not evaluated` warning and does not claim that the installed file is
current. Use `--run-generators` to execute the selected generators explicitly and inspect the resulting status or diff without installing the files:

```bash
shine app info surge --run-generators
shine info app/surge --run-generators --diff
shine update app/surge --run-generators --diff
shine update --run-generators
```

The global form evaluates generators for every installed App category; the targeted forms evaluate
only the selected App. Both automatic and `auto = false` manual generators participate because the
flag is explicit. External or overlay generator evaluation with `--run-generators` still requires matching scoped trust;
interactive lifecycle operations can obtain one-time code consent. Generator
failures do not stop evaluation of the remaining selection, but the command returns nonzero after
reporting incomplete results. Cancellation, command-wide analysis budget exhaustion, or an analyzer
failure that prevents safe continuation stops the remaining work.

Before execution, supported App Bun code is checked against its declarations. A definite difference
or parse failure prevents that script from starting; existing trust does not override it. Other
independent selected files may still be evaluated. See [App code checks](custom-presets.md#check-app-code-before-execution)
for fixes, coverage limits and author tools. Evaluation still does not install files, update receipts,
or run hooks or artifacts.

Refresh applies only to generated files already tracked by the App manifest. After adding a new
generated `[[files]]` entry, run `shine install app/<CATEGORY>` once before refreshing that source.

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

For external or overlay generators, interactive lifecycle confirmation can authorize one operation.
Automation and `--run-generators` inspection require a matching `shine trust grant app/<CATEGORY>`
after review. Generators receive declared environment inputs and fixed `SHINE_APP_*` path variables;
execution time and output size are limited. Run only presets you trust.

Permission declarations help you review a preset; they do not sandbox its code. Never include
secret values or credentials in those declarations. See
[capability declarations](./custom-presets.md#declare-permissions) when authoring a preset.

### Surge URI subscriptions

The built-in `surge` preset can convert HTTPS Base64 URI subscriptions into managed proxy files.
`subscription-proxies.conf` reads `SURGE_SUBSCRIPTION_URL`; `wget-cloud-proxies.conf` reads
`W_GET_CLOUD_SURGE_SUBSCRIPTION_URL`. The generator requires Bun and supports compatible `ss://`,
`vmess://`, and `trojan://` records. Trojan TCP and WebSocket transports are supported. VLESS,
unsupported transports or security modes, plugins, malformed records, and duplicates are skipped;
diagnostics contain no credentials. User-maintained `local-proxies.conf` is not rewritten.

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
shine env set W_GET_CLOUD_SURGE_SUBSCRIPTION_URL 'https://provider.example/wget-cloud?...'
shine app install surge
```

The generator is manual, so routine `shine update` and `shine upgrade` never access the subscription.
Open the provider's access window and refresh explicitly:

```bash
shine app refresh surge subscription-proxies.conf
shine app refresh surge sub/wget-cloud-proxies.conf
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
Scripts receive configured sources from their structured `[artifact].env` entries, plus path variables such as
`SHINE_CACHE_DIR` and `SHINE_STATE_DIR`. See
[Tasks and application artifacts](./tasks-and-serve.md) for the complete variable list.

Keep script output that must survive execution in `SHINE_STATE_DIR` or `SHINE_CACHE_DIR`.
Temporary source copies are removed afterward.

The built-in `surge` preset installs `local-proxies.conf`, `local-proxy-groups.conf`,
`local-rules.conf`, and the optional subscription file in the Surge Profiles directory. After setting
`SURGE_PROFILE` in `[env]`, `shine app artifact apply surge` uses a built-in Bun artifact to
idempotently patch `[Proxy]`, `[Proxy Group]`, and `[Rule]` `#!include` lines in the active profile.
An overlay supplies only its policy files and does not need its own build script.

If a legacy Shine proxy-group block has a missing or nested marker, both applying and undoing the
patch fail without changing the profile. Repair the malformed markers in your profile before retrying.

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
External script hooks require human confirmation of the lifecycle Plan or a matching grant. Optional
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

Declare each environment input a hook consumes with `source`, optional `target`, and `sensitivity`
in its structured `env` list; no duplicate category environment declaration is needed. Values are not displayed in the Plan. Missing required
environment inputs block approval; optional script inputs are omitted from the child environment.

Each hook declares exactly one action. `command` runs the declared command and arguments. `script`
runs a native or Bun script with its declared `env` values plus the fixed `SHINE_APP_*` variables.
`runtime` is valid only for a script hook; Bun scripts use the same extension and locked-dependency
rules as artifacts and generators.

```toml
post_upgrade = [
  { command = "my-reloader", env = [{ source = "API_URL", sensitivity = "plain" }, { source = "API_TOKEN", sensitivity = "secret" }] },
]

[permissions]
schema_version = 1
```

```toml
post_upgrade = [{
  script = "refresh.ts",
  runtime = "bun",
  env = [{ source = "API_URL", sensitivity = "plain" }, { source = "API_TOKEN", sensitivity = "secret" }],
}]

[permissions]
schema_version = 1
network = [{ scope = "any" }]
```

For persistent authorization of external hooks and generators, use target-scoped trust.
Interactive lifecycle confirmation can instead authorize only the current operation:

```bash
shine trust inspect app/<CATEGORY>
shine trust grant app/<CATEGORY>
shine trust grant app/<CATEGORY> --development
```

`trust inspect` shows what needs authorization without granting trust. Snapshot trust binds the complete
effective App category, including helper and data files. Use `--development` only when you intend
to trust later code edits from the displayed local source. Before granting trust, resolve missing
environment inputs or Administrator requirements shown by the Plan. If an active overlay's `app/<CATEGORY>/shine.toml` is an old full copy that
overrides built-in metadata, remove or migrate that metadata file first; overlay payload files such
as `merge.yaml` and `rules/` remain usable.

Hooks hide stdout by default. When a preset sets `show_output = true`, successful output is shown
during installation and refresh; `shine upgrade` reserves successful hook completion and output for
`--verbose`. A hook failure or permission block is always shown and does not interrupt installation
or upgrades for other categories.

## Managed secret files {#managed-secret-files}

App metadata version 3 can install encrypted workspace files and manage their decrypted targets.
Keep plaintext out of the preset repository. Use the existing workspace version 3 `file_groups`
mappings and encryption policy; identities remain on the decrypting machine.

```toml
metadata_schema_version = 3

[dest]
linux = "/srv/example-service"

[[files]]
source = "compose.yml"

[[secret_files]]
workspace = "shine.workspace.toml"
file_group = "credentials"
mode = "production"
auto_decrypt = true
unix_uid = 1000
unix_gid = 1000
unix_mode = "0400"
```

`workspace` is relative to the category. Its selected ciphertext files and the workspace definition
are installed with the same relative layout; do not duplicate them in `[[files]]`. Plaintext targets
come from the workspace mappings and must not overlap other managed resources. Secret files do not
support generators, transforms, JSON merge, or automatic destination relocation.

`auto_decrypt` defaults to `false`. When enabled, install and upgrade decrypt required files after
Plan approval and before modifying the category's destination files. Inside an enabled SSH broker
session, decryption and hardware authorization run on the local computer. Otherwise they use the
installing machine's configured backend. Broker failure never falls back to another backend.
See [SSH file authorization](ssh-transfer.md#file-secret-broker).

Unix defaults are the installing user's ownership and `0600`; optional numeric `unix_uid`,
`unix_gid`, and `unix_mode` (`0400` or `0600`) select the service account's access. Run installation
as an account allowed to create the destination and set that ownership. Windows uses private
current-user ACLs and rejects Unix permission fields. Review permissions before installing.

An existing unowned target is adopted only when its content exactly matches the decrypted file.
Different content is a conflict, including with `--replace-managed`; no plaintext `.shine.bak`
is created. Later local edits are preserved unless explicitly replaced. Unchanged files skip
decryption. Read-only commands and diffs show status without decrypting or displaying content.

Uninstall removes unchanged managed secret files and preserves local edits. Turning automatic
decryption off pauses creation and updates but retains existing ownership. Removing a declaration
requires `upgrade --prune-stale` to remove an unchanged old target. Destination moves require
explicitly handling the old installation first. Interrupted writes retain protected recovery state;
review `shine app recover` before retrying. Do not delete Shine's private secret state or fingerprint
key, which are required for ownership checks and recovery.

With automatic decryption disabled, use `shine env workspace export --format files` from the
installed workspace, selecting its mode and file group. Manual export refuses existing files,
does not create App ownership, and uses its own private permission defaults. Explicitly set service
read permissions before starting a container. A later automatic install can adopt identical content.

Containers still need runtime plaintext. Configuration installation does not start or restart them;
explicitly recreate containers whose file bind mounts must pick up a replaced password file, and
stop dependent services before uninstalling their required configuration.

Upgrade both SSH peers before using these presets. Older clients reject App metadata v3 and the
new App manifest format rather than treating ciphertext as an ordinary password file.
