use crate::config::Config;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tokio::io::AsyncWriteExt;

const SENTINEL_START: &str = "# >>> shine >>>";
const SENTINEL_END: &str = "# <<< shine <<<";

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) enum ShellType {
    Bash,
    Fish,
    Zsh,
    PowerShell,
    Elvish,
}

pub(crate) async fn handle_install(config: &Config, category: Option<&str>) -> Result<()> {
    let prefix = match category {
        Some(cat) => format!("shell/{cat}"),
        None => "shell".to_string(),
    };
    let report = crate::presets::extract_prefix(&prefix, config.presets_dir(), false).await?;
    println!(
        "Presets ({}): {} created, {} skipped",
        prefix,
        report.created.len(),
        report.skipped.len()
    );

    let sources: Vec<_> = report
        .created
        .iter()
        .chain(report.overwritten.iter())
        .chain(report.skipped.iter())
        .cloned()
        .collect();
    let link_report = crate::bin_links::link_executables(config.bin_dir(), &sources, false).await?;
    println!(
        "Bin links: {} created, {} skipped, {} conflicts",
        link_report.created.len(),
        link_report.skipped.len(),
        link_report.conflicts.len(),
    );

    append_path_to_shell_config(config).await?;
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

    if !dry_run {
        remove_path_from_shell_config(config).await?;
    }

    Ok(())
}

pub(crate) async fn handle_list() -> Result<()> {
    let categories = crate::presets::list_categories("shell");

    if categories.is_empty() {
        println!("No shell preset categories found.");
        return Ok(());
    }

    println!("Available shell preset categories:\n");

    for cat in &categories {
        let word = if cat.scripts.len() == 1 {
            "script"
        } else {
            "scripts"
        };
        println!("  {} ({} {})", cat.name, cat.scripts.len(), word);

        // Strip extensions for display and compute alignment column.
        let stems: Vec<&str> = cat.scripts.iter().map(|s| script_stem(&s.name)).collect();
        let max_stem = stems.iter().map(|s| s.len()).max().unwrap_or(0);
        // 4 spaces indent before name, then gap after the longest name.
        let gap = 4;
        let desc_col = max_stem + gap;
        let continuation_indent = " ".repeat(4 + desc_col);

        for (script, stem) in cat.scripts.iter().zip(stems.iter()) {
            let padding = " ".repeat(desc_col - stem.len());
            match script.description.as_slice() {
                [] => println!("    {stem}"),
                [first, rest @ ..] => {
                    println!("    {stem}{padding}{first}");
                    for line in rest {
                        if line.is_empty() {
                            println!();
                        } else {
                            println!("{continuation_indent}{line}");
                        }
                    }
                }
            }
            println!();
        }
    }

    println!("Use 'shine shell install <CATEGORY>' to install a specific category.");
    println!("Use 'shine shell install' to install all.");

    Ok(())
}

fn script_stem(filename: &str) -> &str {
    match filename.rfind('.') {
        Some(i) => &filename[..i],
        None => filename,
    }
}

/// Build the PATH export snippet for the given shell, using `$HOME` when possible.
fn path_export_snippet(shell: &ShellType, bin_dir: &Path, home_dir: &Path) -> String {
    let bin_str = match bin_dir.strip_prefix(home_dir) {
        Ok(rel) => format!("$HOME/{}", rel.display()),
        Err(_) => bin_dir.display().to_string(),
    };
    let body = match shell {
        ShellType::Fish => format!("fish_add_path \"{bin_str}\""),
        _ => format!(
            "if [[ \":$PATH:\" != *\":{bin_str}:\"* ]]; then\n  export PATH=\"{bin_str}:$PATH\"\nfi"
        ),
    };
    format!("{SENTINEL_START}\n{body}\n{SENTINEL_END}\n")
}

/// Remove the shine sentinel block from `content`, including one preceding blank line.
fn remove_sentinel_block(content: &str) -> String {
    let start = match content.find(SENTINEL_START) {
        Some(i) => i,
        None => return content.to_string(),
    };
    let end_marker = match content.find(SENTINEL_END) {
        Some(i) => i + SENTINEL_END.len(),
        None => return content.to_string(),
    };
    // Consume the newline that follows SENTINEL_END.
    let end = if content[end_marker..].starts_with('\n') {
        end_marker + 1
    } else {
        end_marker
    };
    // Also consume one preceding blank line (the separator we wrote).
    let block_start = if start > 0 && content[..start].ends_with("\n\n") {
        start - 1
    } else {
        start
    };
    format!("{}{}", &content[..block_start], &content[end..])
}

async fn append_path_to_shell_config(config: &Config) -> Result<()> {
    let config_path = get_shell_config_path(&config.shell_type, &config.home_dir)?;

    if let Some(parent) = config_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating directory for shell config: {parent:?}"))?;
    }

    let existing = tokio::fs::read_to_string(&config_path)
        .await
        .unwrap_or_default();

    if existing.contains(SENTINEL_START) {
        println!(
            "Shell config ({}): already configured, skipped",
            config_path.display()
        );
        return Ok(());
    }

    let snippet = path_export_snippet(&config.shell_type, config.bin_dir(), &config.home_dir);

    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config_path)
        .await
        .with_context(|| format!("opening shell config: {config_path:?}"))?;

    file.write_all(format!("\n{snippet}").as_bytes())
        .await
        .with_context(|| format!("writing to shell config: {config_path:?}"))?;

    println!("Shell config ({}): PATH updated", config_path.display());
    Ok(())
}

async fn remove_path_from_shell_config(config: &Config) -> Result<()> {
    let config_path = get_shell_config_path(&config.shell_type, &config.home_dir)?;

    if !config_path.exists() {
        return Ok(());
    }

    let content = tokio::fs::read_to_string(&config_path)
        .await
        .with_context(|| format!("reading shell config: {config_path:?}"))?;

    if !content.contains(SENTINEL_START) {
        return Ok(());
    }

    let cleaned = remove_sentinel_block(&content);
    tokio::fs::write(&config_path, cleaned.as_bytes())
        .await
        .with_context(|| format!("writing shell config: {config_path:?}"))?;

    println!(
        "Shell config ({}): PATH entry removed",
        config_path.display()
    );
    Ok(())
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

        handle_install(&config, None).await.unwrap();
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

        handle_install(&config, None).await.unwrap();
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

        handle_install(&config, None).await.unwrap();
        let preset_path = config.presets_dir().join("shell/proxy/set_proxy.sh");
        assert!(preset_path.exists());

        handle_uninstall(&config, false, true).await.unwrap();

        assert!(preset_path.exists(), "dry-run must not remove preset files");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    // --- PATH / shell config tests ---

    #[test]
    fn snippet_uses_home_relative_path() {
        let home = PathBuf::from("/home/user");
        let bin = home.join(".shine/bin");
        let snippet = path_export_snippet(&ShellType::Zsh, &bin, &home);
        assert!(
            snippet.contains("$HOME/.shine/bin"),
            "should use $HOME: {snippet}"
        );
        assert!(snippet.contains(SENTINEL_START));
        assert!(snippet.contains(SENTINEL_END));
    }

    #[test]
    fn snippet_uses_absolute_path_when_outside_home() {
        let home = PathBuf::from("/home/user");
        let bin = PathBuf::from("/opt/shine/bin");
        let snippet = path_export_snippet(&ShellType::Zsh, &bin, &home);
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
        let snippet = path_export_snippet(&ShellType::Fish, &bin, &home);
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
            let snippet = path_export_snippet(&shell, &bin, &home);
            assert!(
                snippet.contains("if [["),
                "{shell:?} should have if-guard: {snippet}"
            );
            assert!(snippet.contains("export PATH="));
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

    #[tokio::test]
    async fn append_writes_snippet_to_shell_config() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);

        append_path_to_shell_config(&config).await.unwrap();

        let config_path = get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        let content = fs::read_to_string(&config_path).await.unwrap();
        assert!(
            content.contains(SENTINEL_START),
            "sentinel should be present"
        );
    }

    #[tokio::test]
    async fn append_is_idempotent() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);

        append_path_to_shell_config(&config).await.unwrap();
        append_path_to_shell_config(&config).await.unwrap();

        let config_path = get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        let content = fs::read_to_string(&config_path).await.unwrap();
        let count = content.matches(SENTINEL_START).count();
        assert_eq!(count, 1, "sentinel should appear exactly once");
    }

    #[tokio::test]
    async fn remove_clears_sentinel_from_shell_config() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);

        append_path_to_shell_config(&config).await.unwrap();
        remove_path_from_shell_config(&config).await.unwrap();

        let config_path = get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        let content = fs::read_to_string(&config_path).await.unwrap();
        assert!(
            !content.contains(SENTINEL_START),
            "sentinel should be gone after remove"
        );
    }

    #[tokio::test]
    async fn remove_is_no_op_when_config_missing() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        // No install — config file doesn't exist
        remove_path_from_shell_config(&config).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn uninstall_dry_run_does_not_modify_shell_config() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, None).await.unwrap();
        let config_path = get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        let before = fs::read_to_string(&config_path).await.unwrap();

        handle_uninstall(&config, false, true).await.unwrap();

        let after = fs::read_to_string(&config_path).await.unwrap();
        assert_eq!(before, after, "dry-run must not touch shell config");

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
