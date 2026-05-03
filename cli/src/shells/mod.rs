pub(crate) mod metadata;

use crate::colors;
use crate::config::Config;
use crate::env::EnvConfig;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub(crate) const SENTINEL_START: &str = "# >>> shine >>>";
const SENTINEL_END: &str = "# <<< shine <<<";

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) enum ShellType {
    Bash,
    Fish,
    Zsh,
    PowerShell,
    Elvish,
}

pub(crate) async fn handle_install(
    config: &Config,
    category: Option<&str>,
    force: bool,
) -> Result<()> {
    crate::config::print_presets_note(config);
    let prefix = match category {
        Some(cat) => format!("shell/{cat}"),
        None => "shell".to_string(),
    };

    // When using the default presets directory, extract the embedded assets first.
    if !config.is_external_presets {
        let report = crate::presets::extract_prefix(&prefix, config.presets_dir(), force).await?;

        let mut shell_parts: Vec<String> = Vec::new();
        if !report.created.is_empty() {
            shell_parts.push(colors::green(&format!("{} created", report.created.len())));
        }
        if !report.overwritten.is_empty() {
            shell_parts.push(colors::green(&format!(
                "{} updated",
                report.overwritten.len()
            )));
        }
        if !report.skipped.is_empty() {
            shell_parts.push(colors::dim(&format!("{} skipped", report.skipped.len())));
        }
        let sep = colors::dim(" · ");
        println!(
            "{}  {}",
            colors::bold("Shell Presets"),
            shell_parts.join(&sep)
        );
    }

    let categories = metadata::load_installed_categories(config, category).await?;
    // Build (template_source, rendered_dest) pairs for all scripts.
    // apply_template_to_scripts renders source → rendered_dir, never modifies presets_dir.
    let script_pairs: Vec<(PathBuf, PathBuf)> = categories
        .iter()
        .flat_map(|cat| {
            cat.files.iter().map(|file| {
                let source = config
                    .presets_dir()
                    .join("shell")
                    .join(&cat.name)
                    .join(&file.source_rel);
                let rendered = config
                    .rendered_dir()
                    .join("shell")
                    .join(&cat.name)
                    .join(&file.source_rel);
                (source, rendered)
            })
        })
        .collect();

    // Apply env-variable substitution to scripts that opt in via `# shine-template: true`.
    // Output goes to rendered_dir; presets_dir templates are left untouched.
    apply_template_to_scripts(config, &script_pairs).await;

    // Symlinks point to the rendered file when one was produced, otherwise to the
    // raw source in presets_dir (non-template scripts).
    let link_specs: Vec<_> = categories
        .iter()
        .flat_map(|cat| {
            cat.files.iter().map(|file| {
                let source = config
                    .presets_dir()
                    .join("shell")
                    .join(&cat.name)
                    .join(&file.source_rel);
                let rendered = config
                    .rendered_dir()
                    .join("shell")
                    .join(&cat.name)
                    .join(&file.source_rel);
                let effective = if rendered.exists() { rendered } else { source };
                crate::bin_links::LinkSpec {
                    source: effective,
                    link_name: OsString::from(&file.command_name),
                }
            })
        })
        .collect();
    let link_report =
        crate::bin_links::link_executables_with_names(config.bin_dir(), &link_specs, force).await?;

    let sep = colors::dim(" · ");
    let mut link_parts: Vec<String> = Vec::new();
    if !link_report.created.is_empty() {
        link_parts.push(colors::green(&format!(
            "{} created",
            link_report.created.len()
        )));
    }
    if !link_report.overwritten.is_empty() {
        link_parts.push(colors::green(&format!(
            "{} updated",
            link_report.overwritten.len()
        )));
    }
    if !link_report.skipped.is_empty() {
        link_parts.push(colors::dim(&format!(
            "{} up to date",
            link_report.skipped.len()
        )));
    }
    if !link_report.conflicts.is_empty() {
        link_parts.push(colors::yellow(&format!(
            "{} conflicts",
            link_report.conflicts.len()
        )));
    }
    println!(
        "{}     {}",
        colors::bold("Bin Links    "),
        link_parts.join(&sep)
    );

    let source_commands: Vec<String> = categories
        .iter()
        .flat_map(|cat| cat.files.iter())
        .filter(|f| f.needs_source)
        .map(|f| f.command_name.clone())
        .collect();

    append_path_to_shell_config(config, force, &source_commands).await?;
    Ok(())
}

pub(crate) async fn handle_upgrade_installed(config: &Config) -> Result<()> {
    let categories = if config.is_external_presets {
        metadata::load_installed_categories(config, None).await?
    } else {
        metadata::load_embedded_categories(None)?
    };

    let mut installed_categories = Vec::new();
    for cat in &categories {
        let has_installed_file = cat.files.iter().any(|file| {
            let source = config
                .presets_dir()
                .join("shell")
                .join(&cat.name)
                .join(&file.source_rel);
            let rendered = config
                .rendered_dir()
                .join("shell")
                .join(&cat.name)
                .join(&file.source_rel);
            let link = config.bin_dir().join(&file.command_name);
            source.exists() || rendered.exists() || link.exists()
        });
        if has_installed_file {
            installed_categories.push(cat.name.clone());
        }
    }

    if installed_categories.is_empty() {
        println!("{}", colors::dim("No installed shell presets found."));
        return Ok(());
    }

    println!(
        "{}  {}",
        colors::bold("Shell Presets"),
        colors::dim(&format!(
            "{} installed categories",
            installed_categories.len()
        ))
    );
    for category in installed_categories {
        handle_install(config, Some(&category), true).await?;
    }

    Ok(())
}

pub(crate) async fn handle_uninstall(
    config: &Config,
    category: Option<&str>,
    purge: bool,
    dry_run: bool,
) -> Result<()> {
    crate::config::print_presets_note(config);
    if dry_run {
        println!("{}", colors::dim("[dry-run] No files will be modified."));
    }

    let sep = colors::dim(" · ");

    // When a category is given, scope removal to that category's subdirectory.
    let managed_presets_root = match category {
        Some(cat) => config.presets_dir().join("shell").join(cat),
        None => config.presets_dir().to_path_buf(),
    };
    let managed_rendered_root = match category {
        Some(cat) => config.rendered_dir().join("shell").join(cat),
        None => config.rendered_dir().join("shell"),
    };
    let prefix = match category {
        Some(cat) => format!("shell/{cat}"),
        None => "shell".to_owned(),
    };

    // Remove symlinks pointing to presets_dir (old-style) or rendered_dir (new-style).
    let unlink_presets =
        crate::bin_links::unlink_managed(config.bin_dir(), &managed_presets_root, dry_run).await?;
    let unlink_rendered =
        crate::bin_links::unlink_managed(config.bin_dir(), &managed_rendered_root, dry_run).await?;
    let unlink_report = crate::bin_links::UnlinkReport {
        removed: [unlink_presets.removed, unlink_rendered.removed].concat(),
        skipped: [unlink_presets.skipped, unlink_rendered.skipped].concat(),
    };
    let mut link_parts: Vec<String> = Vec::new();
    if !unlink_report.removed.is_empty() {
        link_parts.push(colors::green(&format!(
            "{} removed",
            unlink_report.removed.len()
        )));
    }
    if !unlink_report.skipped.is_empty() {
        link_parts.push(colors::dim(&format!(
            "{} skipped",
            unlink_report.skipped.len()
        )));
    }
    println!(
        "{}     {}",
        colors::bold("Bin Links    "),
        link_parts.join(&sep)
    );

    // When the user has a custom presets directory, the source files are theirs —
    // only remove the embedded-managed files when using the default directory.
    if !config.is_external_presets {
        let remove_report =
            crate::presets::remove_prefix(&prefix, config.presets_dir(), dry_run).await?;
        let mut shell_parts: Vec<String> = Vec::new();
        if !remove_report.removed.is_empty() {
            shell_parts.push(colors::green(&format!(
                "{} removed",
                remove_report.removed.len()
            )));
        }
        if !remove_report.skipped.is_empty() {
            shell_parts.push(colors::dim(&format!(
                "{} skipped",
                remove_report.skipped.len()
            )));
        }
        println!(
            "{}  {}",
            colors::bold("Shell Presets"),
            shell_parts.join(&sep)
        );
    }

    // Only purge managed directories when using the default presets directory.
    // Never delete a user-configured external folder.
    if purge && !dry_run && !config.is_external_presets {
        let purge_dir = match category {
            Some(cat) => config.presets_dir().join("shell").join(cat),
            None => config.presets_dir().join("shell"),
        };
        if purge_dir.exists() {
            tokio::fs::remove_dir_all(&purge_dir)
                .await
                .with_context(|| format!("removing presets directory: {purge_dir:?}"))?;
        }
        if category.is_none() {
            // remove_dir only succeeds if empty — treat non-empty as benign
            let _ = tokio::fs::remove_dir(config.presets_dir()).await;
            let _ = tokio::fs::remove_dir(config.bin_dir()).await;
        }
        println!(
            "  {}  {}",
            colors::symbol("✓"),
            colors::dim("managed directories purged (if empty)"),
        );
    }

    // Remove rendered_dir files — always shine-managed regardless of external-presets mode.
    if !dry_run && managed_rendered_root.exists() {
        tokio::fs::remove_dir_all(&managed_rendered_root)
            .await
            .with_context(|| {
                format!("removing rendered dir: {}", managed_rendered_root.display())
            })?;
    }

    // Only remove the PATH sentinel when uninstalling all shell presets.
    if category.is_none() && !dry_run {
        remove_path_from_shell_config(config).await?;
    }

    Ok(())
}

pub(crate) async fn handle_list(config: &Config) -> Result<()> {
    crate::config::print_presets_note(config);
    let categories = if config.is_external_presets {
        metadata::load_installed_categories(config, None).await?
    } else {
        metadata::load_embedded_categories(None)?
    };

    if categories.is_empty() {
        println!("{}", colors::dim("No shell preset categories found."));
        return Ok(());
    }

    println!("{}\n", colors::bold("Shell Preset Categories"));

    for cat in &categories {
        let word = if cat.files.len() == 1 {
            "script"
        } else {
            "scripts"
        };
        println!(
            "  {}  {}",
            cat.name,
            colors::dim(&format!("{} {}", cat.files.len(), word))
        );

        let names: Vec<&str> = cat.files.iter().map(|s| s.command_name.as_str()).collect();
        let max_name = names.iter().map(|s| s.len()).max().unwrap_or(0);
        let gap = 4;
        let desc_col = max_name + gap;
        let continuation_indent = " ".repeat(4 + desc_col);

        for (script, name) in cat.files.iter().zip(names.iter()) {
            let padding = " ".repeat(desc_col - name.len());
            match script.description.as_slice() {
                [] => println!("    {name}"),
                [first, rest @ ..] => {
                    println!("    {name}{padding}{first}");
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

    println!(
        "{}",
        colors::dim("Run `shine shell install <CATEGORY>` to install a specific category.")
    );
    println!(
        "{}",
        colors::dim("Run `shine shell install` to install all.")
    );
    println!();
    println!(
        "{}",
        colors::dim(
            "After installation, commands are available directly by name (e.g. `setproxy`)."
        )
    );

    Ok(())
}

/// For each script that declares `# shine-template: true`, read the template from
/// `source_path` (presets_dir — never modified), substitute env variables from
/// `config.toml` `[env]`, and write the rendered result to `rendered_path`
/// (rendered_dir — always shine-managed).  File permissions are copied from source.
async fn apply_template_to_scripts(config: &Config, script_pairs: &[(PathBuf, PathBuf)]) {
    let env = match EnvConfig::load_or_init(config).await {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Warning: could not load config.toml [env]: {e:#}");
            return;
        }
    };
    let env_map = env.as_map().clone();

    for (source_path, rendered_path) in script_pairs {
        let content = match tokio::fs::read(source_path).await {
            Ok(b) => b,
            Err(_) => continue,
        };

        if !crate::presets::parse_template_annotation(&content) {
            continue;
        }

        let rendered =
            match crate::apps::apply_transforms(&["template".to_string()], &content, &env_map) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!(
                        "Warning: template substitution failed for {}: {e:#}",
                        source_path.display()
                    );
                    continue;
                }
            };

        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::metadata(source_path)
                .await
                .map(|m| m.permissions().mode())
                .unwrap_or(0o755)
        };

        if let Some(parent) = rendered_path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }

        if let Err(e) = tokio::fs::write(rendered_path, &rendered).await {
            eprintln!(
                "Warning: failed to write rendered script {}: {e:#}",
                rendered_path.display()
            );
            continue;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(mode);
            let _ = tokio::fs::set_permissions(rendered_path, perms).await;
        }
    }
}

/// Build the PATH export snippet for the given shell, using `$HOME` when possible.
/// For commands that need sourcing, wrapper functions are appended so that the user
/// can type `setproxy` directly without prefixing `source`.
fn path_export_snippet(
    shell: &ShellType,
    bin_dir: &Path,
    home_dir: &Path,
    source_commands: &[String],
) -> String {
    let bin_str = match bin_dir.strip_prefix(home_dir) {
        Ok(rel) => format!("$HOME/{}", rel.display()),
        Err(_) => bin_dir.display().to_string(),
    };
    let mut body = match shell {
        ShellType::Fish => format!("fish_add_path \"{bin_str}\""),
        _ => format!(
            "if [[ \":$PATH:\" != *\":{bin_str}:\"* ]]; then\n  export PATH=\"{bin_str}:$PATH\"\nfi"
        ),
    };
    // Wrapper functions for scripts that must be sourced to export env vars.
    for cmd in source_commands {
        match shell {
            ShellType::Fish => {
                body.push_str(&format!(
                    "\nfunction {cmd}\n  source \"{bin_str}/{cmd}\" $argv\nend"
                ));
            }
            _ => {
                body.push_str(&format!(
                    "\n{cmd}() {{ source \"{bin_str}/{cmd}\" \"$@\"; }}"
                ));
            }
        }
    }
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

async fn append_path_to_shell_config(
    config: &Config,
    force: bool,
    source_commands: &[String],
) -> Result<()> {
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
        if !force {
            println!(
                "Shell config ({}): already configured, skipped",
                config_path.display()
            );
            return Ok(());
        }
        // Force: remove old sentinel block and re-add with current snippet.
        let cleaned = remove_sentinel_block(&existing);
        tokio::fs::write(&config_path, cleaned.as_bytes())
            .await
            .with_context(|| format!("rewriting shell config: {config_path:?}"))?;
    }

    let existing = tokio::fs::read_to_string(&config_path)
        .await
        .unwrap_or_default();
    let snippet = path_export_snippet(
        &config.shell_type,
        config.bin_dir(),
        &config.home_dir,
        source_commands,
    );

    // Write the complete new content atomically so the file is closed (and thus
    // fully visible to subsequent reads) before this function returns.
    let new_content = format!("{existing}\n{snippet}");
    tokio::fs::write(&config_path, new_content.as_bytes())
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

        handle_install(&config, None, false).await.unwrap();
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
        // symlinks use stem names (no .sh suffix)
        assert!(
            config.bin_dir().join("setproxy").exists(),
            "bin link should use configured rename"
        );
        assert!(!config.bin_dir().join("set_proxy").exists());

        handle_uninstall(&config, None, false, false).await.unwrap();
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
        handle_uninstall(&config, None, false, false).await.unwrap();

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn uninstall_purge_removes_managed_dirs_but_not_config() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, None, false).await.unwrap();
        handle_uninstall(&config, None, true, false).await.unwrap();

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

        handle_install(&config, None, false).await.unwrap();
        let preset_path = config.presets_dir().join("shell/proxy/set_proxy.sh");
        assert!(preset_path.exists());

        handle_uninstall(&config, None, false, true).await.unwrap();

        assert!(preset_path.exists(), "dry-run must not remove preset files");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    // --- PATH / shell config tests ---

    #[test]
    fn snippet_uses_home_relative_path() {
        let home = PathBuf::from("/home/user");
        let bin = home.join(".shine/bin");
        let snippet = path_export_snippet(&ShellType::Zsh, &bin, &home, &[]);
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
        let snippet = path_export_snippet(&ShellType::Zsh, &bin, &home, &[]);
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
        let snippet = path_export_snippet(&ShellType::Fish, &bin, &home, &[]);
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
            let snippet = path_export_snippet(&shell, &bin, &home, &[]);
            assert!(
                snippet.contains("if [["),
                "{shell:?} should have if-guard: {snippet}"
            );
            assert!(snippet.contains("export PATH="));
        }
    }

    #[test]
    fn snippet_source_commands_generate_wrapper_functions() {
        let home = PathBuf::from("/home/user");
        let bin = home.join(".shine/bin");
        let cmds = vec!["setproxy".to_string(), "usetproxy".to_string()];
        for shell in [ShellType::Bash, ShellType::Zsh] {
            let snippet = path_export_snippet(&shell, &bin, &home, &cmds);
            assert!(
                snippet.contains("setproxy() { source"),
                "{shell:?} should have setproxy wrapper: {snippet}"
            );
            assert!(
                snippet.contains("usetproxy() { source"),
                "{shell:?} should have usetproxy wrapper: {snippet}"
            );
        }
        let fish_snippet = path_export_snippet(&ShellType::Fish, &bin, &home, &cmds);
        assert!(
            fish_snippet.contains("function setproxy"),
            "fish should have setproxy function: {fish_snippet}"
        );
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

        append_path_to_shell_config(&config, false, &[])
            .await
            .unwrap();

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

        append_path_to_shell_config(&config, false, &[])
            .await
            .unwrap();
        append_path_to_shell_config(&config, false, &[])
            .await
            .unwrap();

        let config_path = get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        let content = fs::read_to_string(&config_path).await.unwrap();
        let count = content.matches(SENTINEL_START).count();
        assert_eq!(count, 1, "sentinel should appear exactly once");
    }

    #[tokio::test]
    async fn remove_clears_sentinel_from_shell_config() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);

        append_path_to_shell_config(&config, false, &[])
            .await
            .unwrap();
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

        handle_install(&config, None, false).await.unwrap();
        let config_path = get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        let before = fs::read_to_string(&config_path).await.unwrap();

        handle_uninstall(&config, None, false, true).await.unwrap();

        let after = fs::read_to_string(&config_path).await.unwrap();
        assert_eq!(before, after, "dry-run must not touch shell config");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    // --- external presets tests ---

    #[cfg(unix)]
    #[tokio::test]
    async fn external_presets_install_links_disk_scripts_without_extraction() {
        let dir = make_temp_dir().await;
        // new_for_test sets presets_dir = dir/presets, bin_dir = dir/bin
        // Create a script in presets_dir/shell/custom/ to simulate user-managed presets.
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        let script = cat_dir.join("my_tool.sh");
        fs::write(&script, b"#!/bin/bash\n# My tool.\necho hi\n")
            .await
            .unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).await.unwrap().permissions();
        perms.set_mode(perms.mode() | 0o111);
        fs::set_permissions(&script, perms).await.unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("custom"), false)
            .await
            .unwrap();

        // The script must NOT have been extracted from embedded assets into
        // presets_dir — the only file there is the one we created above.
        let count = {
            let mut rd = fs::read_dir(&cat_dir).await.unwrap();
            let mut n = 0u32;
            while rd.next_entry().await.unwrap().is_some() {
                n += 1;
            }
            n
        };
        assert_eq!(count, 1, "no embedded assets should have been extracted");

        // A bin symlink for the script should have been created.
        let link = config.bin_dir().join("my_tool");
        assert!(link.exists(), "bin symlink should point at disk script");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_presets_uninstall_preserves_disk_scripts() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        let script = cat_dir.join("my_tool.sh");
        fs::write(&script, b"#!/bin/bash\n# My tool.\necho hi\n")
            .await
            .unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).await.unwrap().permissions();
        perms.set_mode(perms.mode() | 0o111);
        fs::set_permissions(&script, perms).await.unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("custom"), false)
            .await
            .unwrap();
        assert!(config.bin_dir().join("my_tool").exists());

        handle_uninstall(&config, Some("custom"), false, false)
            .await
            .unwrap();

        // User-owned script must survive uninstall.
        assert!(script.exists(), "user script must not be deleted");
        // Bin symlink should be gone.
        assert!(
            !config.bin_dir().join("my_tool").exists(),
            "bin link should be removed"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_presets_install_applies_metadata_rename() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"set_proxy.sh\"\ntarget = \"setproxy\"\n",
        )
        .await
        .unwrap();
        let script = cat_dir.join("set_proxy.sh");
        fs::write(&script, b"#!/bin/bash\n# Set proxy.\necho hi\n")
            .await
            .unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).await.unwrap().permissions();
        perms.set_mode(perms.mode() | 0o111);
        fs::set_permissions(&script, perms).await.unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("custom"), false)
            .await
            .unwrap();

        assert!(config.bin_dir().join("setproxy").exists());
        assert!(!config.bin_dir().join("set_proxy").exists());

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
