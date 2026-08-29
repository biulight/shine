use crate::runtime::{CoreRuntime, FileSystemHost, ShellType};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub const SHELL_SENTINEL_START: &str = "# >>> shine >>>";
pub const SHELL_SENTINEL_END: &str = "# <<< shine <<<";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PathUpdateStatus {
    AlreadyConfigured,
    Updated(PathBuf),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShellConfigUpdate {
    pub profile_updated: bool,
    pub config_status: PathUpdateStatus,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ShellProfileRemoval {
    pub config_paths: Vec<PathBuf>,
    pub managed_profile: Option<PathBuf>,
}

pub fn managed_shell_profile_path(shine_dir: &Path, shell: ShellType) -> PathBuf {
    shine_dir.join("shell").join(match shell {
        ShellType::Fish => "config.fish",
        ShellType::PowerShell => "profile.ps1",
        _ => "profile.sh",
    })
}

fn home_relative_path(path: &Path, home_dir: &Path) -> String {
    match path.strip_prefix(home_dir) {
        Ok(relative) => format!("$HOME/{}", relative.display()),
        Err(_) => path.display().to_string(),
    }
}

pub fn managed_profile_snippet(
    shell: ShellType,
    bin_dir: &Path,
    home_dir: &Path,
    source_commands: &[String],
) -> String {
    let bin = home_relative_path(bin_dir, home_dir);
    let mut body = match shell {
        ShellType::Fish => format!("fish_add_path \"{bin}\""),
        ShellType::PowerShell => powershell_path_snippet(&bin),
        _ => format!(
            "if [[ \":$PATH:\" != *\":{bin}:\"* ]]; then\n  export PATH=\"{bin}:$PATH\"\nfi"
        ),
    };
    for command in source_commands {
        match shell {
            ShellType::Fish => body.push_str(&format!(
                "\nfunction {command}\n  source \"{bin}/{command}\" $argv\nend"
            )),
            ShellType::PowerShell => {
                let script = if cfg!(windows) {
                    format!("{command}.ps1")
                } else {
                    command.clone()
                };
                body.push_str(&format!(
                    "\nfunction {command} {{ . (Join-Path $shineBin '{}') @args }}",
                    script.replace('\'', "''")
                ));
            }
            _ => body.push_str(&format!(
                "\n{command}() {{ source \"{bin}/{command}\" \"$@\"; }}"
            )),
        }
    }
    if let Some(completion) = completion_registration_snippet(shell) {
        body.push_str(completion);
    }
    format!("{body}\n")
}

fn completion_registration_snippet(shell: ShellType) -> Option<&'static str> {
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

pub fn supports_completion_registration(shell: ShellType) -> bool {
    completion_registration_snippet(shell).is_some()
}

pub fn shell_config_snippet(shell: ShellType, profile_path: &Path, home_dir: &Path) -> String {
    let profile = home_relative_path(profile_path, home_dir);
    let body = match shell {
        ShellType::PowerShell => format!(". {}", powershell_path_expr(&profile)),
        _ => format!("source {}", shell_quote_expand_home(&profile)),
    };
    format!("{SHELL_SENTINEL_START}\n{body}\n{SHELL_SENTINEL_END}\n")
}

fn powershell_path_snippet(bin: &str) -> String {
    let assignment = powershell_bin_assignment(bin);
    format!(
        "{assignment}\n$shinePathEntries = $env:Path -split [System.IO.Path]::PathSeparator\nif ($shinePathEntries -notcontains $shineBin) {{\n  $env:Path = \"$shineBin$([System.IO.Path]::PathSeparator)$env:Path\"\n}}"
    )
}

pub fn powershell_bin_assignment(bin: &str) -> String {
    let bin = strip_windows_verbatim_prefix(bin);
    let normalized = bin.replace('\\', "/");
    if let Some(relative) = normalized.strip_prefix("$HOME/") {
        format!(
            "$shineBin = Join-Path $HOME '{}'",
            relative.replace('\'', "''")
        )
    } else if normalized == "$HOME" {
        "$shineBin = $HOME".to_string()
    } else {
        format!("$shineBin = '{}'", bin.replace('\'', "''"))
    }
}

pub fn shell_source_command(shell: ShellType, path: &Path) -> String {
    match shell {
        ShellType::PowerShell => format!(". {}", powershell_quote(path)),
        _ => format!("source {}", shell_quote(path)),
    }
}

fn shell_quote(path: &Path) -> String {
    single_quote(&path.display().to_string())
}

pub fn powershell_quote(path: &Path) -> String {
    format!(
        "'{}'",
        strip_windows_verbatim_prefix(&path.display().to_string()).replace('\'', "''")
    )
}

fn single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn shell_quote_expand_home(value: &str) -> String {
    if value == "$HOME" || value.starts_with("$HOME/") {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        single_quote(value)
    }
}

fn powershell_path_expr(value: &str) -> String {
    let value = strip_windows_verbatim_prefix(value);
    let normalized = value.replace('\\', "/");
    if let Some(relative) = normalized.strip_prefix("$HOME/") {
        format!("(Join-Path $HOME '{}')", relative.replace('\'', "''"))
    } else if normalized == "$HOME" {
        "$HOME".to_string()
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}

fn sentinel() -> crate::sentinel::Sentinel<'static> {
    crate::sentinel::Sentinel {
        start: SHELL_SENTINEL_START,
        end: SHELL_SENTINEL_END,
    }
}

pub fn remove_shell_sentinel(content: &str) -> String {
    crate::sentinel::remove_block_bytewise(content, &sentinel())
}

fn shell_sentinel_block(content: &str) -> Option<&str> {
    crate::sentinel::find_block(content, &sentinel())
}

impl<H: FileSystemHost> CoreRuntime<H> {
    pub async fn install_shell_profile(
        &self,
        config_paths: &[PathBuf],
        force: bool,
        source_commands: &[String],
    ) -> Result<ShellConfigUpdate> {
        let profile_updated = self.write_shell_profile(source_commands).await?;
        let mut updated_path = None;
        for path in config_paths {
            if self
                .append_shell_config(path, force)
                .await
                .with_context(|| format!("updating shell config {}", path.display()))?
            {
                updated_path.get_or_insert_with(|| path.clone());
            }
        }
        Ok(ShellConfigUpdate {
            profile_updated,
            config_status: updated_path.map_or(
                PathUpdateStatus::AlreadyConfigured,
                PathUpdateStatus::Updated,
            ),
        })
    }

    pub async fn write_shell_profile(&self, source_commands: &[String]) -> Result<bool> {
        let path = managed_shell_profile_path(&self.context.shine_dir, self.context.shell);
        let desired = managed_profile_snippet(
            self.context.shell,
            &self.context.bin_dir,
            &self.context.home_dir,
            source_commands,
        );
        let current = match self.host.read(&path).await {
            Ok(bytes) => bytes,
            Err(error) if error.is_not_found() => Vec::new(),
            Err(error) => return Err(error.into_anyhow("reading managed shell profile")),
        };
        if current == desired.as_bytes() {
            return Ok(false);
        }
        self.host
            .write_atomic(&path, desired.as_bytes())
            .await
            .map_err(|error| error.into_anyhow("writing managed shell profile"))?;
        Ok(true)
    }

    async fn append_shell_config(&self, path: &Path, force: bool) -> Result<bool> {
        let existing = match self.host.read(path).await {
            Ok(bytes) => String::from_utf8(bytes).context("shell config is not UTF-8")?,
            Err(error) if error.is_not_found() => String::new(),
            Err(error) => return Err(error.into_anyhow("reading shell config")),
        };
        let profile = managed_shell_profile_path(&self.context.shine_dir, self.context.shell);
        let snippet = shell_config_snippet(self.context.shell, &profile, &self.context.home_dir);
        if let Some(block) = shell_sentinel_block(&existing)
            && !force
            && block == snippet.trim_end_matches('\n')
        {
            return Ok(false);
        }
        let cleaned = remove_shell_sentinel(&existing);
        let desired = format!("{cleaned}\n{snippet}");
        self.host
            .write_atomic(path, desired.as_bytes())
            .await
            .map_err(|error| error.into_anyhow("writing shell config"))?;
        Ok(true)
    }

    pub async fn remove_shell_profile(
        &self,
        config_paths: &[PathBuf],
    ) -> Result<ShellProfileRemoval> {
        let mut removal = ShellProfileRemoval {
            config_paths: self.remove_shell_config_blocks(config_paths).await?,
            managed_profile: None,
        };
        removal.managed_profile = self.remove_managed_shell_profile().await?;
        Ok(removal)
    }

    pub async fn remove_shell_config_blocks(
        &self,
        config_paths: &[PathBuf],
    ) -> Result<Vec<PathBuf>> {
        let mut removed = Vec::new();
        for path in config_paths {
            let content = match self.host.read(path).await {
                Ok(bytes) => String::from_utf8(bytes).context("shell config is not UTF-8")?,
                Err(error) if error.is_not_found() => continue,
                Err(error) => return Err(error.into_anyhow("reading shell config")),
            };
            if !content.contains(SHELL_SENTINEL_START) {
                continue;
            }
            let cleaned = remove_shell_sentinel(&content);
            self.host
                .write_atomic(path, cleaned.as_bytes())
                .await
                .map_err(|error| error.into_anyhow("writing shell config"))?;
            removed.push(path.clone());
        }
        Ok(removed)
    }

    pub async fn remove_managed_shell_profile(&self) -> Result<Option<PathBuf>> {
        let profile = managed_shell_profile_path(&self.context.shine_dir, self.context.shell);
        match self.host.metadata(&profile).await {
            Ok(_) => {
                self.host
                    .remove_file(&profile)
                    .await
                    .map_err(|error| error.into_anyhow("removing managed shell profile"))?;
                Ok(Some(profile))
            }
            Err(error) if error.is_not_found() => Ok(None),
            Err(error) => Err(error.into_anyhow("inspecting managed shell profile")),
        }
    }
}

fn strip_windows_verbatim_prefix(value: &str) -> String {
    value
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .or_else(|| value.strip_prefix(r"\\?\").map(str::to_string))
        .unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytewise_removal_preserves_crlf_and_preceding_blank_line_contract() {
        let input = "keep\r\n\r\n# >>> shine >>>\r\nsource x\r\n# <<< shine <<<\r\nafter\r\n";
        assert_eq!(remove_shell_sentinel(input), "keep\r\n\r\nafter\r\n");
    }
}
