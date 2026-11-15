---
title: Manage application configuration
sidebar_position: 2
---

# Manage application configuration

Use App presets to install and update application configuration. The
[built-in App presets](../built-in-presets/index.md#application-presets) require you to install the
applications separately.

## Browse and preview

```bash
shine app list
shine app info starship
shine app install starship --dry-run
```

Preview the destinations and scope before installing, especially for system directories that need
administrator access. `--dry-run` does not write application files.

## Install and update

```bash
shine app install starship
shine update app/starship
shine upgrade app/starship
```

`update` checks for differences; `upgrade` applies configuration updates. These commands affect only
Starship. The equivalent install form is `shine install app/starship`.

Before making changes, Shine shows a plan and asks for confirmation, defaulting to No. Review the
files, required inputs, and permissions. Automation can use `--yes` after review; it cannot bypass
permission, trust, or safety checks. Presets that run external code also need
[code authorization](#lifecycle-hooks).

Existing unmanaged configuration files are backed up as `*.shine.bak` before replacement.
[Secret files](#managed-secret-files) have stricter conflict rules and never create plaintext backups.
Reinstallation preserves your later edits, including managed JSON keys. To replace those edits with
the preset content, preview and then approve:

```bash
shine app install starship --replace-managed --dry-run
shine app install starship --replace-managed
```

To remove unchanged files no longer declared by the preset, add `--prune-stale` to the upgrade
command you review. Modified files remain preserved.

### Understand configuration changes {#configuration-transforms}

Some presets convert JSONC or supported JSON5 to JSON, fill `@@VAR_NAME@@` from `[env]`, or manage
only selected JSON keys. Updates compare the installed result with the processed configuration;
settings outside managed JSON keys are preserved.

### Refresh generated configuration {#generated-files-and-surge-uri-subscriptions}

Some presets generate files from inputs such as subscription URLs. Ordinary `list`, `info`, and
`update` do not run generators. `generator not evaluated` means the content has not been checked,
not that the installed file is current.

To evaluate generated content without installing it:

```bash
shine update app/surge --run-generators --diff
```

This runs both automatic and manual generators for the selected App and may access the network.
External or overlay generators need an existing [trust grant](#lifecycle-hooks). Evaluation does not
install files or run application follow-up actions. Resolve failed or incomplete checks before
relying on the result. Use `shine update --run-generators` to check all installed App categories.

To install fresh generated content:

```bash
shine app refresh <CATEGORY>
shine app refresh <CATEGORY> <SOURCE_FILE>
```

Choose the category-wide or single-file form. `SOURCE_FILE` is the relative source path listed by
the preset. Newly added files need `shine app install <CATEGORY>` before they can be refreshed.
Failure preserves the last successful content; local edits are preserved unless you add `--force`.

Automatic generators can also run during install and upgrade. Manual generators run during
installation, explicit evaluation, or refresh, but not routine upgrade; reinstalling with
`--replace-managed` can run them too. Only generators enabled by the preset's conditions run.

### Connect configuration to the application {#build-helper-resources}

If the preset asks for an additional setup step, run:

```bash
shine app artifact apply <CATEGORY>
```

This can change application settings. Follow the preset's instructions and review the plan. Some
presets also reload the application automatically after installation or upgrade changes files;
unchanged categories do not trigger these actions. Check any reported failures. A failed follow-up
action does not stop installation or upgrades for other categories; use `shine upgrade --verbose`
when you need successful output that the preset makes available.

To write these scripts, see [Custom presets](./custom-presets.md#application-artifact-runtimes).

## Authorize external code {#lifecycle-hooks}

External hooks, generators, and artifact scripts run without sandbox isolation. Review the code
before approving it. Interactive confirmation can authorize the listed code for that operation
only, without saving trust. Automation and explicit `--run-generators` checks need persistent trust:

```bash
shine trust inspect app/<CATEGORY>
shine trust grant app/<CATEGORY>
```

A grant covers all listed code in the category, including helpers; category changes may require a
new review. `--development` also trusts future code changes from the displayed local source. Use it
only if you intend that wider authorization. Trust does not fix missing inputs, incompatible
metadata, or blocked [App code checks](./custom-presets.md#check-app-code-before-execution).

## Uninstall and restore

```bash
shine app uninstall starship --dry-run
shine app uninstall starship
```

Uninstall preserves files modified after installation and reports a conflict. It restores the
original backup when one was created. `--force` permits deleting modified managed content; preview
that removal with `--dry-run` first.

Uninstall also deletes the category's entire Shine cache, including extra files stored there.
Without a category, it clears all App caches. Keep personal files outside these caches; external
preset sources are preserved. `shine uninstall app/<CATEGORY>` has the same category-limited scope.

Application cleanup may fail while managed files are still removed. Inspect warnings and the final
result, and follow the application's removal instructions. Where a preset provides cleanup, run
`shine app artifact remove <CATEGORY>` separately if needed. Its effect depends on the preset;
check which settings it removes and which steps you must complete manually.
If apply or remove fails or is interrupted, some settings may already have changed: fix the reported
problem and repeat the command. Do not delete installation records or backups to bypass a failure.

## Resolve conflicts and interruptions

If another App operation is running, wait for it to finish. If Shine reports that files or settings
changed after review, review a new plan. A failed installation can leave some files installed;
check the reported results before retrying.

When an update moves a managed file, the old file must be unchanged and the new path free. Otherwise
Shine leaves both locations untouched. For a generated-file move, also check the old backup before
approval: missing or changed backups may need manual repair, and these moves are not covered by
`shine app recover`.

### Recover an interrupted App operation

An interrupted operation may block further changes; read-only commands remain available. Run:

```bash
shine app recover
```

Review the recovery plan and any administrator access it requests. Recovery completes or rolls back
the operation only when the affected files still match the recorded state. It preserves later
changes, including unrelated JSON settings, and stops on conflicts. Back up and inspect the named
paths before resolving them. Recovery files can contain sensitive configuration; do not edit or
delete recovery state manually. Automation can use `shine app recover --yes` after reviewing the plan.

### Use an older Shine version {#upgrade-and-downgrade}

Shine 2.8 and later save App installation records that 2.7.x and earlier cannot read. Before
switching to an older version:

1. Back up configuration you want to keep. Use the newer binary to resolve protected files and
   pending recovery, then preview and complete App uninstall.
2. If uninstall was interrupted, rerun the same uninstall command to finish cleanup.
3. Replace the binary and reinstall using presets supported by the older version.

Installing an old binary, resealing encrypted sources, or using `--replace-managed` cannot convert
the records. Do not delete them or edit their version to bypass the check.

### Migrate an older preset {#migrating-legacy-app-metadata-to-shine-2}

If Shine reports incompatible metadata in an external preset or overlay, preview with
`shine preset migrate --dry-run`, then review and run `shine preset migrate`. Migration backs up
metadata and preserves customized configuration files. Resolve items needing manual changes before
installing; see [migrate a 1.x source](./custom-presets.md#migrate-a-1x-source).

## Manage secret files {#managed-secret-files}

App presets can install encrypted workspace files and manage the decrypted files a service needs.
Keep plaintext out of the preset repository and decryption identities on the decrypting machine.
To author a preset, follow [Declare managed secret files](./custom-presets.md#app-secret-files).

With `auto_decrypt` enabled, install and upgrade decrypt required files after approval and before
changing the category's destination files; unchanged files skip decryption. Read-only commands and
diffs show status without decrypting or displaying content. In an enabled SSH broker session,
decryption and hardware authorization happen on your local computer; otherwise the installing
machine uses its configured backend. Broker failure does not fall back to another backend.
Upgrade both SSH peers before using these presets; see [SSH file authorization](ssh-transfer.md#file-secret-broker).

### Prepare service access

Unix defaults are the installing user's ownership and `0600`. To use a different service account,
use the preset's optional `unix_uid`, `unix_gid`, and `unix_mode` (`0400` or `0600`). Install with an
account allowed to create the target and set that ownership. For a different non-root owner, pre-create all
parent directories with search (`x`) permission for that UID, selected GID, or others. Supplementary
groups or ACLs alone are insufficient; Shine does not change existing directory permissions.
On Windows, secret files are private to the current user; omit Unix fields. Review permissions
before installing.

### Handle existing files and removal

- An unmanaged target is adopted only if its content matches the decrypted file. Different content
  is a conflict even with `--replace-managed`; no plaintext `.shine.bak` is created.
- Later local edits are preserved unless explicitly replaced. Uninstall removes unchanged secret
  files and preserves local edits. Turning off automatic decryption pauses creation and updates but
  leaves existing files managed. Removing a declaration requires `upgrade --prune-stale` to remove
  an unchanged old target; destination moves require handling the old installation first.
- After interrupted writes, review and run `shine app recover`. Do not delete Shine's private
  secret state or keys. Recovery may require manual inspection even if file contents look unchanged.

With automatic decryption off, you can use `shine env workspace export --format files` from the
installed workspace, explicitly selecting the mode and file group. Manual export refuses existing
files, uses private default permissions, and does not make them App-managed. Set service read
permissions yourself; later automatic installation can adopt identical files.

Containers still need plaintext at runtime. Shine does not start or restart them: recreate containers
whose file mounts need to pick up replaced password files, and stop dependent services before
uninstalling required configuration.

## Choose a built-in preset {#application-specific-setup}

<span id="surge-uri-subscriptions" />
<span id="clash-verge-rev" />

Browse [built-in application presets](../built-in-presets/index.md#application-presets) for optional
configurations and their prerequisites, customization, and removal steps.
