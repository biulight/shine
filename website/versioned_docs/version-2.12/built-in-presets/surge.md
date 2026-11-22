---
title: Surge
sidebar_position: 2
---

# Surge

With the Shine preset, you can keep receiving updates from your provider while adding your own
proxies, policy groups, and routing rules. You can also bring nodes from multiple subscriptions
together in one Surge policy group.

Install Bun and use Surge Mac with [CLI module support](https://manual.nssurge.com/tools/cli.html).
Keep Surge running while you set up or remove the integration.

## Prepare the profile to customize

1. In Surge's **Profile** settings, import the provider's Surge configuration from its URL if you
   haven't already.
2. Create a [Linked Profile](https://manual.nssurge.com/profile/format.html) from that
   configuration and save it in Surge's local Profiles folder. This is the profile Shine will
   modify. Make sure `[Proxy]`, `[Proxy Group]`, and `[Rule]` each have a `#!include` line that
   references the provider's configuration. Shine adds its local files to these lines.
3. Note the new profile's `.conf` filename. Use **Reveal in Finder** if you need to confirm it.

## Add subscription sources (optional) {#surge-uri-subscriptions}

For extra nodes, set the variable for the source you want before installing.

| Source | Shine variable | Surge policy group |
| --- | --- | --- |
| Provider's HTTPS Base64 URI subscription | `SURGE_SUBSCRIPTION_URL` | `Subscription` |
| wGetCloud's HTTPS Base64 URI subscription | `W_GET_CLOUD_SURGE_SUBSCRIPTION_URL` | `WgetCloud` |
| Surge's downloaded wGetCloud cache | `W_GET_CLOUD_SURGE_CACHE_SOURCE` | `WgetCloud` |

To set a provider's URI subscription URL, use the following command.

```bash
shine env set SURGE_SUBSCRIPTION_URL 'https://provider.example/subscription?...'
```

Use your provider's URI/Base64 link for either URL variable. Shine imports compatible `ss://`,
`vmess://`, and `trojan://` nodes. Trojan supports TCP and WebSocket.

### Use the wGetCloud cache offline

In Surge's Profile settings, locate or create a separate local Linked Profile that directly
references the wGetCloud Surge configuration URL with `#!include`. The `[Proxy]` and
`[Proxy Group]` sections together must reference exactly one URL. Load the profile in Surge so
it downloads the subscription. Set `W_GET_CLOUD_SURGE_CACHE_SOURCE` to this **source profile's
filename** in the local Profiles folder, for example `WgetCloud.conf`.

```bash
shine env set W_GET_CLOUD_SURGE_CACHE_SOURCE WgetCloud.conf
```

You can also give the absolute path to the downloaded cache file. Shine imports the nodes
defined directly in the cache's `[Proxy]` section. Leave `W_GET_CLOUD_SURGE_SUBSCRIPTION_URL`
unset for offline use. When both inputs are set, Shine uses the URL.

## Add your own settings (optional)

Follow the [overlay setup](../guides/custom-presets.md#override-selected-files-with-an-overlay)
for `app/surge` and keep the copies you want to customize. Edit the files in the overlay.

- `local-proxies.conf` adds your own [proxies](https://manual.nssurge.com/policies/overview.html).
- `local-proxy-groups.conf` defines your own [policy groups](https://manual.nssurge.com/policy-groups/policy-including.html).
  To combine nodes from the two subscription groups in one group, use
  `include-other-group="Subscription,WgetCloud"` in that group once both sources are set up.
- `local-rules.conf` adds [routing rules](https://manual.nssurge.com/rules/overview.html) that
  Surge checks before the provider's rules.
- `shine-general.sgmodule` adds [General settings through a Surge module](https://manual.nssurge.com/profile/module.html).
  Keep the filename and `#!name=shine-general`. The bundled module contains commented examples.
  Enabling it applies its settings device-wide, including when you switch profiles.

## Install and apply

Replace `MySurge.conf` with the filename of the **Linked Profile you want Shine to modify**.
The built-in preset installs its files in the local Profiles folder shown below. Shine finds the
files to connect beside the target profile. Make sure any configured subscription URLs are
reachable, then review the install and apply plans before approving them.

```bash
shine env set SURGE_PROFILE "~/Library/Application Support/Surge/Profiles/MySurge.conf"
shine app install surge --dry-run
shine app install surge
shine app artifact apply surge
```

Apply connects the local files, enables `shine-general`, and reloads Surge. Select your target
profile in Surge and check that the groups you configured appear. Use Rule-Based mode to test
your routing rules. For approval and updates, see [Manage application configuration](../guides/app-presets.md).

If apply reports a missing `#!include`, check the corresponding section of the target profile.
If the installed module differs from the preset, review your edits and install or upgrade before
trying again.

## Refresh subscriptions

Use the command for the source you configured. Routine `update` and `upgrade` skip these subscriptions.

```bash
shine app refresh surge subscription-proxies.conf
shine app refresh surge sub/wget-cloud-proxies.conf
```

Run `shine app install surge` before the first refresh of a source you add later. A successful
refresh reloads Surge when the content changes. A failed refresh keeps the last successful file.
Refresh preserves local edits unless you opt in to replacing them with `--force`.

- For a failed download or no compatible nodes, check access to the subscription URL and its
  [format](#surge-uri-subscriptions).
- For an offline cache error, check the source profile's filename and URL reference, then confirm
  Surge has downloaded the subscription. If you supplied an absolute path, verify it points to
  the cache file.

## Remove the integration

Keep `SURGE_PROFILE` pointed at the profile you applied the integration to. Run the following
with Surge open.

```bash
shine app artifact remove surge
```

This removes the Shine references from the profile, disables `shine-general`, and reloads Surge.
Keep the module file in place until removal succeeds so Surge can clear its enabled state. If
you already deleted the module, restore the file and run remove again. To delete the installed
files as well, see [App uninstall](../guides/app-presets.md#uninstall-and-restore).

If apply or remove fails or is interrupted, some changes may already be in effect. Fix the
reported issue and retry the same command. If Shine reports broken legacy proxy-group markers,
repair them in the target profile before retrying.
