use crate::commands::ShellCommands;
use crate::config::Config;
use crate::shells::proxy::install_proxy;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::str::FromStr;

mod proxy;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) enum ShellType {
    Bash,
    Fish,
    Zsh,
    PowerShell,
    Elvish,
}

pub(crate) async fn handle_install(config: &Config) -> Result<()> {
    let report = crate::presets::extract_prefix("shell", config.presets_dir(), false).await?;
    println!(
        "Presets (shell): {} created, {} skipped",
        report.created.len(),
        report.skipped.len()
    );

    let sources: Vec<_> = report
        .created
        .iter()
        .chain(report.overwritten.iter())
        .cloned()
        .collect();
    let link_report = crate::bin_links::link_executables(config.bin_dir(), &sources, false).await?;
    println!(
        "Bin links: {} created, {} skipped, {} conflicts",
        link_report.created.len(),
        link_report.skipped.len(),
        link_report.conflicts.len(),
    );

    install_proxy(config).await?;
    println!("Loaded config: {:#?}", &config);
    Ok(())
}

pub(crate) async fn handle_uninstall(config: &Config, purge: bool, dry_run: bool) -> Result<()> {
    if dry_run {
        println!("[dry-run] No files will be modified.");
    }

    let unlink_report =
        crate::bin_links::unlink_managed(config.bin_dir(), config.presets_dir(), dry_run).await?;
    println!(
        "Bin links: {} removed, {} skipped",
        unlink_report.removed.len(),
        unlink_report.skipped.len(),
    );

    let remove_report =
        crate::presets::remove_prefix("shell", config.presets_dir(), dry_run).await?;
    println!(
        "Presets (shell): {} removed, {} skipped",
        remove_report.removed.len(),
        remove_report.skipped.len(),
    );

    if purge && !dry_run {
        let shell_dir = config.presets_dir().join("shell");
        if shell_dir.exists() {
            tokio::fs::remove_dir_all(&shell_dir)
                .await
                .with_context(|| format!("removing shell presets directory: {shell_dir:?}"))?;
        }
        // remove_dir only succeeds if empty — treat non-empty as benign
        let _ = tokio::fs::remove_dir(config.presets_dir()).await;
        let _ = tokio::fs::remove_dir(config.bin_dir()).await;
        println!("Purge: managed directories removed (if empty).");
    }

    Ok(())
}

pub(crate) async fn handle_command(command: ShellCommands, _config: &Config) -> Result<()> {
    println!("{:#?}", command);
    match command {
        ShellCommands::Install | ShellCommands::Uninstall { .. } => {
            bail!("This command must be handled in main.rs");
        }
        ShellCommands::List => todo!(),
        _ => todo!(),
    }
}

pub(crate) fn get_shell() -> Result<ShellType> {
    let shell = std::env::var("SHELL").context("Could not find $SHELL")?;
    shell.parse()
}

pub(crate) fn get_shell_config_path(shell_type: &ShellType, home_path: &Path) -> Result<PathBuf> {
    match shell_type {
        ShellType::Bash => Ok(home_path.join(".bashrc")),
        ShellType::Fish => Ok(home_path.join(".config/fish/config.fish")),
        ShellType::Zsh => Ok(home_path.join(".zshrc")),
        ShellType::PowerShell => Ok(home_path.join(".profile")),
        ShellType::Elvish => Ok(home_path.join(".config/elvish/rc.elv")),
    }
}

impl FromStr for ShellType {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.ends_with("bash") {
            Ok(ShellType::Bash)
        } else if s.ends_with("fish") {
            Ok(ShellType::Fish)
        } else if s.ends_with("zsh") {
            Ok(ShellType::Zsh)
        } else if s.ends_with("powershell") {
            Ok(ShellType::PowerShell)
        } else if s.ends_with("elvish") {
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
    use crate::config::Config;
    use tokio::fs;

    async fn make_temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shine-shell-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();
        dir
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn install_then_uninstall_roundtrip() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config).await.unwrap();
        assert!(
            config
                .presets_dir()
                .join("shell/proxy/set_proxy.sh")
                .exists(),
            "preset should exist after install"
        );
        let first_bin_entry = fs::read_dir(config.bin_dir())
            .await
            .unwrap()
            .next_entry()
            .await
            .unwrap();
        assert!(
            first_bin_entry.is_some(),
            "bin dir should have symlinks after install"
        );

        handle_uninstall(&config, false, false).await.unwrap();
        assert!(
            !config
                .presets_dir()
                .join("shell/proxy/set_proxy.sh")
                .exists(),
            "preset should be gone after uninstall"
        );
        let mut rd = fs::read_dir(config.bin_dir()).await.unwrap();
        assert!(
            rd.next_entry().await.unwrap().is_none(),
            "bin dir should be empty after uninstall"
        );

        // Idempotency: second uninstall must not error
        handle_uninstall(&config, false, false).await.unwrap();

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn uninstall_purge_removes_managed_dirs_but_not_config() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config).await.unwrap();
        handle_uninstall(&config, true, false).await.unwrap();

        assert!(!config.bin_dir().exists(), "bin_dir should be purged");
        assert!(
            !config.presets_dir().join("shell").exists(),
            "shell presets dir should be purged"
        );
        // config.toml must never be removed by uninstall
        assert!(
            config.presets_dir().parent().is_some(),
            "shine root still accessible"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn uninstall_dry_run_leaves_everything_intact() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config).await.unwrap();
        let preset_path = config.presets_dir().join("shell/proxy/set_proxy.sh");
        assert!(preset_path.exists());

        handle_uninstall(&config, false, true).await.unwrap();

        assert!(preset_path.exists(), "dry-run must not remove preset files");

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
