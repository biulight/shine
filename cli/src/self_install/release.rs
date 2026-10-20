//! Release installation and best-effort synchronization of the configured binary copy.

use super::binary::sync_self_install_dest;
use crate::config::Config;
use crate::update_check::ReleaseChannel;
use crate::{colors, update_check, version};
use anyhow::Result;

pub async fn handle_self_upgrade(config: &Config, channel: Option<ReleaseChannel>) -> Result<()> {
    let current = version::semver();
    let selected_channel = channel.unwrap_or(ReleaseChannel::Stable);
    let force_install = channel.is_some();
    println!(
        "Checking for {} upgrades (current: {current})...",
        selected_channel.as_str()
    );

    match update_check::upgrade_to_release(config, selected_channel, force_install).await {
        Ok(update_check::UpgradeResult::AlreadyUpToDate { channel, latest }) => {
            println!(
                "{}",
                colors::green(&format!(
                    "shine {current} is up to date on the {} channel ({latest}).",
                    channel.as_str()
                ))
            );
        }
        Ok(update_check::UpgradeResult::Upgraded {
            channel,
            previous: _,
            previous_display,
            release_tag,
            installed_version,
            installed_path,
        }) => {
            println!(
                "{}",
                colors::green(&format_self_upgrade_message(
                    channel,
                    &previous_display,
                    &installed_version,
                    &release_tag,
                ))
            );
            sync_self_install_dest(config, &installed_path).await;
        }
        Err(e) => {
            update_check::invalidate_update_cache(config).await;
            return Err(e.context("Upgrade failed"));
        }
    }

    Ok(())
}

fn format_self_upgrade_message(
    channel: ReleaseChannel,
    previous_display: &str,
    installed_version: &str,
    release_tag: &str,
) -> String {
    match channel {
        ReleaseChannel::Stable => {
            format!("Installed shine {installed_version} over {previous_display}.")
        }
        ReleaseChannel::Preview => {
            if previous_display.contains("-preview") {
                format!(
                    "Updated shine preview from {previous_display} to {installed_version} ({release_tag})."
                )
            } else {
                format!(
                    "Installed shine preview {installed_version} over stable {previous_display} ({release_tag})."
                )
            }
        }
    }
}

#[cfg(test)]
mod tests;
