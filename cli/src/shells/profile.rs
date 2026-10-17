//! CLI adapter for Core-owned Shell profile rendering and execution.

#[cfg(test)]
use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::output;
#[cfg(test)]
use crate::path_display;

use super::ShellType;
#[cfg(test)]
use super::{ShellConfigUpdate, get_shell_config_paths};

pub(super) fn managed_shell_profile_path(config: &Config) -> PathBuf {
    shine_core::runtime::managed_shell_profile_path(config.shine_dir(), config.shell_type)
}

pub(super) fn supports_completion_registration(shell: &ShellType) -> bool {
    shine_core::runtime::supports_completion_registration(*shell)
}

pub(super) fn shell_source_command(shell: &ShellType, path: &Path) -> String {
    shine_core::runtime::shell_source_command(*shell, path)
}

pub(super) fn source_command_activation_hint_lines(
    config: &Config,
    shell_config_path: &Path,
    source_commands: &[String],
) -> Vec<String> {
    if source_commands.is_empty() {
        return Vec::new();
    }
    vec![
        output::hint_line_text(
            "Next Step",
            &format!(
                "run `{}` once, or open a new shell",
                shell_source_command(&config.shell_type, shell_config_path)
            ),
        ),
        output::hint_line_text(
            "Commands",
            &format!("available after reload: {}", source_commands.join(", ")),
        ),
    ]
}

#[cfg(test)]
pub(super) async fn append_path_to_shell_config(
    config: &Config,
    force: bool,
    source_commands: &[String],
) -> Result<ShellConfigUpdate> {
    let paths = get_shell_config_paths(&config.shell_type, &config.home_dir)?;
    crate::core_runtime::from_config(config)
        .await?
        .install_shell_profile(&paths, force, source_commands)
        .await
}

#[cfg(test)]
pub(super) async fn remove_path_from_shell_config(config: &Config) -> Result<()> {
    let paths = get_shell_config_paths(&config.shell_type, &config.home_dir)?;
    let removed = crate::core_runtime::from_config(config)
        .await?
        .remove_shell_config_blocks(&paths)
        .await?;
    for path in removed {
        println!(
            "Shell config ({}): shine entry removed",
            path_display::format_home(&path, &config.home_dir)
        );
    }
    Ok(())
}
