mod install;
mod links;
pub mod metadata;
mod profile;
mod report;
mod template;
mod uninstall;

pub use install::{
    handle_completion_install, handle_init_template, handle_install, handle_upgrade_installed,
};
pub use report::{ShellUpgradeReport, handle_list};
pub(crate) use template::env_map_for_shell_template;
pub use uninstall::handle_uninstall;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub const SENTINEL_START: &str = "# >>> shine >>>";
const SENTINEL_END: &str = "# <<< shine <<<";

#[derive(Debug)]
enum PathUpdateStatus {
    AlreadyConfigured,
    Updated(PathBuf),
}

#[derive(Debug)]
struct ShellConfigUpdate {
    profile_updated: bool,
    config_status: PathUpdateStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ShellType {
    Bash,
    Fish,
    Zsh,
    PowerShell,
    Elvish,
}

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

impl FromStr for ShellType {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let shell_name = s
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(s)
            .to_ascii_lowercase();
        let normalized = shell_name.trim_end_matches(".exe");
        if normalized == "bash" {
            Ok(ShellType::Bash)
        } else if normalized == "fish" {
            Ok(ShellType::Fish)
        } else if normalized == "zsh" {
            Ok(ShellType::Zsh)
        } else if normalized == "powershell" || normalized == "pwsh" {
            Ok(ShellType::PowerShell)
        } else if normalized == "elvish" {
            Ok(ShellType::Elvish)
        } else {
            bail!("Unknown shell item type: {}", s)
        }
    }
}

impl From<ShellType> for &'static str {
    fn from(value: ShellType) -> Self {
        match value {
            ShellType::Bash => "bash",
            ShellType::Fish => "fish",
            ShellType::Zsh => "zsh",
            ShellType::PowerShell => "powershell",
            ShellType::Elvish => "elvish",
        }
    }
}

impl Default for ShellType {
    fn default() -> Self {
        get_shell().unwrap_or(ShellType::Zsh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use profile::shell_source_command;
    use profile::{
        managed_profile_snippet, powershell_bin_assignment, powershell_quote,
        remove_sentinel_block, shell_config_snippet,
    };

    #[test]
    fn managed_profile_uses_home_relative_bin_path() {
        let home = PathBuf::from("/home/user");
        let bin = home.join(".shine/bin");
        let snippet = managed_profile_snippet(&ShellType::Zsh, &bin, &home, &[]);
        assert!(
            snippet.contains("$HOME/.shine/bin"),
            "should use $HOME: {snippet}"
        );
        assert!(!snippet.contains(SENTINEL_START));
        assert!(!snippet.contains(SENTINEL_END));
    }

    #[test]
    fn managed_profile_uses_absolute_bin_path_when_outside_home() {
        let home = PathBuf::from("/home/user");
        let bin = PathBuf::from("/opt/shine/bin");
        let snippet = managed_profile_snippet(&ShellType::Zsh, &bin, &home, &[]);
        assert!(
            snippet.contains("/opt/shine/bin"),
            "should use absolute: {snippet}"
        );
        assert!(!snippet.contains("$HOME"));
    }

    #[test]
    fn snippet_fish_uses_fish_add_path() {
        let home = PathBuf::from("/home/user");
        let bin = home.join("bin");
        let snippet = managed_profile_snippet(&ShellType::Fish, &bin, &home, &[]);
        assert!(
            snippet.contains("fish_add_path"),
            "fish should use fish_add_path: {snippet}"
        );
    }

    #[test]
    fn snippet_bash_zsh_uses_if_guard() {
        let home = PathBuf::from("/home/user");
        let bin = home.join("bin");
        for shell in [ShellType::Bash, ShellType::Zsh] {
            let snippet = managed_profile_snippet(&shell, &bin, &home, &[]);
            assert!(
                snippet.contains("if [["),
                "{shell:?} should have if-guard: {snippet}"
            );
            assert!(snippet.contains("export PATH="));
            let shell_name: &'static str = shell.into();
            assert!(
                snippet.contains(&format!("COMPLETE={shell_name} shine")),
                "{shell:?} should register shine completion: {snippet}"
            );
            if matches!(shell, ShellType::Zsh) {
                assert!(
                    snippet.contains("autoload -Uz compinit"),
                    "zsh completion registration should initialize compinit: {snippet}"
                );
                assert!(
                    snippet.contains("compinit -i"),
                    "zsh completion registration should avoid insecure-dir prompts: {snippet}"
                );
            }
        }
    }

    #[test]
    fn snippet_powershell_registers_completion_but_fish_does_not() {
        let home = PathBuf::from("/home/user");
        let bin = home.join("bin");

        let powershell = managed_profile_snippet(&ShellType::PowerShell, &bin, &home, &[]);
        assert!(
            powershell.contains("$env:COMPLETE = 'powershell'"),
            "PowerShell should register shine completion: {powershell}"
        );

        let fish = managed_profile_snippet(&ShellType::Fish, &bin, &home, &[]);
        assert!(
            !fish.contains("COMPLETE=fish shine"),
            "fish completion should not be changed: {fish}"
        );
    }

    #[test]
    fn snippet_source_commands_generate_wrapper_functions() {
        let home = PathBuf::from("/home/user");
        let bin = home.join(".shine/bin");
        let cmds = vec!["setproxy".to_string(), "usetproxy".to_string()];
        for shell in [ShellType::Bash, ShellType::Zsh] {
            let snippet = managed_profile_snippet(&shell, &bin, &home, &cmds);
            assert!(
                snippet.contains("setproxy() { source"),
                "{shell:?} should have setproxy wrapper: {snippet}"
            );
            assert!(
                snippet.contains("usetproxy() { source"),
                "{shell:?} should have usetproxy wrapper: {snippet}"
            );
        }
        let fish_snippet = managed_profile_snippet(&ShellType::Fish, &bin, &home, &cmds);
        assert!(
            fish_snippet.contains("function setproxy"),
            "fish should have setproxy function: {fish_snippet}"
        );
        let powershell_snippet =
            managed_profile_snippet(&ShellType::PowerShell, &bin, &home, &cmds);
        assert!(
            powershell_snippet.contains("$env:Path"),
            "PowerShell should update env Path: {powershell_snippet}"
        );
        assert!(
            powershell_snippet.contains("function setproxy"),
            "PowerShell should have setproxy function: {powershell_snippet}"
        );
        assert!(
            powershell_snippet.contains("Join-Path $shineBin"),
            "PowerShell wrapper should resolve through shine bin: {powershell_snippet}"
        );
        assert!(
            powershell_snippet.contains("$shineBin = Join-Path $HOME '.shine/bin'"),
            "PowerShell should expand $HOME when assigning shine bin: {powershell_snippet}"
        );
        assert!(
            !powershell_snippet.contains("$shineBin = '$HOME"),
            "PowerShell should not keep $HOME as a literal path: {powershell_snippet}"
        );
    }

    #[test]
    fn shell_config_snippet_sources_managed_profile_only() {
        let home = PathBuf::from("/home/user");
        let profile = home.join(".shine/shell/profile.sh");
        let snippet = shell_config_snippet(&ShellType::Zsh, &profile, &home);
        assert!(snippet.contains(SENTINEL_START));
        assert!(snippet.contains("source \"$HOME/.shine/shell/profile.sh\""));
        assert!(!snippet.contains("export PATH"));
        assert!(!snippet.contains("function setproxy"));
    }

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

    #[test]
    fn powershell_paths_strip_windows_verbatim_prefix() {
        let assignment = powershell_bin_assignment(r"\\?\D:\Github\Biulight\shine\.shine\bin");
        assert!(assignment.contains(r"D:\Github\Biulight\shine\.shine\bin"));
        assert!(!assignment.contains(r"\\?\"));

        let quoted = powershell_quote(Path::new(r"\\?\D:\Github\Biulight\shine\profile.ps1"));
        assert_eq!(quoted, r"'D:\Github\Biulight\shine\profile.ps1'");
    }

    #[cfg(unix)]
    #[test]
    fn proxy_scripts_fail_fast_when_not_sourced() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let preset_dir = manifest_dir.join("../presets/shell/proxy");

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

    #[test]
    fn remove_sentinel_block_strips_block_and_blank_line() {
        let content = "before\n\n# >>> shine >>>\nexport PATH\n# <<< shine <<<\nafter\n";
        let cleaned = remove_sentinel_block(content);
        assert_eq!(cleaned, "before\nafter\n");
    }

    #[test]
    fn remove_sentinel_block_no_op_when_absent() {
        let content = "no sentinel here\n";
        let cleaned = remove_sentinel_block(content);
        assert_eq!(cleaned, content);
    }
}
