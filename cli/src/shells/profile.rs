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
    utils::runtime::managed_shell_profile_path(config.shine_dir(), config.shell_type)
}

#[cfg(test)]
pub(super) fn managed_profile_snippet(
    shell: &ShellType,
    bin_dir: &Path,
    home_dir: &Path,
    source_commands: &[String],
) -> String {
    utils::runtime::managed_profile_snippet(*shell, bin_dir, home_dir, source_commands)
}

pub(super) fn supports_completion_registration(shell: &ShellType) -> bool {
    utils::runtime::supports_completion_registration(*shell)
}

#[cfg(test)]
pub(super) fn shell_config_snippet(
    shell: &ShellType,
    profile_path: &Path,
    home_dir: &Path,
) -> String {
    utils::runtime::shell_config_snippet(*shell, profile_path, home_dir)
}

#[cfg(test)]
pub(super) fn powershell_bin_assignment(bin: &str) -> String {
    utils::runtime::powershell_bin_assignment(bin)
}

pub(super) fn shell_source_command(shell: &ShellType, path: &Path) -> String {
    utils::runtime::shell_source_command(*shell, path)
}

#[cfg(test)]
pub(super) fn powershell_quote(path: &Path) -> String {
    utils::runtime::powershell_quote(path)
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
pub(super) fn remove_sentinel_block(content: &str) -> String {
    utils::runtime::remove_shell_sentinel(content)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_sentinel_block_preserves_existing_bytewise_contract() {
        assert_eq!(
            remove_sentinel_block("keep\n\n# >>> shine >>>\nsource x\n# <<< shine <<<\nafter\n"),
            "keep\nafter\n"
        );
    }
}
