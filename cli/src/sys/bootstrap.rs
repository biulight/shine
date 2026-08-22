use anyhow::{Context, Result, bail};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use crate::{colors, config::Config};

use super::{
    LoadedSysPreset, SysDetection, SysDetectionProbe, SysInstall, SysItem, SysItemOutcome,
    SysItemStatus, SysPackageProvider,
};

const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const MAX_LOG_BYTES: usize = 64 * 1024;

#[derive(Debug)]
enum DetectionResult {
    Present(String),
    Missing,
}

pub(super) async fn item_is_present(config: &Config, item: &SysItem) -> Result<bool> {
    let detect = item
        .detect
        .as_ref()
        .with_context(|| format!("sys item `{}` has no standard detection", item.id))?;
    Ok(matches!(
        detect_item(config, detect).await?,
        DetectionResult::Present(_)
    ))
}

pub(super) async fn run_standard_bootstrap_item(
    config: &Config,
    os_id: &str,
    loaded: &LoadedSysPreset,
    item: &SysItem,
    sys_shell: &str,
    proxy_env: &[(&'static str, String)],
) -> Result<SysItemOutcome> {
    let Some(detect) = item.detect.as_ref() else {
        bail!("sys item `{}` has no standard detection", item.id);
    };
    let Some(install) = item.install.as_ref() else {
        bail!("sys item `{}` has no standard installer", item.id);
    };

    if let DetectionResult::Present(detail) = detect_item(config, detect).await? {
        return Ok(outcome(
            item,
            SysItemStatus::AlreadyInstalled,
            detail,
            Vec::new(),
        ));
    }

    if let SysInstall::Script { path, .. } = install {
        let script_path = resolve_preset_file(loaded, path)?;
        require_external_code_permission(config, &script_path, "install script")?;
    }
    super::execution::print_item_install_start(item, install_requires_admin(os_id, install, item)?);

    let execution = match install {
        SysInstall::Package {
            provider, package, ..
        } => run_package_provider(os_id, *provider, package, proxy_env).await?,
        SysInstall::Script { path, .. } => {
            let script_path = resolve_preset_file(loaded, path)?;
            run_item_script(
                config,
                os_id,
                &script_path,
                &loaded.root,
                sys_shell,
                item,
                proxy_env,
            )
            .await?
        }
    };

    if !execution.success {
        return Ok(outcome(
            item,
            SysItemStatus::Failed,
            execution.failure_detail,
            execution.logs,
        ));
    }

    let detected = detect_item(config, detect).await?;
    if matches!(detected, DetectionResult::Missing) {
        return Ok(outcome(
            item,
            SysItemStatus::Failed,
            "installer succeeded but the declared detection is still missing".to_string(),
            execution.logs,
        ));
    }

    let status = install.success_status();
    let mut detail = install.success_hint().to_string();
    if detail.is_empty()
        && let DetectionResult::Present(detected_detail) = detected
    {
        detail = detected_detail;
    }

    Ok(outcome(item, status, detail, execution.logs))
}

fn install_requires_admin(os_id: &str, install: &SysInstall, item: &SysItem) -> Result<bool> {
    match install {
        SysInstall::Package {
            provider, package, ..
        } => {
            let (_, _, requires_admin) = package_command(os_id, *provider, package, &[])?;
            Ok(requires_admin)
        }
        SysInstall::Script { .. } => Ok(item.requires_admin && os_id != "windows"),
    }
}

pub(super) fn standard_install_preview(
    config: &Config,
    os_id: &str,
    loaded: &LoadedSysPreset,
    item: &SysItem,
) -> Result<String> {
    let Some(install) = item.install.as_ref() else {
        bail!("sys item `{}` has no standard installer", item.id);
    };
    match install {
        SysInstall::Package {
            provider, package, ..
        } => package_preview(os_id, *provider, package),
        SysInstall::Script { path, .. } => {
            let script_path = resolve_preset_file(loaded, path)?;
            let permission = if (config.is_external_presets
                || config
                    .active_presets_overlay_dir()
                    .is_some_and(|overlay| script_path.starts_with(overlay)))
                && !config.allow_sys_code
            {
                " (requires allow_sys_code = true)"
            } else {
                ""
            };
            Ok(format!("{}{}", script_path.display(), permission))
        }
    }
}

pub(super) fn preflight_standard_bootstrap_item(
    config: &Config,
    loaded: &LoadedSysPreset,
    item: &SysItem,
) -> Result<()> {
    if let Some(SysInstall::Script { path, .. }) = &item.install {
        let script_path = resolve_preset_file(loaded, path)?;
        if !script_path.is_file() {
            bail!(
                "sys item `{}` install script is missing: {}",
                item.id,
                script_path.display()
            );
        }
        require_external_code_permission(config, &script_path, "install script")?;
    }
    Ok(())
}

pub(super) fn require_external_code_permission(
    config: &Config,
    path: &Path,
    label: &str,
) -> Result<()> {
    let overlay_code = config
        .active_presets_overlay_dir()
        .is_some_and(|overlay| path.starts_with(overlay));
    if (config.is_external_presets || overlay_code) && !config.allow_sys_code {
        return Err(external_code_permission_error(
            config,
            &format!("executable sys {label}"),
            Some(path),
        ));
    }
    Ok(())
}

pub(super) fn external_code_permission_error(
    config: &Config,
    capability: &str,
    code_path: Option<&Path>,
) -> anyhow::Error {
    let overlay = config.active_presets_overlay_dir();
    let (reason, remediation) = match (config.is_external_presets, overlay.is_some()) {
        (true, true) => (
            "an external preset source and preset overlay are active",
            "Disable both the external preset source and preset overlay",
        ),
        (true, false) => (
            "an external preset source is active",
            "Disable the external preset source",
        ),
        (false, true) => ("a preset overlay is active", "Disable the preset overlay"),
        (false, false) => {
            unreachable!("external code permission error created without an external source")
        }
    };
    let mut source_details = String::new();
    if config.is_external_presets {
        source_details.push_str(&format!(
            "Preset source:  {}\n",
            config.presets_dir().display()
        ));
    }
    if let Some(path) = overlay {
        source_details.push_str(&format!("Preset overlay: {}\n", path.display()));
    }
    if let Some(path) = code_path {
        source_details.push_str(&format!("Code path:      {}\n", path.display()));
    }
    let allow_setting = colors::bold_yellow_stderr("allow_sys_code = true");
    let global_config = colors::cyan_stderr(&config.global_config_path().display().to_string());
    let keep_blocked = colors::yellow_stderr(remediation);

    anyhow::anyhow!(
        "{capability} is blocked because {reason}.\n\n\
{source_details}\
After reviewing the active preset sources, choose one:\n\n\
  Allow external sys code:\n\
    Set {allow_setting} in {global_config}\n\n\
  Keep external sys code blocked:\n\
    {keep_blocked}."
    )
}

fn resolve_preset_file(loaded: &LoadedSysPreset, relative: &str) -> Result<PathBuf> {
    let candidate = loaded.root.join(relative);
    let canonical = std::fs::canonicalize(&candidate)
        .with_context(|| format!("resolving sys preset resource {}", candidate.display()))?;
    if !canonical.starts_with(&loaded.root) {
        bail!("sys preset path escapes its root: {relative}");
    }
    Ok(canonical)
}

async fn detect_item(config: &Config, detect: &SysDetection) -> Result<DetectionResult> {
    match detect {
        SysDetection::Command {
            command,
            version_args,
        } => detect_command(config, command, version_args).await,
        SysDetection::Path { path } => {
            let expanded = crate::home::full_expand_with_home(path, &config.home_dir)
                .with_context(|| format!("expanding sys detection path `{path}`"))?;
            Ok(if Path::new(&expanded).exists() {
                DetectionResult::Present(expanded)
            } else {
                DetectionResult::Missing
            })
        }
        SysDetection::Any { probes } => {
            for probe in probes {
                let result = match probe {
                    SysDetectionProbe::Command { command } => {
                        detect_command(config, command, &[]).await?
                    }
                    SysDetectionProbe::Path { path } => {
                        let expanded = crate::home::full_expand_with_home(path, &config.home_dir)
                            .with_context(|| {
                            format!("expanding sys detection path `{path}`")
                        })?;
                        if Path::new(&expanded).exists() {
                            DetectionResult::Present(expanded)
                        } else {
                            DetectionResult::Missing
                        }
                    }
                };
                if matches!(result, DetectionResult::Present(_)) {
                    return Ok(result);
                }
            }
            Ok(DetectionResult::Missing)
        }
    }
}

async fn detect_command(
    config: &Config,
    command: &str,
    version_args: &[String],
) -> Result<DetectionResult> {
    let Some(program) = find_command(&config.home_dir, command) else {
        return Ok(DetectionResult::Missing);
    };
    if version_args.is_empty() {
        return Ok(DetectionResult::Present(program.display().to_string()));
    }
    let output = tokio::process::Command::new(&program)
        .args(version_args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output()
        .await;
    let detail = output
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            String::from_utf8(output.stdout)
                .ok()
                .and_then(|stdout| stdout.lines().next().map(str::to_string))
        })
        .filter(|line| !line.trim().is_empty())
        .unwrap_or_else(|| program.display().to_string());
    Ok(DetectionResult::Present(detail))
}

fn find_command(home_dir: &Path, command: &str) -> Option<PathBuf> {
    let mut directories = std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default();
    directories.extend([
        home_dir.join(".local/bin"),
        home_dir.join(".cargo/bin"),
        home_dir.join(".bun/bin"),
        home_dir.join(".local/share/pnpm"),
        home_dir.join("AppData/Local/Microsoft/WinGet/Links"),
    ]);
    directories.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
    ]);
    for directory in directories {
        let candidate = directory.join(command);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            for extension in ["exe", "cmd", "bat", "ps1"] {
                let candidate = directory.join(format!("{command}.{extension}"));
                if is_executable_file(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    true
}

struct ExecutionResult {
    success: bool,
    failure_detail: String,
    logs: Vec<String>,
}

async fn run_package_provider(
    os_id: &str,
    provider: SysPackageProvider,
    package: &str,
    proxy_env: &[(&'static str, String)],
) -> Result<ExecutionResult> {
    let (program, args, requires_admin) = package_command(os_id, provider, package, proxy_env)?;
    if requires_admin && !crate::privilege::ensure_admin(1).await? {
        return Ok(ExecutionResult {
            success: false,
            failure_detail: "administrator authorization was not granted".to_string(),
            logs: Vec::new(),
        });
    }

    let mut command = if requires_admin {
        let mut command = crate::install_core::file_ops::sudo_command();
        if !proxy_env.is_empty() {
            command.arg("env");
            for (key, value) in proxy_env {
                command.arg(format!("{key}={value}"));
            }
        }
        command.arg(&program);
        command
    } else {
        tokio::process::Command::new(&program)
    };
    command
        .args(&args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (key, value) in proxy_env {
        command.env(key, value);
    }
    let mut execution = run_command(command, format!("{provider:?} install failed")).await?;
    if !execution.success && provider == SysPackageProvider::Winget {
        execution.failure_detail.push_str(
            "; retry from an elevated PowerShell if the package requires administrator access",
        );
    }
    Ok(execution)
}

fn package_preview(os_id: &str, provider: SysPackageProvider, package: &str) -> Result<String> {
    let (program, args, requires_admin) = package_command(os_id, provider, package, &[])?;
    let prefix = if requires_admin { "sudo " } else { "" };
    Ok(format!(
        "{prefix}{} {}",
        program.to_string_lossy(),
        args.join(" ")
    ))
}

fn package_command(
    os_id: &str,
    provider: SysPackageProvider,
    package: &str,
    proxy_env: &[(&'static str, String)],
) -> Result<(OsString, Vec<String>, bool)> {
    match provider {
        SysPackageProvider::Homebrew | SysPackageProvider::HomebrewCask => {
            if os_id == "windows" {
                bail!("Homebrew sys provider is unavailable on Windows");
            }
            let program = [
                PathBuf::from("/opt/homebrew/bin/brew"),
                PathBuf::from("/usr/local/bin/brew"),
                PathBuf::from("/home/linuxbrew/.linuxbrew/bin/brew"),
            ]
            .into_iter()
            .find(|path| is_executable_file(path))
            .map(PathBuf::into_os_string)
            .unwrap_or_else(|| OsString::from("brew"));
            let mut args = vec!["install".to_string()];
            if provider == SysPackageProvider::HomebrewCask {
                args.push("--cask".to_string());
            }
            args.push(package.to_string());
            Ok((program, args, false))
        }
        SysPackageProvider::Apt => {
            if os_id != "ubuntu" {
                bail!("APT sys provider is supported only on Ubuntu presets");
            }
            Ok((
                OsString::from("apt-get"),
                vec!["install".to_string(), "-y".to_string(), package.to_string()],
                true,
            ))
        }
        SysPackageProvider::Winget => {
            if os_id != "windows" {
                bail!("Winget sys provider is supported only on Windows presets");
            }
            let mut args = vec![
                "install".to_string(),
                "--exact".to_string(),
                "--id".to_string(),
                package.to_string(),
                "--accept-package-agreements".to_string(),
                "--accept-source-agreements".to_string(),
            ];
            if let Some((_, proxy)) = proxy_env.iter().find(|(key, _)| *key == "SHINE_SYS_PROXY") {
                args.extend(["--proxy".to_string(), proxy.clone()]);
            }
            Ok((OsString::from("winget"), args, false))
        }
    }
}

async fn run_item_script(
    config: &Config,
    os_id: &str,
    script_path: &Path,
    preset_root: &Path,
    sys_shell: &str,
    item: &SysItem,
    proxy_env: &[(&'static str, String)],
) -> Result<ExecutionResult> {
    if !script_path.is_file() {
        bail!(
            "sys item `{}` install script is missing: {}",
            item.id,
            script_path.display()
        );
    }
    let (program, fixed_args): (&str, &[&str]) = if os_id == "windows" {
        (
            "powershell.exe",
            &["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"],
        )
    } else if os_id == "macos" {
        ("zsh", &[])
    } else {
        ("bash", &[])
    };
    let requires_unix_admin = item.requires_admin && os_id != "windows";
    if requires_unix_admin && !crate::privilege::ensure_admin(1).await? {
        return Ok(ExecutionResult {
            success: false,
            failure_detail: "administrator authorization was not granted".to_string(),
            logs: Vec::new(),
        });
    }

    let mut env_values = vec![
        (
            "SHINE_SYS_PRESET_ROOT".to_string(),
            preset_root.as_os_str().to_string_lossy().into_owned(),
        ),
        ("SHINE_SYS_SHELL".to_string(), sys_shell.to_string()),
        (
            "SHINE_TARGET_HOME".to_string(),
            config.home_dir.as_os_str().to_string_lossy().into_owned(),
        ),
    ];
    for key in &item.required_env {
        if let Some(value) = config.env.get(key) {
            env_values.push((key.clone(), value.clone()));
        }
    }
    env_values.extend(
        proxy_env
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone())),
    );

    let mut command = if requires_unix_admin {
        let mut command = crate::install_core::file_ops::sudo_command();
        command.arg(format!(
            "--preserve-env={}",
            env_values
                .iter()
                .map(|(key, _)| key.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ));
        command.arg(program);
        command
    } else {
        tokio::process::Command::new(program)
    };
    command
        .args(fixed_args)
        .arg(script_path)
        .current_dir(script_path.parent().unwrap_or_else(|| Path::new(".")))
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (key, value) in &env_values {
        command.env(key, value);
    }
    let mut execution = run_command(
        command,
        format!("sys item `{}` install script failed", item.id),
    )
    .await?;
    if !execution.success && item.requires_admin && os_id == "windows" {
        execution
            .failure_detail
            .push_str("; retry from an elevated PowerShell");
    }
    Ok(execution)
}

async fn run_command(
    mut command: tokio::process::Command,
    failure_prefix: String,
) -> Result<ExecutionResult> {
    let output = match tokio::time::timeout(BOOTSTRAP_TIMEOUT, command.output()).await {
        Ok(output) => output.context("running sys bootstrap installer")?,
        Err(_) => {
            return Ok(ExecutionResult {
                success: false,
                failure_detail: format!("{failure_prefix}: timed out after 30 minutes"),
                logs: Vec::new(),
            });
        }
    };
    let logs = bounded_logs(&output.stdout, &output.stderr);
    let failure_detail = if output.status.success() {
        String::new()
    } else {
        format!(
            "{failure_prefix} (exit {})",
            output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string())
        )
    };
    Ok(ExecutionResult {
        success: output.status.success(),
        failure_detail,
        logs,
    })
}

fn bounded_logs(stdout: &[u8], stderr: &[u8]) -> Vec<String> {
    let mut combined = Vec::new();
    combined.extend_from_slice(stdout);
    if !stdout.is_empty() && !stderr.is_empty() {
        combined.push(b'\n');
    }
    combined.extend_from_slice(stderr);
    let truncated = combined.len() > MAX_LOG_BYTES;
    if truncated {
        combined = combined[combined.len() - MAX_LOG_BYTES..].to_vec();
    }
    let mut logs = String::from_utf8_lossy(&combined)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if truncated {
        logs.insert(
            0,
            "installer output truncated at 64 KiB; showing final output".to_string(),
        );
    }
    logs
}

fn outcome(
    item: &SysItem,
    status: SysItemStatus,
    detail: String,
    logs: Vec<String>,
) -> SysItemOutcome {
    SysItemOutcome {
        item_id: item.id.clone(),
        label: format!("sys/{}", item.id),
        status,
        detail,
        logs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::manifest::parse_and_validate_manifest;
    use tokio::fs;

    #[test]
    fn package_commands_are_fixed_argv() {
        let (program, args, admin) =
            package_command("ubuntu", SysPackageProvider::Apt, "fzf", &[]).unwrap();
        assert_eq!(program, OsString::from("apt-get"));
        assert_eq!(args, ["install", "-y", "fzf"]);
        assert!(admin);

        let (_, args, admin) = package_command(
            "windows",
            SysPackageProvider::Winget,
            "jdx.mise",
            &[("SHINE_SYS_PROXY", "http://127.0.0.1:7890".to_string())],
        )
        .unwrap();
        assert_eq!(
            args,
            [
                "install",
                "--exact",
                "--id",
                "jdx.mise",
                "--accept-package-agreements",
                "--accept-source-agreements",
                "--proxy",
                "http://127.0.0.1:7890",
            ]
        );
        assert!(!admin);
    }

    #[test]
    fn install_admin_requirement_includes_packages_and_script_metadata() {
        let manifest = parse_and_validate_manifest(
            r#"
version = 2

[[items]]
id = "apt-tool"
label = "APT tool"
detect = { kind = "command", command = "apt-tool" }
install = { kind = "package", provider = "apt", package = "apt-tool" }

[[items]]
id = "script-tool"
label = "Script tool"
requires_admin = true
detect = { kind = "command", command = "script-tool" }
install = { kind = "script", path = "install/script-tool.sh" }
"#,
        )
        .unwrap();

        assert!(
            install_requires_admin(
                "ubuntu",
                manifest.items[0].install.as_ref().unwrap(),
                &manifest.items[0]
            )
            .unwrap()
        );
        assert!(
            install_requires_admin(
                "ubuntu",
                manifest.items[1].install.as_ref().unwrap(),
                &manifest.items[1]
            )
            .unwrap()
        );
        assert!(
            !install_requires_admin(
                "windows",
                manifest.items[1].install.as_ref().unwrap(),
                &manifest.items[1]
            )
            .unwrap()
        );
    }

    #[test]
    fn bounded_logs_truncates_combined_output() {
        let stdout = vec![b'x'; MAX_LOG_BYTES + 1];
        let logs = bounded_logs(&stdout, b"error");
        assert_eq!(
            logs.first().unwrap(),
            "installer output truncated at 64 KiB; showing final output"
        );
    }

    #[test]
    fn permission_error_identifies_all_external_layers_and_config() {
        let dir =
            std::env::temp_dir().join(format!("shine-sys-permission-{}", uuid::Uuid::new_v4()));
        let overlay = dir.join("overlay");
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        let config = config.with_presets_overlay_dir_override(Some(overlay.clone()));
        let code_path = config.presets_dir().join("sys/ubuntu/init.sh");

        let error =
            require_external_code_permission(&config, &code_path, "legacy bootstrap script")
                .unwrap_err();
        let message = error.to_string();

        assert!(message.contains(
            "executable sys legacy bootstrap script is blocked because an external preset source and preset overlay are active"
        ));
        assert!(message.contains(&format!(
            "Preset source:  {}",
            config.presets_dir().display()
        )));
        assert!(message.contains(&format!("Preset overlay: {}", overlay.display())));
        assert!(message.contains(&format!("Code path:      {}", code_path.display())));
        assert!(message.contains("After reviewing the active preset sources, choose one:"));
        assert!(message.contains(&format!(
            "Set allow_sys_code = true in {}",
            config.global_config_path().display()
        )));
        assert!(message.contains("Keep external sys code blocked:"));
        assert!(message.contains("Disable both the external preset source and preset overlay."));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn per_item_script_uses_exit_code_and_post_detection() {
        let dir = crate::test_support::make_temp_dir("shine-sys-standard-script").await;
        let preset_root = dir.join("presets/sys/ubuntu");
        fs::create_dir_all(preset_root.join("install"))
            .await
            .unwrap();
        fs::write(
            preset_root.join("install/tool.sh"),
            r#"#!/bin/bash
set -euo pipefail
mkdir -p "$SHINE_TARGET_HOME/.local/bin"
printf '#!/bin/sh\n' > "$SHINE_TARGET_HOME/.local/bin/test-sys-tool"
chmod +x "$SHINE_TARGET_HOME/.local/bin/test-sys-tool"
"#,
        )
        .await
        .unwrap();
        let manifest = parse_and_validate_manifest(
            r#"
version = 2

[[items]]
id = "tool"
label = "Tool"

[items.detect]
kind = "command"
command = "test-sys-tool"

[items.install]
kind = "script"
path = "install/tool.sh"
"#,
        )
        .unwrap();
        let loaded = LoadedSysPreset {
            manifest,
            root: std::fs::canonicalize(&preset_root).unwrap(),
        };
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config.allow_sys_code = true;

        let outcome = run_standard_bootstrap_item(
            &config,
            "ubuntu",
            &loaded,
            &loaded.manifest.items[0],
            "bash",
            &[],
        )
        .await
        .unwrap();
        assert_eq!(outcome.status, SysItemStatus::Installed);
        assert!(dir.join(".local/bin/test-sys-tool").is_file());

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
