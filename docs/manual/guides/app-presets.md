---
title: Manage application configuration
sidebar_position: 2
---

# Manage application configuration

Application presets install configuration where the application expects it. Existing unmanaged
configuration files are backed up as `*.shine.bak` before replacement.
[Secret files](#managed-secret-files) have stricter conflict rules and never create plaintext backups.

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

Shine shows a plan before installing, upgrading, removing, refreshing, or applying an artifact.
Review it before confirming; the default is No. Automation can use `--yes` after review, but the
option does not bypass permission, trust, or safety checks.

Install once, then check and apply updates when needed:

```bash
shine app install starship
```

The equivalent target form is `shine install app/starship`.

```bash
shine update app/starship
shine upgrade app/starship
```

`shine update` reports differences without applying them. `shine upgrade` applies available
configuration updates; these examples affect only Starship.

If installation fails, some files may already be installed. Check the reported results before retrying.

Reinstalling a category preserves files changed since installation, including managed JSON keys.
To replace your edits with the preset content, preview and then approve the replacement:

```bash
shine app install starship --replace-managed --dry-run
shine app install starship --replace-managed
```

If another App operation is running, wait for it to finish. Review a new Plan if Shine reports
that the relevant files or settings have changed.

### Migrating legacy App metadata to Shine 2

If Shine reports incompatible metadata in an external preset or overlay, preview migration with
`shine preset migrate --dry-run`, then review and apply it with `shine preset migrate`. Migration
backs up metadata and preserves customized configuration files. Resolve any items reported as
requiring manual changes before installing; granting code trust cannot fix incompatible metadata.
See [migrate a 1.x source](./custom-presets.md#migrate-a-1x-source) for validation and authoring steps.

## Uninstall and restore

| Operation | Command |
| --- | --- |
| Preview removal | `shine app uninstall starship --dry-run` |
| Apply only after reviewing the preview | `shine app uninstall starship` |

By default, Shine preserves files modified after installation and reports a conflict. Safe uninstall
restores the original backup when one was created. A destination move also requires the old managed
file to be unchanged and the new path to be free. If either location has changed, Shine leaves both
untouched for review.

If a generated file moves to another destination, check the old file and its backup before approving
the move. Missing or changed backups can require manual repair; generated-file moves are not covered
by `shine app recover`.

`app uninstall --force` permits deleting user-modified managed content; preview it with `--dry-run`.
Uninstall uses the strategy recorded at installation, even if the preset now declares a different one.
Uninstall also deletes the selected category's entire Shine cache, including any extra files there.
Without a category, it clears all App caches. External preset sources are preserved. Keep personal
files outside these caches and review the removal Plan before approval.

`shine uninstall app/<category>` uses the same category-limited behavior. The old `--purge` option
has been removed. Resolve protected or failed removals before retrying; do not delete installation
records or backups manually.

During upgrade, obsolete managed entries are removed only when `--prune-stale` is part of the approved command;
modified entries remain preserved.

## Upgrade and downgrade {#upgrade-and-downgrade}

Shine 2.8 and later save App installation records that Shine 2.7.x and earlier cannot read.
Before downgrading:

1. Use the newer binary to preview and complete App uninstall. Resolve protected files and pending
   recovery first. Back up any configuration you want to keep.
2. If uninstall was interrupted, rerun the same uninstall command to finish cleanup.
3. Replace the binary, then reinstall using presets supported by the older version.

Installing the old binary, resealing encrypted sources, or using `--replace-managed` cannot convert
installation records. Do not delete records or edit their version to bypass the compatibility check.

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

Recovery may require administrator access. Recovery files can contain configuration data, so
treat them as sensitive. Do not edit or delete recovery state manually; if recovery reports a conflict, inspect the named paths and keep a backup
before resolving it.

## Configuration transforms

Some presets process source files before installation:

- `jsonc-to-json` removes JSONC comments and trailing commas and writes standard JSON.
- `template` replaces `@@VAR_NAME@@` with the current `[env]` value.
- The `json-merge` strategy manages declared top-level keys while preserving other user settings.

`shine update` compares the final transformed result, not the original preset file.

## Generated files and Surge URI subscriptions

Some presets generate configuration from other inputs, such as subscription URLs. Ordinary
`list`, `info`, and `update` do not run generators. A `generator not evaluated` warning means Shine
has not checked the generated content; it does not mean the installed file is current.

To check generated content without installing it, choose a scope below. These commands run code
and may access the network. External or overlay generators require an existing
[trust grant](#lifecycle-hooks).

| Operation | Command |
| --- | --- |
| Evaluate generators and inspect status | `shine app info surge --run-generators` |
| Evaluate and inspect differences | `shine info app/surge --run-generators --diff` |
| Check this App with generator evaluation | `shine update app/surge --run-generators --diff` |
| Evaluate all installed App categories | `shine update --run-generators` |

Evaluation runs both automatic and manual generators in the selected scope. It does not install
files or run hooks or artifacts. Failed or incomplete evaluation is reported with a nonzero exit
status; resolve those errors before treating the result as a complete check. If Shine blocks a
script because its code does not match its declarations, follow
[App code checks](./custom-presets.md#check-app-code-before-execution); trust does not bypass that check.

To save newly generated content, refresh the installed files:

| Operation | Command |
| --- | --- |
| Refresh all installed generated files in a category | `shine app refresh <CATEGORY>` |
| Refresh one file | `shine app refresh <CATEGORY> <SOURCE_FILE>` |

`SOURCE_FILE` is the relative source path listed by the preset. Newly added generated files must
be installed with `shine app install <CATEGORY>` before they can be refreshed. Refresh shows a plan
for confirmation; automation requires `--yes`. Failure preserves the last successful content.
Local edits are preserved unless you explicitly add `--force`.

Automatic generators can also run during install and upgrade. Manual generators (`auto = false`)
run during installation, explicit evaluation, or refresh, but not routine upgrade. Reinstalling with
`--replace-managed` can run them too. Only generators enabled by the preset's conditions run.

Permission declarations do not sandbox code. Review external scripts before approving them; see
[external-code authorization](#lifecycle-hooks) for one-operation approval and persistent trust.

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

When refreshed content changes, Surge is reloaded. Failure preserves the
last successful file. The `Subscription` group in `local-proxy-groups.conf` reads nodes through
`policy-path=subscription-proxies.conf`; other groups can include them with
`include-other-group=Subscription`.

## Build helper resources

Some presets need a separate step to connect installed files to the application. Run it explicitly
when prompted; for example:

```bash
shine app artifact apply surge
```

Each apply or remove command shows a plan for confirmation and reports failure if its script fails.
Use `--yes` for automation only after reviewing the operation. These commands can change application
settings, so follow the preset-specific instructions below. For custom scripts, see
[Tasks and application artifacts](./tasks-and-serve.md).

The built-in `surge` preset installs `local-proxies.conf`, `local-proxy-groups.conf`,
`local-rules.conf`, `shine-general.sgmodule`, and the optional subscription file in the Surge Profiles directory. After setting
`SURGE_PROFILE` in `[env]`, `shine app artifact apply surge` connects those files to the selected Profile, enables the
`shine-general` module, and reloads Surge.
An overlay supplies only its policy files and does not need its own build script.

For local `[General]` settings, create `app/surge/shine-general.sgmodule` in your active overlay:

```ini
#!name=shine-general
#!desc=Local General settings managed by Shine.
#!system=mac

[General]
skip-proxy = %APPEND% *.internal
```

Keep the filename, `#!name`, and native module destination
`~/Library/Application Support/Surge/Profiles/` unchanged. The built-in module contains only
commented examples. Edit the overlay source, not its installed copy, then run:

```bash
shine app install surge
shine env set SURGE_PROFILE "/absolute/path/to/Profile.conf"
shine app artifact apply surge
```

Apply and remove require Bun and a running Surge Mac with CLI module support. Apply checks that
the installed module matches the reviewed preset before enabling it. If they differ, review your
edits and install or upgrade the preset first. Install and upgrade deploy files and reload Surge;
they do not issue a module-enable command. A new module starts disabled, but Surge remembers a
previously enabled module across deletion and reinstallation.

Modules change the effective configuration without rewriting `[General]` in the original Profile.
Their enable state is device-wide, not limited to the path in `SURGE_PROFILE`. Add more values on
the same line, another property line, or use `key = value` to override a scalar setting. `%APPEND%`
does not deduplicate existing values; repeated enable/reload does not accumulate additional copies.
An already enabled local module picks up installed changes on reload. `skip-proxy` configures system
proxy bypass, not a `DIRECT` routing rule; Enhanced Mode can still handle that traffic.

If a legacy Shine proxy-group block has a missing or nested marker, both applying and undoing the
patch fail without changing the profile. Remove may already have disabled the module. Repair the
malformed markers in your profile before retrying.

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
Surge teardown disables `shine-general` and reloads before undoing includes; `artifact remove`
keeps the installed files, while uninstall removes managed files afterward. Do not delete the module
file first: deletion alone can leave its effect active until reload and does not clear remembered
enable state. If already deleted, restore the module file and run remove again to clear that state.
CLI failures fail the artifact command. An interrupted apply may have updated includes, or an
interrupted remove may have disabled the module; fix the reported issue and repeat the command to
finish. Other modules are not changed.

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

Shine updates the four editor configurations without changing the remote subscription or creating
editor bindings. After it writes new content, reselect the subscription in Clash Verge Rev, then run
the apply command again if you want an immediate rule-provider refresh.

Choose one complete rule-provider layout in `merge.yaml`: local files inside mihomo's `HomeDir`,
loopback HTTP on the same device, or remote HTTPS. Enable the matching policy groups and
`prepend-rules`. Only the local-file layout uses the reference lists installed under
`HomeDir/ruleset/shine-source/`; customize them through your overlay when using that layout.
`proxy: DIRECT` affects provider downloads only; change it if the server requires a proxy.
Private domains that rely on system split DNS also need mihomo `dns.nameserver-policy` configuration.

Bun must be installed. When `shine upgrade` changes this category, its approved script automatically
updates the integration. If it changes the editor configurations, reselect the subscription and run
`shine app artifact apply clash-verge` again. Use that command for first-time setup and failed-refresh
retries too.

To request an immediate refresh, configure `CLASH_CONTROLLER_URL` and `CLASH_CONTROLLER_TOKEN` in
Shine's environment settings. Without the URL, providers still update on their own intervals.
The refresh includes all rule providers declared in `merge.yaml`; after they all refresh successfully,
it closes active mihomo connections so applications reconnect under the new rules. This may briefly
interrupt downloads and other long-lived sessions. Keep controller tokens out of overlays and
documentation.

`shine app artifact remove clash-verge` does not clear subscription bindings stored by Clash Verge
Rev. Clear the four editors manually when removing the integration completely.

## Lifecycle hooks

Some presets run a follow-up action after installation or upgrade, such as reloading the application.
These hooks run only when the operation writes files; unchanged categories do not trigger them.
Review any requested environment inputs and administrator access before confirming. Hook failures
are reported without stopping installation or upgrades for other categories. Add `--verbose` to
`shine upgrade` when you need successful hook output that the preset makes available.

External hooks, generators, and artifacts need code authorization. Interactive confirmation can
authorize the listed code for that operation only, without saving trust. For automation or explicit
`--run-generators` checks, inspect and grant persistent trust first:

| Operation | Command |
| --- | --- |
| Inspect the code and requested capabilities | `shine trust inspect app/<CATEGORY>` |
| Trust the reviewed category | `shine trust grant app/<CATEGORY>` |

A grant covers all listed code in the category, including its helpers. Changes to that category may
require a new review. `--development` also trusts future code changes from the displayed local source;
use it only if you intend that wider authorization. Missing inputs or incompatible metadata must still
be resolved, and permission declarations do not sandbox the code.

For hook declarations and environment inputs, see
[application artifact runtimes](./custom-presets.md#application-artifact-runtimes).

## Managed secret files {#managed-secret-files}

Use an App preset to install encrypted workspace files and manage the decrypted files needed by a
service. Keep plaintext out of the preset repository and decryption identities on the decrypting
machine. First prepare the workspace file groups and encryption policy described in
[Manage environment variables](./environment.md), then declare the selected group in the App preset:

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
as an account allowed to create the destination and set that ownership. When assigning a different
non-root owner, pre-create all parent directories with Unix search (`x`) permission for that UID
or the selected GID (or others). Shine checks the applicable owner/group/other mode bits before
decryption; supplementary groups and ACL-only grants are not used to satisfy this check. Existing
directory permissions are not changed automatically. Windows uses private
current-user ACLs and rejects Unix permission fields. Review permissions before installing.

An existing unowned target is adopted only when its content exactly matches the decrypted file.
Different content is a conflict, including with `--replace-managed`; no plaintext `.shine.bak`
is created. Later local edits are preserved unless explicitly replaced. Unchanged files skip
decryption. Read-only commands and diffs show status without decrypting or displaying content.

Uninstall removes unchanged managed secret files and preserves local edits. Turning automatic
decryption off pauses creation and updates but retains existing ownership. Removing a declaration
requires `upgrade --prune-stale` to remove an unchanged old target. Destination moves require
explicitly handling the old installation first. Interrupted writes retain protected recovery state;
review `shine app recover` before retrying. Do not delete Shine's private secret state or keys. If recovery cannot safely identify a file, it
retains that file for manual inspection even when its content appears unchanged.

With automatic decryption disabled, use `shine env workspace export --format files` from the
installed workspace, selecting its mode and file group. Manual export refuses existing files,
does not create App ownership, and uses its own private permission defaults. Explicitly set service
read permissions before starting a container. A later automatic install can adopt identical content.

Containers still need runtime plaintext. Configuration installation does not start or restart them;
explicitly recreate containers whose file bind mounts must pick up a replaced password file, and
stop dependent services before uninstalling their required configuration.

Presets using `secret_files` require `metadata_schema_version = 3`. Upgrade both SSH peers before
using these presets remotely. See [upgrade and downgrade](#upgrade-and-downgrade) before switching
to an older Shine version.
