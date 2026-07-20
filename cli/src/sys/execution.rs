use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Stdio;

use crate::colors;

use super::{
    ResolvedSelection, SYS_STATUS_PREFIX, SYS_UPDATE_PREFIX, SysAction, SysInitCommand, SysItem,
    SysItemOutcome, SysItemStatus, SysManifest, SysUpdateCheck, SysUpdateState,
    selection::format_item_ids,
};

/// The home directory to pass to sys scripts as `SHINE_TARGET_HOME`.
///
/// Uses the sudo-aware resolver so scripts see the invoking user's home under
/// `sudo`, not root's.
pub(super) fn target_home() -> std::ffi::OsString {
    crate::home::effective_home_dir().into_os_string()
}

pub(super) fn sys_init_command(os_id: &str) -> SysInitCommand {
    match os_id {
        "windows" => SysInitCommand {
            program: "powershell.exe",
            fixed_args: vec!["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"],
        },
        "macos" => SysInitCommand {
            program: "zsh",
            fixed_args: Vec::new(),
        },
        _ => SysInitCommand {
            program: "bash",
            fixed_args: Vec::new(),
        },
    }
}

pub(super) fn format_command_preview(
    command: &SysInitCommand,
    script_path: &Path,
    item_ids: &[String],
) -> String {
    let script = script_path.display().to_string();
    std::iter::once(command.program)
        .chain(command.fixed_args.iter().copied())
        .chain(std::iter::once(script.as_str()))
        .chain(item_ids.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn manifest_item_labels(manifest: &SysManifest) -> BTreeMap<&str, String> {
    manifest
        .items
        .iter()
        .map(|item| (item.id.as_str(), item.label.clone()))
        .collect()
}

/// Width of the widest selected item label (or "profile", whichever is longer),
/// used to align the status column when printing per-item outcomes.
pub(super) fn sys_item_label_width(
    selection: &ResolvedSelection,
    item_labels: &BTreeMap<&str, String>,
) -> usize {
    selection
        .item_ids
        .iter()
        .filter_map(|item_id| item_labels.get(item_id.as_str()))
        .map(String::len)
        .chain(std::iter::once("profile".len()))
        .max()
        .unwrap_or(14)
        .max(14)
}

pub(super) fn print_run_header(os_id: &str, sys_shell: &str, selection: &ResolvedSelection) {
    println!("{}", colors::bold("System Init"));
    println!("  OS: {os_id}");
    println!("  Shell: {sys_shell}");
    println!("  Selection: {}", selection.source.describe());
    println!("  Items: {} selected", selection.item_ids.len());
    println!("  {}", colors::dim(&format_item_ids(&selection.item_ids)));
    println!();
}

/// Build the HTTP proxy environment variables injected into init scripts by
/// `shine sys init --proxy`, from shine's preset proxy `[env]` values.
///
/// Reuses the `[env]` keys already consumed by the `proxy` shell preset
/// (`PROXY_HOST` / `HTTP_PROXY_PORT` / `PROXY_NO_PROXY`) and assembles the same
/// HTTP-form URL as `set_proxy.sh`. SOCKS5 is intentionally left out — winget
/// does not understand it, and it would add a live-socks-port dependency.
pub(super) fn proxy_env_vars(config: &crate::config::Config) -> Vec<(&'static str, String)> {
    let env = crate::env::EnvConfig::from_config(config);
    build_proxy_env_vars(
        env.get("PROXY_HOST").unwrap_or("127.0.0.1"),
        env.get("HTTP_PROXY_PORT").unwrap_or("6152"),
        env.get("PROXY_NO_PROXY")
            .unwrap_or("localhost,127.0.0.1,::1"),
    )
}

fn build_proxy_env_vars(host: &str, port: &str, no_proxy: &str) -> Vec<(&'static str, String)> {
    let url = format!("http://{host}:{port}");
    // Lower + upper case for both scheme-specific and all_proxy so curl, apt,
    // rustup, and Homebrew (macOS/Ubuntu) all pick it up regardless of which
    // form they read. winget (Windows) ignores these entirely, so we also expose
    // the URL as SHINE_SYS_PROXY — the explicit signal init.ps1 keys off to pass
    // `winget install --proxy <url>` (env-only proxying does not work for winget).
    vec![
        ("http_proxy", url.clone()),
        ("HTTP_PROXY", url.clone()),
        ("https_proxy", url.clone()),
        ("HTTPS_PROXY", url.clone()),
        ("all_proxy", url.clone()),
        ("ALL_PROXY", url.clone()),
        ("no_proxy", no_proxy.to_string()),
        ("NO_PROXY", no_proxy.to_string()),
        ("SHINE_SYS_PROXY", url),
    ]
}

pub(super) async fn run_sys_item(
    command: &SysInitCommand,
    script_dir: &Path,
    script_path: &Path,
    sys_shell: &str,
    item_id: &str,
    label: &str,
    proxy_env: &[(&'static str, String)],
) -> Result<SysItemOutcome> {
    let mut cmd = tokio::process::Command::new(command.program);
    cmd.current_dir(script_dir)
        .env("SHINE_SYS_PRESET_ROOT", script_dir)
        .env("SHINE_SYS_SHELL", sys_shell);
    for (key, value) in proxy_env {
        cmd.env(key, value);
    }
    let output = cmd
        .args(&command.fixed_args)
        .arg(script_path)
        .arg(item_id)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("failed to execute {}", script_path.display()))?;

    Ok(parse_sys_item_output(
        item_id,
        label,
        output.status.success(),
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
    ))
}

pub(super) async fn run_sys_item_action(
    command: &SysInitCommand,
    script_dir: &Path,
    script_path: &Path,
    sys_shell: &str,
    item: &SysItem,
    action: SysAction,
    env: &BTreeMap<String, String>,
) -> Result<SysItemOutcome> {
    let mut process = if item.requires_admin && cfg!(windows) {
        let mut process = tokio::process::Command::new("powershell.exe");
        let argument_list = std::iter::once("-NoProfile".to_string())
            .chain(std::iter::once("-ExecutionPolicy".to_string()))
            .chain(std::iter::once("Bypass".to_string()))
            .chain(std::iter::once("-File".to_string()))
            .chain(std::iter::once(script_path.display().to_string()))
            .chain(std::iter::once(item.id.clone()))
            .chain(std::iter::once(action.as_str().to_string()))
            .map(|arg| format!("'{}'", arg.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",");
        process.args([
            "-NoProfile",
            "-Command",
            &format!(
                "$p = Start-Process powershell.exe -Verb RunAs -Wait -PassThru -ArgumentList @({argument_list}); exit $p.ExitCode"
            ),
        ]);
        process
    } else if item.requires_admin {
        let mut process = tokio::process::Command::new("sudo");
        process
            .arg("-E")
            .arg(command.program)
            .args(&command.fixed_args)
            .arg(script_path)
            .arg(&item.id)
            .arg(action.as_str());
        process
    } else {
        let mut process = tokio::process::Command::new(command.program);
        process
            .args(&command.fixed_args)
            .arg(script_path)
            .arg(&item.id)
            .arg(action.as_str());
        process
    };

    process
        .current_dir(script_dir)
        .env("SHINE_SYS_PRESET_ROOT", script_dir)
        .env("SHINE_SYS_SHELL", sys_shell)
        .env("SHINE_TARGET_HOME", target_home());
    for key in &item.required_env {
        if let Some(value) = env.get(key) {
            process.env(key, value);
        }
    }

    let output = process
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("failed to execute {}", script_path.display()))?;

    Ok(parse_sys_item_output(
        &item.id,
        &item.label,
        output.status.success(),
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
    ))
}

/// Run a platform-owned update check. This path intentionally never uses sudo,
/// elevation, proxy setup, or a manifest write: scripts may inspect their source
/// but must only print an upstream command for the user to run.
pub(super) async fn run_sys_update_check(
    command: &SysInitCommand,
    script_dir: &Path,
    script_path: &Path,
    sys_shell: &str,
    item_id: &str,
    label: &str,
    proxy_env: &[(&'static str, String)],
) -> Result<SysUpdateCheck> {
    let mut process = tokio::process::Command::new(command.program);
    process
        .current_dir(script_dir)
        .env("SHINE_SYS_PRESET_ROOT", script_dir)
        .env("SHINE_SYS_SHELL", sys_shell)
        .env("SHINE_TARGET_HOME", target_home())
        .args(&command.fixed_args)
        .arg(script_path)
        .arg(item_id)
        .arg("check-update")
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in proxy_env {
        process.env(key, value);
    }
    let output = process
        .output()
        .await
        .with_context(|| format!("failed to execute {}", script_path.display()))?;
    Ok(parse_sys_update_output(
        item_id,
        label,
        output.status.success(),
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
    ))
}

pub(super) fn parse_sys_update_output(
    item_id: &str,
    label: &str,
    success: bool,
    stdout: &str,
    stderr: &str,
) -> SysUpdateCheck {
    let mut event = None;
    let mut logs = Vec::new();
    for line in stdout.lines().chain(stderr.lines()) {
        if let Some(parsed) = parse_update_event(line) {
            event = Some(parsed);
        } else if !line.trim().is_empty() {
            logs.push(line.to_string());
        }
    }
    let (state, detail, upgrade_command) = if !success {
        let detail = event
            .as_ref()
            .map(|(_, detail, _)| detail.clone())
            .filter(|detail| !detail.is_empty())
            .unwrap_or_else(|| "update checker exited with a non-zero status".to_string());
        (SysUpdateState::Failed, detail, String::new())
    } else if let Some(event) = event {
        event
    } else {
        (
            SysUpdateState::Failed,
            "update checker emitted no valid update event".to_string(),
            String::new(),
        )
    };
    SysUpdateCheck {
        item_id: item_id.to_string(),
        label: label.to_string(),
        state,
        detail,
        upgrade_command,
        logs,
    }
}

pub(super) fn parse_update_event(line: &str) -> Option<(SysUpdateState, String, String)> {
    let mut parts = line.strip_prefix(SYS_UPDATE_PREFIX)?.splitn(3, '\t');
    let state = match parts.next()? {
        "available" => SysUpdateState::Available,
        "current" => SysUpdateState::Current,
        "manual" => SysUpdateState::Manual,
        "unsupported" => SysUpdateState::Unsupported,
        "failed" => SysUpdateState::Failed,
        _ => return None,
    };
    let detail = normalize_status_detail(parts.next().unwrap_or_default());
    let upgrade_command = parts.next().unwrap_or_default().trim().to_string();
    Some((state, detail, upgrade_command))
}

pub(super) fn parse_sys_item_output(
    item_id: &str,
    label: &str,
    success: bool,
    stdout: &str,
    stderr: &str,
) -> SysItemOutcome {
    let mut status = None;
    let mut logs = Vec::new();

    for line in stdout.lines().chain(stderr.lines()) {
        if let Some(parsed) = parse_status_event(line) {
            status = Some(parsed);
        } else if !line.trim().is_empty() {
            logs.push(line.to_string());
        }
    }

    let (status, detail) = if !success {
        let detail = status
            .map(|(_, detail)| detail)
            .filter(|detail| !detail.is_empty())
            .unwrap_or_else(|| "script exited with a non-zero status".to_string());
        (SysItemStatus::Failed, detail)
    } else if let Some((status, detail)) = status {
        (status, detail)
    } else {
        (SysItemStatus::Completed, String::new())
    };

    SysItemOutcome {
        item_id: item_id.to_string(),
        label: label.to_string(),
        status,
        detail,
        logs,
    }
}

pub(super) fn parse_status_event(line: &str) -> Option<(SysItemStatus, String)> {
    let rest = line.strip_prefix(SYS_STATUS_PREFIX)?;
    let mut parts = rest.splitn(2, '\t');
    let status = match parts.next()? {
        "installed" => SysItemStatus::Installed,
        "already-installed" => SysItemStatus::AlreadyInstalled,
        "skipped" => SysItemStatus::Skipped,
        "updated" => SysItemStatus::Updated,
        "needs-action" => SysItemStatus::NeedsAction,
        "completed" => SysItemStatus::Completed,
        "failed" => SysItemStatus::Failed,
        _ => return None,
    };
    let detail = normalize_status_detail(parts.next().unwrap_or_default().trim());
    Some((status, detail))
}

fn normalize_status_detail(detail: &str) -> String {
    detail
        .strip_suffix(" ()")
        .unwrap_or(detail)
        .trim()
        .to_string()
}

pub(super) fn print_item_outcome(outcome: &SysItemOutcome, label_width: usize) {
    let symbol = status_symbol(outcome.status);
    let label = format!("{:<label_width$}", outcome.label);
    let status = format!("{:<17}", status_text(outcome.status));
    let detail = if outcome.detail.is_empty() {
        String::new()
    } else {
        colors::dim(&outcome.detail)
    };

    println!(
        "  {} {} {} {}",
        colors::symbol(symbol),
        colors::bold(&label),
        colors::status_label(&status, symbol),
        detail
    );

    for line in &outcome.logs {
        println!("    {}", colors::dim(line));
    }
}

pub(super) fn status_symbol(status: SysItemStatus) -> &'static str {
    match status {
        SysItemStatus::Skipped | SysItemStatus::NeedsAction => "~",
        SysItemStatus::Failed => "✗",
        _ => "✓",
    }
}

pub(super) fn status_text(status: SysItemStatus) -> &'static str {
    match status {
        SysItemStatus::Installed => "installed",
        SysItemStatus::AlreadyInstalled => "already installed",
        SysItemStatus::Skipped => "skipped",
        SysItemStatus::Updated => "updated",
        SysItemStatus::NeedsAction => "needs action",
        SysItemStatus::Completed => "completed",
        SysItemStatus::Failed => "failed",
    }
}

pub(super) fn print_sys_summary(outcomes: &[SysItemOutcome]) {
    let mut counts = BTreeMap::<SysItemStatus, usize>::new();
    for outcome in outcomes {
        *counts.entry(outcome.status).or_default() += 1;
    }

    let parts = [
        SysItemStatus::Installed,
        SysItemStatus::AlreadyInstalled,
        SysItemStatus::Skipped,
        SysItemStatus::Updated,
        SysItemStatus::NeedsAction,
        SysItemStatus::Completed,
        SysItemStatus::Failed,
    ]
    .into_iter()
    .filter_map(|status| {
        counts
            .get(&status)
            .copied()
            .filter(|count| *count > 0)
            .map(|count| format!("{count} {}", status_text(status)))
    })
    .collect::<Vec<_>>();

    println!("Summary: {}", parts.join(", "));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::env_lock;

    #[test]
    fn target_home_uses_effective_home_dir_not_raw_home_env() {
        let _guard = env_lock();
        let old_home = std::env::var_os("HOME");
        let old_sudo_user = std::env::var_os("SUDO_USER");

        unsafe {
            std::env::set_var("HOME", "/tmp/shine-fake-home");
            std::env::remove_var("SUDO_USER");
        }

        assert_eq!(
            target_home(),
            crate::home::effective_home_dir().into_os_string()
        );

        unsafe {
            match old_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match old_sudo_user {
                Some(value) => std::env::set_var("SUDO_USER", value),
                None => std::env::remove_var("SUDO_USER"),
            }
        }
    }

    #[test]
    fn build_proxy_env_vars_assembles_http_form_lower_and_upper() {
        let vars = build_proxy_env_vars("127.0.0.1", "6152", "localhost");
        assert_eq!(vars.len(), 9);

        let get = |key: &str| {
            vars.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("http_proxy"), Some("http://127.0.0.1:6152"));
        assert_eq!(get("HTTP_PROXY"), Some("http://127.0.0.1:6152"));
        assert_eq!(get("https_proxy"), Some("http://127.0.0.1:6152"));
        assert_eq!(get("HTTPS_PROXY"), Some("http://127.0.0.1:6152"));
        assert_eq!(get("all_proxy"), Some("http://127.0.0.1:6152"));
        assert_eq!(get("ALL_PROXY"), Some("http://127.0.0.1:6152"));
        assert_eq!(get("no_proxy"), Some("localhost"));
        assert_eq!(get("NO_PROXY"), Some("localhost"));
        // Explicit signal init.ps1 reads to pass `winget install --proxy <url>`.
        assert_eq!(get("SHINE_SYS_PROXY"), Some("http://127.0.0.1:6152"));
    }
}
