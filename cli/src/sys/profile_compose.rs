use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::path::Path;

use crate::config::Config;

use super::{
    LoadedSysPreset, SysManifest, SysProfilePhase, SysShellIntegration, SysShellKind,
    bootstrap::require_external_code_permission,
};

pub(super) struct ComposedSysProfiles {
    pub(super) pre: Vec<u8>,
    pub(super) post: Vec<u8>,
}

pub(super) async fn compose_sys_profiles(
    config: &Config,
    os_id: &str,
    loaded: &LoadedSysPreset,
    enabled_items: &BTreeSet<String>,
    sys_shell: &str,
) -> Result<ComposedSysProfiles> {
    let shell = SysShellKind::from_runtime(sys_shell)
        .with_context(|| format!("unsupported shell for composed sys profile: {sys_shell}"))?;
    let mut pre = read_base_profile(config, os_id, SysProfilePhase::Pre).await?;
    let mut post = read_base_profile(config, os_id, SysProfilePhase::Post).await?;

    let mut integrations = Vec::new();
    for (item_order, item) in loaded.manifest.items.iter().enumerate() {
        if !enabled_items.contains(&item.id) {
            continue;
        }
        for (integration_order, integration) in item.shell.iter().enumerate() {
            if integration.shells.contains(&shell) {
                integrations.push((
                    integration.phase,
                    integration.priority,
                    item_order,
                    integration_order,
                    item.id.as_str(),
                    integration,
                ));
            }
        }
    }
    integrations.sort_by_key(|(phase, priority, item_order, integration_order, _, _)| {
        (*phase, *priority, *item_order, *integration_order)
    });

    for (phase, _, _, _, item_id, integration) in integrations {
        let rendered = render_integration(config, os_id, item_id, integration, shell).await?;
        let target = if phase == SysProfilePhase::Pre {
            &mut pre
        } else {
            &mut post
        };
        append_section(target, item_id, &rendered);
    }

    Ok(ComposedSysProfiles { pre, post })
}

pub(super) fn enabled_profile_items(
    manifest: &SysManifest,
    entries: &[super::run_manifest::SysRunEntry],
    os_id: &str,
) -> BTreeSet<String> {
    entries
        .iter()
        .filter(|entry| {
            entry.os_id == os_id
                && !entry.managed
                && entry.profile_enabled
                && manifest
                    .items
                    .iter()
                    .any(|item| item.id == entry.item_id && !item.shell.is_empty())
        })
        .map(|entry| entry.item_id.clone())
        .collect()
}

async fn read_base_profile(
    config: &Config,
    os_id: &str,
    phase: SysProfilePhase,
) -> Result<Vec<u8>> {
    let ext = if os_id == "windows" { "ps1" } else { "sh" };
    let relative = Path::new("sys")
        .join(os_id)
        .join("profile")
        .join(format!("base.{}.{ext}", phase.as_str()));
    let path = config.preset_path(&relative);
    if path.is_file() {
        require_external_code_permission(config, &path, "base profile")?;
        return tokio::fs::read(&path)
            .await
            .with_context(|| format!("reading {}", path.display()));
    }
    if !config.is_external_presets {
        let asset = relative.to_string_lossy().replace('\\', "/");
        if let Some(bytes) = crate::presets::read_asset_bytes(&asset) {
            return Ok(bytes);
        }
    }
    Ok(Vec::new())
}

async fn render_integration(
    config: &Config,
    os_id: &str,
    item_id: &str,
    integration: &SysShellIntegration,
    shell: SysShellKind,
) -> Result<String> {
    let executable = !integration.eval_argv.is_empty()
        || integration.source.is_some()
        || integration.fragment.is_some();
    if executable
        && (config.is_external_presets || config.active_presets_overlay_dir().is_some())
        && !config.allow_sys_code
    {
        bail!("external sys profile code for `{item_id}` is disabled; set `allow_sys_code = true`");
    }

    if let Some(fragment) = &integration.fragment {
        let relative = Path::new("sys").join(os_id).join(fragment);
        let path = config.preset_path(&relative);
        let body = if path.is_file() {
            require_external_code_permission(config, &path, "profile fragment")?;
            tokio::fs::read_to_string(&path)
                .await
                .with_context(|| format!("reading {}", path.display()))?
        } else if !config.is_external_presets {
            let asset = relative.to_string_lossy().replace('\\', "/");
            if let Some(bytes) = crate::presets::read_asset_bytes(&asset) {
                String::from_utf8(bytes)
                    .with_context(|| format!("embedded profile fragment `{asset}` is not UTF-8"))?
            } else {
                bail!("sys profile fragment is missing: {}", path.display());
            }
        } else {
            bail!("sys profile fragment is missing: {}", path.display());
        };
        return Ok(guard_body(body, integration.when_command.as_deref(), shell));
    }

    let body = match shell {
        SysShellKind::Bash | SysShellKind::Zsh => render_posix(integration, shell)?,
        SysShellKind::Powershell => render_powershell(integration)?,
    };
    Ok(guard_body(body, integration.when_command.as_deref(), shell))
}

fn render_posix(integration: &SysShellIntegration, shell: SysShellKind) -> Result<String> {
    if let Some(path) = &integration.path {
        let expr = posix_path_expr(path)?;
        return Ok(format!(
            "case \":$PATH:\" in\n  *\":{expr}:\"*) ;;\n  *) export PATH={expr}:\"$PATH\" ;;\nesac\n"
        ));
    }
    if !integration.env.is_empty() {
        return integration
            .env
            .iter()
            .map(|(key, value)| Ok(format!("export {key}={}\n", posix_value(value)?)))
            .collect::<Result<String>>();
    }
    if !integration.eval_argv.is_empty() {
        let shell_name = shell.as_str();
        let argv = integration
            .eval_argv
            .iter()
            .map(|arg| posix_quote(&arg.replace("{shell}", shell_name)))
            .collect::<Vec<_>>()
            .join(" ");
        return Ok(format!("eval \"$({argv})\"\n"));
    }
    if let Some(source) = &integration.source {
        let source = posix_value(source)?;
        return Ok(format!(
            "if [[ -f {source} ]]; then\n  source {source}\nfi\n"
        ));
    }
    if !integration.aliases.is_empty() {
        return Ok(integration
            .aliases
            .iter()
            .map(|(name, value)| format!("alias {name}={}\n", posix_quote(value)))
            .collect());
    }
    bail!("empty POSIX sys shell integration")
}

fn render_powershell(integration: &SysShellIntegration) -> Result<String> {
    if let Some(path) = &integration.path {
        let value = powershell_value(path);
        return Ok(format!(
            "$shineSysPath = {value}\nif (-not (($env:PATH -split ';') -contains $shineSysPath)) {{\n    $env:PATH = \"$shineSysPath;$env:PATH\"\n}}\n"
        ));
    }
    if !integration.env.is_empty() {
        return Ok(integration
            .env
            .iter()
            .map(|(key, value)| format!("$env:{key} = {}\n", powershell_value(value)))
            .collect());
    }
    if !integration.eval_argv.is_empty() {
        let argv = integration
            .eval_argv
            .iter()
            .map(|arg| powershell_quote(&arg.replace("{shell}", "pwsh")))
            .collect::<Vec<_>>();
        let (program, args) = argv
            .split_first()
            .context("profile eval requires a program")?;
        return Ok(format!(
            "Invoke-Expression ((& {program} {}) | Out-String)\n",
            args.join(" ")
        ));
    }
    if let Some(source) = &integration.source {
        let source = powershell_value(source);
        return Ok(format!(
            "$shineSysSource = {source}\nif (Test-Path -LiteralPath $shineSysSource) {{ . $shineSysSource }}\n"
        ));
    }
    if !integration.aliases.is_empty() {
        bail!("PowerShell aliases with arguments require an item-owned fragment");
    }
    bail!("empty PowerShell sys shell integration")
}

fn guard_body(body: String, command: Option<&str>, shell: SysShellKind) -> String {
    let Some(command) = command else {
        return body;
    };
    match shell {
        SysShellKind::Bash | SysShellKind::Zsh => format!(
            "if command -v {} >/dev/null 2>&1; then\n{}fi\n",
            posix_quote(command),
            indent(&body, "  ")
        ),
        SysShellKind::Powershell => format!(
            "if (Get-Command {} -ErrorAction SilentlyContinue) {{\n{}}}\n",
            powershell_quote(command),
            indent(&body, "    ")
        ),
    }
}

fn append_section(target: &mut Vec<u8>, item_id: &str, rendered: &str) {
    if !target.is_empty() && !target.ends_with(b"\n") {
        target.push(b'\n');
    }
    if !target.is_empty() {
        target.push(b'\n');
    }
    target.extend_from_slice(format!("# shine sys/{item_id}\n").as_bytes());
    target.extend_from_slice(rendered.as_bytes());
    if !target.ends_with(b"\n") {
        target.push(b'\n');
    }
}

fn posix_value(value: &str) -> Result<String> {
    if value == "$HOME" {
        return Ok("\"$HOME\"".to_string());
    }
    if let Some(suffix) = value.strip_prefix("$HOME/") {
        if suffix
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.'))
        {
            return Ok(format!("\"$HOME/{suffix}\""));
        }
        bail!("unsafe characters in $HOME-relative profile value");
    }
    Ok(posix_quote(value))
}

fn posix_path_expr(value: &str) -> Result<String> {
    posix_value(value)
}

fn posix_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn powershell_value(value: &str) -> String {
    if value == "$HOME" {
        "$HOME".to_string()
    } else if let Some(suffix) = value.strip_prefix("$HOME/") {
        format!("(Join-Path $HOME {})", powershell_quote(suffix))
    } else {
        powershell_quote(value)
    }
}

fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn indent(value: &str, prefix: &str) -> String {
    value
        .lines()
        .map(|line| format!("{prefix}{line}\n"))
        .collect()
}

impl SysShellKind {
    fn from_runtime(value: &str) -> Option<Self> {
        match value {
            "bash" => Some(Self::Bash),
            "zsh" => Some(Self::Zsh),
            "powershell" => Some(Self::Powershell),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Powershell => "powershell",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn integration() -> SysShellIntegration {
        SysShellIntegration {
            shells: vec![SysShellKind::Bash, SysShellKind::Zsh],
            phase: SysProfilePhase::Post,
            priority: 0,
            when_command: Some("mise".to_string()),
            path: None,
            env: BTreeMap::new(),
            eval_argv: vec![
                "mise".to_string(),
                "activate".to_string(),
                "{shell}".to_string(),
            ],
            source: None,
            aliases: BTreeMap::new(),
            fragment: None,
        }
    }

    #[test]
    fn renders_guarded_eval_with_runtime_shell() {
        let rendered = render_posix(&integration(), SysShellKind::Zsh).unwrap();
        let rendered = guard_body(rendered, Some("mise"), SysShellKind::Zsh);
        assert!(rendered.contains("command -v 'mise'"));
        assert!(rendered.contains("eval \"$('mise' 'activate' 'zsh')\""));
    }

    #[test]
    fn sections_are_byte_deterministic() {
        let mut first = b"base\n".to_vec();
        append_section(&mut first, "mise", "eval mise\n");
        let mut second = b"base\n".to_vec();
        append_section(&mut second, "mise", "eval mise\n");
        assert_eq!(first, second);
        assert_eq!(
            String::from_utf8(first).unwrap(),
            "base\n\n# shine sys/mise\neval mise\n"
        );
    }

    #[test]
    fn powershell_placeholder_uses_pwsh() {
        let mut value = integration();
        value.shells = vec![SysShellKind::Powershell];
        let rendered = render_powershell(&value).unwrap();
        assert!(rendered.contains("'mise' 'activate' 'pwsh'"));
    }
}
