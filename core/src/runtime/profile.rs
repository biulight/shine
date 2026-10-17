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

pub(super) fn managed_profile_snippet(
    shell: ShellType,
    bin_dir: &Path,
    home_dir: &Path,
    source_commands: &[String],
) -> String {
    let bin = home_relative_path(bin_dir, home_dir);
    let quoted_bin = escape_shell_path_double_quoted(&bin, shell);
    let mut body = match shell {
        ShellType::Fish => format!("fish_add_path \"{quoted_bin}\""),
        ShellType::PowerShell => powershell_path_snippet(&bin),
        _ => format!(
            "if [[ \":$PATH:\" != *\":{quoted_bin}:\"* ]]; then\n  export PATH=\"{quoted_bin}:$PATH\"\nfi"
        ),
    };
    for command in source_commands {
        match shell {
            ShellType::Fish => body.push_str(&format!(
                "\nfunction {command}\n  source \"{quoted_bin}/{command}\" $argv\nend"
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
                "\n{command}() {{ source \"{quoted_bin}/{command}\" \"$@\"; }}"
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

pub(super) fn shell_config_snippet(
    shell: ShellType,
    profile_path: &Path,
    home_dir: &Path,
) -> String {
    let profile = home_relative_path(profile_path, home_dir);
    let body = match shell {
        ShellType::PowerShell => format!(". {}", powershell_path_expr(&profile)),
        _ => format!("source {}", shell_quote_expand_home(&profile, shell)),
    };
    format!("{SHELL_SENTINEL_START}\n{body}\n{SHELL_SENTINEL_END}\n")
}

fn powershell_path_snippet(bin: &str) -> String {
    let assignment = powershell_bin_assignment(bin);
    format!(
        "{assignment}\n$shinePathEntries = $env:Path -split [System.IO.Path]::PathSeparator\nif ($shinePathEntries -notcontains $shineBin) {{\n  $env:Path = \"$shineBin$([System.IO.Path]::PathSeparator)$env:Path\"\n}}"
    )
}

fn powershell_bin_assignment(bin: &str) -> String {
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

fn powershell_quote(path: &Path) -> String {
    format!(
        "'{}'",
        strip_windows_verbatim_prefix(&path.display().to_string()).replace('\'', "''")
    )
}

fn single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn escape_shell_path_double_quoted(value: &str, shell: ShellType) -> String {
    let (prefix, literal) = if value == "$HOME" {
        ("$HOME", "")
    } else if let Some(relative) = value.strip_prefix("$HOME/") {
        ("$HOME/", relative)
    } else {
        ("", value)
    };
    let mut escaped = literal
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$");
    if shell != ShellType::Fish {
        escaped = escaped.replace('`', "\\`");
    }
    format!("{prefix}{escaped}")
}

fn shell_quote_expand_home(value: &str, shell: ShellType) -> String {
    if value == "$HOME" || value.starts_with("$HOME/") {
        format!("\"{}\"", escape_shell_path_double_quoted(value, shell))
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

pub(super) fn remove_shell_sentinel(content: &str) -> String {
    crate::sentinel::remove_block_bytewise(content, &sentinel())
}

pub(crate) fn shell_sentinel_block(content: &str) -> Option<&str> {
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

    pub(in crate::runtime) async fn write_shell_profile(
        &self,
        source_commands: &[String],
    ) -> Result<bool> {
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

    pub(in crate::runtime) async fn remove_shell_profile(
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

    pub(in crate::runtime) async fn remove_managed_shell_profile(&self) -> Result<Option<PathBuf>> {
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
    fn managed_profile_uses_home_relative_bin_path() {
        let home = PathBuf::from("/home/user");
        let bin = home.join(".shine/bin");
        let snippet = managed_profile_snippet(ShellType::Zsh, &bin, &home, &[]);
        assert!(
            snippet.contains("$HOME/.shine/bin"),
            "should use $HOME: {snippet}"
        );
        assert!(!snippet.contains(SHELL_SENTINEL_START));
        assert!(!snippet.contains(SHELL_SENTINEL_END));
    }

    #[test]
    fn managed_profile_uses_absolute_bin_path_when_outside_home() {
        let home = PathBuf::from("/home/user");
        let bin = PathBuf::from("/opt/shine/bin");
        let snippet = managed_profile_snippet(ShellType::Zsh, &bin, &home, &[]);
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
        let snippet = managed_profile_snippet(ShellType::Fish, &bin, &home, &[]);
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
            let snippet = managed_profile_snippet(shell, &bin, &home, &[]);
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

        let powershell = managed_profile_snippet(ShellType::PowerShell, &bin, &home, &[]);
        assert!(
            powershell.contains("$env:COMPLETE = 'powershell'"),
            "PowerShell should register shine completion: {powershell}"
        );

        let fish = managed_profile_snippet(ShellType::Fish, &bin, &home, &[]);
        assert!(
            !fish.contains("COMPLETE=fish shine"),
            "fish completion should not be changed: {fish}"
        );
        assert!(supports_completion_registration(ShellType::PowerShell));
        assert!(!supports_completion_registration(ShellType::Fish));
        assert!(!supports_completion_registration(ShellType::Elvish));
    }

    #[test]
    fn snippet_source_commands_generate_wrapper_functions() {
        let home = PathBuf::from("/home/user");
        let bin = home.join(".shine/bin");
        let cmds = vec!["setproxy".to_string(), "usetproxy".to_string()];
        for shell in [ShellType::Bash, ShellType::Zsh] {
            let snippet = managed_profile_snippet(shell, &bin, &home, &cmds);
            assert!(
                snippet.contains("setproxy() { source"),
                "{shell:?} should have setproxy wrapper: {snippet}"
            );
            assert!(
                snippet.contains("usetproxy() { source"),
                "{shell:?} should have usetproxy wrapper: {snippet}"
            );
        }
        let fish_snippet = managed_profile_snippet(ShellType::Fish, &bin, &home, &cmds);
        assert!(
            fish_snippet.contains("function setproxy"),
            "fish should have setproxy function: {fish_snippet}"
        );
        let powershell_snippet = managed_profile_snippet(ShellType::PowerShell, &bin, &home, &cmds);
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
        let snippet = shell_config_snippet(ShellType::Zsh, &profile, &home);
        assert!(snippet.contains(SHELL_SENTINEL_START));
        assert!(snippet.contains("source \"$HOME/.shine/shell/profile.sh\""));
        assert!(!snippet.contains("export PATH"));
        assert!(!snippet.contains("function setproxy"));
    }

    #[test]
    fn powershell_paths_strip_windows_verbatim_prefix() {
        let assignment = powershell_bin_assignment(r"\\?\D:\Github\Biulight\shine\.shine\bin");
        assert!(assignment.contains(r"D:\Github\Biulight\shine\.shine\bin"));
        assert!(!assignment.contains(r"\\?\"));

        let quoted = powershell_quote(Path::new(r"\\?\D:\Github\Biulight\shine\profile.ps1"));
        assert_eq!(quoted, r"'D:\Github\Biulight\shine\profile.ps1'");
    }

    #[test]
    fn remove_sentinel_block_strips_block_and_blank_line() {
        let content = "before\n\n# >>> shine >>>\nexport PATH\n# <<< shine <<<\nafter\n";
        let cleaned = remove_shell_sentinel(content);
        assert_eq!(cleaned, "before\nafter\n");
    }

    #[test]
    fn remove_sentinel_block_no_op_when_absent() {
        let content = "no sentinel here\n";
        let cleaned = remove_shell_sentinel(content);
        assert_eq!(cleaned, content);
    }
    #[test]
    fn remove_sentinel_block_preserves_existing_bytewise_contract() {
        assert_eq!(
            remove_shell_sentinel("keep\n\n# >>> shine >>>\nsource x\n# <<< shine <<<\nafter\n"),
            "keep\nafter\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn fish_paths_escape_expansion_without_adding_backslashes_to_backticks() {
        let home = std::env::temp_dir().join("shine-fish-home");
        let path = home.join("$literal`name").join("bin");
        let snippet = managed_profile_snippet(ShellType::Fish, &path, &home, &[]);
        assert_eq!(snippet, "fish_add_path \"$HOME/\\$literal`name/bin\"\n");
        let snippet = shell_config_snippet(ShellType::Fish, &path, &home);
        assert!(snippet.contains("source \"$HOME/\\$literal`name/bin\""));
    }

    #[cfg(unix)]
    #[test]
    fn generated_profiles_treat_shell_metacharacters_as_literal_paths() {
        let home = std::env::temp_dir().join(format!("shine-profile-{}", uuid::Uuid::new_v4()));
        let literal = "space ' \" $literal `printf injected` $(printf injected) \\";
        for root in [home.clone(), home.with_extension("external")] {
            let bin = root.join(literal).join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::write(bin.join("review_command"), "printf '%s' command-ok").unwrap();
            let snippet =
                managed_profile_snippet(ShellType::Bash, &bin, &home, &["review_command".into()]);
            let output = std::process::Command::new("bash")
                .args([
                    "--noprofile",
                    "--norc",
                    "-c",
                    &format!("{snippet}\nprintf '%s\\n' \"$PATH\"; review_command"),
                ])
                .env("HOME", &home)
                .env("PATH", "/usr/bin:/bin")
                .env_remove("literal")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8(output.stdout).unwrap(),
                format!("{}:/usr/bin:/bin\ncommand-ok", bin.display())
            );

            let profile = root.join(literal).join("profile.sh");
            std::fs::write(&profile, "printf '%s' profile-ok").unwrap();
            let snippet = shell_config_snippet(ShellType::Bash, &profile, &home);
            let output = std::process::Command::new("bash")
                .args(["--noprofile", "--norc", "-c", &snippet])
                .env("HOME", &home)
                .env_remove("literal")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(output.stdout, b"profile-ok");
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn bytewise_removal_preserves_crlf_and_preceding_blank_line_contract() {
        let input = "keep\r\n\r\n# >>> shine >>>\r\nsource x\r\n# <<< shine <<<\r\nafter\r\n";
        assert_eq!(remove_shell_sentinel(input), "keep\r\n\r\nafter\r\n");
    }
}
