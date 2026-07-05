use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::output;
use crate::path_display;

use super::{
    PathUpdateStatus, SENTINEL_END, SENTINEL_START, ShellConfigUpdate, ShellType,
    get_shell_config_paths,
};

pub(super) fn managed_shell_profile_path(config: &Config) -> PathBuf {
    config
        .shine_dir()
        .join("shell")
        .join(match config.shell_type {
            ShellType::Fish => "config.fish",
            ShellType::PowerShell => "profile.ps1",
            _ => "profile.sh",
        })
}

fn home_relative_path(path: &Path, home_dir: &Path) -> String {
    match path.strip_prefix(home_dir) {
        Ok(rel) => format!("$HOME/{}", rel.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Build the managed profile body under `~/.shine/shell/`.
/// User shell config files source this file from a small sentinel block.
pub(super) fn managed_profile_snippet(
    shell: &ShellType,
    bin_dir: &Path,
    home_dir: &Path,
    source_commands: &[String],
) -> String {
    let bin_str = home_relative_path(bin_dir, home_dir);
    let mut body = match shell {
        ShellType::Fish => format!("fish_add_path \"{bin_str}\""),
        ShellType::PowerShell => powershell_path_snippet(&bin_str),
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
            ShellType::PowerShell => {
                let script_name = if cfg!(windows) {
                    format!("{cmd}.ps1")
                } else {
                    cmd.clone()
                };
                body.push_str(&format!(
                    "\nfunction {cmd} {{ . (Join-Path $shineBin '{}') @args }}",
                    script_name.replace('\'', "''")
                ));
            }
            _ => {
                body.push_str(&format!(
                    "\n{cmd}() {{ source \"{bin_str}/{cmd}\" \"$@\"; }}"
                ));
            }
        }
    }
    if let Some(snippet) = completion_registration_snippet(shell) {
        body.push_str(snippet);
    }
    format!("{body}\n")
}

fn completion_registration_snippet(shell: &ShellType) -> Option<&'static str> {
    match shell {
        ShellType::Bash => {
            Some("\nif command -v shine >/dev/null 2>&1; then\n  source <(COMPLETE=bash shine)\nfi")
        }
        ShellType::Zsh => Some(
            "\nif command -v shine >/dev/null 2>&1; then\n  if (( ! $+functions[compdef] )); then\n    autoload -Uz compinit\n    compinit -i\n  fi\n  source <(COMPLETE=zsh shine)\nfi",
        ),
        ShellType::PowerShell => Some(
            "\nif (Get-Command shine -ErrorAction SilentlyContinue) { $env:COMPLETE = 'powershell'; shine | Out-String | Invoke-Expression; Remove-Item Env:\\COMPLETE -ErrorAction SilentlyContinue }",
        ),
        ShellType::Fish | ShellType::Elvish => None,
    }
}

pub(super) fn shell_config_snippet(
    shell: &ShellType,
    profile_path: &Path,
    home_dir: &Path,
) -> String {
    let profile_str = home_relative_path(profile_path, home_dir);
    let body = match shell {
        ShellType::PowerShell => format!(". {}", powershell_path_expr(&profile_str)),
        _ => format!("source {}", shell_quote_expand_home(&profile_str)),
    };
    format!("{SENTINEL_START}\n{body}\n{SENTINEL_END}\n")
}

fn powershell_path_snippet(bin_str: &str) -> String {
    let assignment = powershell_bin_assignment(bin_str);
    format!(
        "{assignment}\n$shinePathEntries = $env:Path -split [System.IO.Path]::PathSeparator\nif ($shinePathEntries -notcontains $shineBin) {{\n  $env:Path = \"$shineBin$([System.IO.Path]::PathSeparator)$env:Path\"\n}}"
    )
}

pub(super) fn powershell_bin_assignment(bin_str: &str) -> String {
    let bin_str = crate::path_display::strip_windows_verbatim_prefix(bin_str);
    let normalized = bin_str.replace('\\', "/");
    if let Some(rel) = normalized.strip_prefix("$HOME/") {
        let escaped = rel.replace('\'', "''");
        format!("$shineBin = Join-Path $HOME '{escaped}'")
    } else if normalized == "$HOME" {
        "$shineBin = $HOME".to_string()
    } else {
        let escaped = bin_str.replace('\'', "''");
        format!("$shineBin = '{escaped}'")
    }
}

pub(super) fn shell_source_command(shell: &ShellType, config_path: &Path) -> String {
    match shell {
        ShellType::PowerShell => format!(". {}", powershell_quote(config_path)),
        _ => format!("source {}", shell_quote(config_path)),
    }
}

fn shell_quote(path: &Path) -> String {
    shell_quote_str(&path.display().to_string())
}

pub(super) fn powershell_quote(path: &Path) -> String {
    powershell_quote_str(&crate::path_display::strip_windows_verbatim_prefix(
        &path.display().to_string(),
    ))
}

fn shell_quote_str(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn powershell_quote_str(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn shell_quote_expand_home(value: &str) -> String {
    if value == "$HOME" || value.starts_with("$HOME/") {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        shell_quote_str(value)
    }
}

fn powershell_path_expr(value: &str) -> String {
    let value = crate::path_display::strip_windows_verbatim_prefix(value);
    let normalized = value.replace('\\', "/");
    if let Some(rel) = normalized.strip_prefix("$HOME/") {
        format!("(Join-Path $HOME '{}')", rel.replace('\'', "''"))
    } else if normalized == "$HOME" {
        "$HOME".to_string()
    } else {
        powershell_quote_str(&value)
    }
}

pub(super) fn print_source_command_activation_hint(
    config: &Config,
    shell_config_path: &Path,
    source_commands: &[String],
) {
    if source_commands.is_empty() {
        return;
    }

    output::hint_line(
        "Next Step",
        &format!(
            "run `{}` once, or open a new shell",
            shell_source_command(&config.shell_type, shell_config_path)
        ),
    );
    output::hint_line(
        "Commands",
        &format!("available after reload: {}", source_commands.join(", ")),
    );
}

/// Remove the shine sentinel block from `content`, including one preceding blank line.
pub(super) fn remove_sentinel_block(content: &str) -> String {
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

fn sentinel_block(content: &str) -> Option<&str> {
    let start = content.find(SENTINEL_START)?;
    let end = content[start..].find(SENTINEL_END)? + start + SENTINEL_END.len();
    Some(&content[start..end])
}

pub(super) async fn append_path_to_shell_config(
    config: &Config,
    force: bool,
    source_commands: &[String],
) -> Result<ShellConfigUpdate> {
    let profile_updated = write_managed_shell_profile(config, source_commands).await?;
    let config_paths = get_shell_config_paths(&config.shell_type, &config.home_dir)?;
    let mut updated_path = None;

    for config_path in config_paths {
        if append_path_to_single_shell_config(config, force, &config_path).await? {
            updated_path.get_or_insert(config_path);
        }
    }

    let config_status = match updated_path {
        Some(path) => PathUpdateStatus::Updated(path),
        None => PathUpdateStatus::AlreadyConfigured,
    };
    Ok(ShellConfigUpdate {
        profile_updated,
        config_status,
    })
}

pub(super) async fn write_managed_shell_profile(
    config: &Config,
    source_commands: &[String],
) -> Result<bool> {
    let profile_path = managed_shell_profile_path(config);
    if let Some(parent) = profile_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating managed shell profile dir: {parent:?}"))?;
    }

    let snippet = managed_profile_snippet(
        &config.shell_type,
        config.bin_dir(),
        &config.home_dir,
        source_commands,
    );
    let current = tokio::fs::read_to_string(&profile_path)
        .await
        .unwrap_or_default();
    if current == snippet {
        return Ok(false);
    }

    tokio::fs::write(&profile_path, snippet.as_bytes())
        .await
        .with_context(|| format!("writing managed shell profile: {}", profile_path.display()))?;
    Ok(true)
}

async fn append_path_to_single_shell_config(
    config: &Config,
    force: bool,
    config_path: &Path,
) -> Result<bool> {
    if let Some(parent) = config_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating directory for shell config: {parent:?}"))?;
    }

    let existing = tokio::fs::read_to_string(&config_path)
        .await
        .unwrap_or_default();
    let profile_path = managed_shell_profile_path(config);
    let snippet = shell_config_snippet(&config.shell_type, &profile_path, &config.home_dir);

    if let Some(existing_block) = sentinel_block(&existing) {
        let expected_block = snippet.trim_end_matches('\n');
        if !force && existing_block == expected_block {
            return Ok(false);
        }
        // Force or stale managed block: remove old sentinel block and re-add
        // the small entry that sources the shine-managed shell profile.
        let cleaned = remove_sentinel_block(&existing);
        tokio::fs::write(&config_path, cleaned.as_bytes())
            .await
            .with_context(|| format!("rewriting shell config: {config_path:?}"))?;
    }

    let existing = tokio::fs::read_to_string(&config_path)
        .await
        .unwrap_or_default();

    // Write the complete new content atomically so the file is closed (and thus
    // fully visible to subsequent reads) before this function returns.
    let new_content = format!("{existing}\n{snippet}");
    tokio::fs::write(&config_path, new_content.as_bytes())
        .await
        .with_context(|| format!("writing to shell config: {config_path:?}"))?;

    Ok(true)
}

pub(super) async fn remove_path_from_shell_config(config: &Config) -> Result<()> {
    let config_paths = get_shell_config_paths(&config.shell_type, &config.home_dir)?;

    for config_path in config_paths {
        if !config_path.exists() {
            continue;
        }

        let content = tokio::fs::read_to_string(&config_path)
            .await
            .with_context(|| format!("reading shell config: {config_path:?}"))?;

        if !content.contains(SENTINEL_START) {
            continue;
        }

        let cleaned = remove_sentinel_block(&content);
        tokio::fs::write(&config_path, cleaned.as_bytes())
            .await
            .with_context(|| format!("writing shell config: {config_path:?}"))?;

        println!(
            "Shell config ({}): shine entry removed",
            path_display::format_home(&config_path, &config.home_dir)
        );
    }
    Ok(())
}

pub(super) async fn remove_managed_shell_profile(config: &Config) -> Result<()> {
    let profile_path = managed_shell_profile_path(config);
    if !profile_path.exists() {
        return Ok(());
    }

    tokio::fs::remove_file(&profile_path)
        .await
        .with_context(|| format!("removing managed shell profile: {}", profile_path.display()))?;
    println!(
        "Shell profile ({}): removed",
        path_display::format_home(&profile_path, &config.home_dir)
    );

    if let Some(parent) = profile_path.parent() {
        let _ = tokio::fs::remove_dir(parent).await;
    }
    Ok(())
}
