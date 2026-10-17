pub(crate) mod deployment;
mod install;
mod links;
#[cfg(all(test, unix))]
mod live_bun_bench;
pub mod metadata;
mod profile;
mod recovery;
mod report;
mod uninstall;

#[doc(hidden)]
pub use deployment::{handle_launch_live, handle_render_live};
pub(crate) use install::collect_update_lifecycle_result;
pub use install::{
    handle_completion_install, handle_init_template, handle_install_approved,
    handle_install_dry_run,
};
pub(crate) use install::{
    handle_upgrade_installed_target_with_result_approved,
    handle_upgrade_installed_with_result_prepared,
};
pub use recovery::handle_recover_approved;
#[doc(hidden)]
pub use report::handle_list_with_presets_note;
pub use report::{ShellUpgradeReport, handle_info, handle_list};
pub use uninstall::handle_uninstall_approved;

use anyhow::{Result, bail};
pub use shine_core::runtime::ShellType;
use std::path::{Path, PathBuf};

pub const SENTINEL_START: &str = "# >>> shine >>>";

use shine_core::runtime::PathUpdateStatus;
#[cfg(test)]
use shine_core::runtime::ShellConfigUpdate;

pub fn get_shell() -> Result<ShellType> {
    match std::env::var("SHELL") {
        Ok(shell) => shell.parse(),
        Err(_) if cfg!(windows) => Ok(ShellType::PowerShell),
        Err(_) => bail!("Could not find $SHELL"),
    }
}

pub fn get_shell_config_path(shell_type: &ShellType, home_path: &Path) -> Result<PathBuf> {
    get_shell_config_paths(shell_type, home_path)?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("shell config paths should never be empty"))
}

fn get_shell_config_paths(shell_type: &ShellType, home_path: &Path) -> Result<Vec<PathBuf>> {
    match shell_type {
        ShellType::Bash => Ok(vec![home_path.join(".bashrc")]),
        ShellType::Fish => Ok(vec![home_path.join(".config/fish/config.fish")]),
        ShellType::Zsh => Ok(vec![home_path.join(".zshrc")]),
        ShellType::PowerShell => {
            if cfg!(windows) {
                Ok(vec![
                    home_path.join("Documents/PowerShell/Microsoft.PowerShell_profile.ps1"),
                    home_path.join("Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1"),
                ])
            } else {
                Ok(vec![home_path.join(
                    ".config/powershell/Microsoft.PowerShell_profile.ps1",
                )])
            }
        }
        ShellType::Elvish => Ok(vec![home_path.join(".config/elvish/rc.elv")]),
    }
}

pub(crate) fn shell_config_paths_for_core(
    shell_type: &ShellType,
    home_path: &Path,
) -> Result<Vec<PathBuf>> {
    get_shell_config_paths(shell_type, home_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use profile::shell_source_command;

    #[test]
    fn source_activation_command_quotes_shell_config_path() {
        let path = PathBuf::from("/home/user/my config/.zshrc");
        assert_eq!(
            shell_source_command(&ShellType::Zsh, &path),
            "source '/home/user/my config/.zshrc'"
        );
        assert_eq!(
            shell_source_command(&ShellType::PowerShell, &path),
            ". '/home/user/my config/.zshrc'"
        );
    }

    #[test]
    fn powershell_shell_detection_accepts_pwsh_names() {
        assert!(matches!("pwsh".parse().unwrap(), ShellType::PowerShell));
        assert!(matches!("pwsh.exe".parse().unwrap(), ShellType::PowerShell));
        assert!(matches!(
            r"C:\Program Files\PowerShell\7\pwsh.exe".parse().unwrap(),
            ShellType::PowerShell
        ));
        assert!(matches!(
            "powershell".parse().unwrap(),
            ShellType::PowerShell
        ));
    }

    #[cfg(unix)]
    #[test]
    fn proxy_scripts_fail_fast_when_not_sourced() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let preset_dir = manifest_dir.join("presets/shell/proxy");

        for script in ["set_proxy.sh", "uset_proxy.sh"] {
            let output = std::process::Command::new("bash")
                .arg(preset_dir.join(script))
                .output()
                .expect("proxy script should run under bash");

            assert!(
                !output.status.success(),
                "{script} should fail when executed directly"
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains("must be sourced"),
                "{script} should explain source requirement: {stderr}"
            );
        }
    }
}
