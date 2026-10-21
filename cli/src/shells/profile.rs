//! CLI path and activation-hint adapters for Core-owned Shell profiles.

use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::output;

use super::ShellType;

pub(super) fn managed_shell_profile_path(config: &Config) -> PathBuf {
    shine_core::runtime::managed_shell_profile_path(config.shine_dir(), config.shell_type)
}

pub(super) fn supports_completion_registration(shell: &ShellType) -> bool {
    shine_core::runtime::supports_completion_registration(*shell)
}

pub(super) fn source_activation_hint(shell: &ShellType, path: &Path) -> String {
    if path
        .to_str()
        .is_none_or(|value| value.chars().any(char::is_control))
    {
        return "open a new shell to load the updated profile".into();
    }
    format!(
        "run `{}` once, or open a new shell",
        shine_core::runtime::shell_source_command(*shell, path)
    )
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
            &source_activation_hint(&config.shell_type, shell_config_path),
        ),
        output::hint_line_text(
            "Commands",
            &format!(
                "available after reload: {}",
                output::escape_controls(&source_commands.join(", "))
            ),
        ),
    ]
}
