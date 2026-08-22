use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::path::Path;

use crate::config::Config;

use super::{
    LoadedSysPreset, SysDetection, SysDetectionProbe, SysDriverKind, SysInstall, SysItem,
    SysItemMode, SysItemStatus, SysManifest,
};

pub(super) async fn load_sys_preset(config: &Config, os_id: &str) -> Result<LoadedSysPreset> {
    if os_id.contains('/') || os_id.contains('\\') || os_id.contains("..") {
        bail!("invalid os id: {os_id:?}");
    }
    let prefix = format!("sys/{os_id}");
    if !config.is_external_presets {
        crate::presets::extract_prefix(&prefix, config.presets_dir(), true).await?;
    }

    let root = Path::new("sys").join(os_id);
    let preset_root = config.preset_path(&root);
    let manifest_path = preset_root.join("shine.toml");
    let content = tokio::fs::read_to_string(&manifest_path)
        .await
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest = parse_and_validate_manifest(&content)
        .with_context(|| format!("parsing {}", manifest_path.display()))?;
    Ok(LoadedSysPreset {
        manifest,
        root: std::fs::canonicalize(&preset_root)
            .with_context(|| format!("resolving sys preset root {}", preset_root.display()))?,
    })
}

pub(super) fn parse_and_validate_manifest(content: &str) -> Result<SysManifest> {
    let manifest: SysManifest = toml::from_str(content)?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &SysManifest) -> Result<()> {
    match manifest.version {
        Some(2) => {}
        None | Some(1) => bail!(
            "sys preset v1 is unsupported; migrate it to version = 2 by removing the monolithic dispatcher, adding detect/install to every init item, and moving software-specific profile code to item integrations. See docs/manual/guides/sys-preset-v2-migration.md"
        ),
        Some(version) => {
            bail!("sys preset version {version} is not supported by this Shine release")
        }
    }
    let mut ids = BTreeSet::new();
    for item in &manifest.items {
        validate_item_id(&item.id)?;
        if item.label.trim().is_empty() {
            bail!("sys bootstrap item `{}` must have a label", item.id);
        }
        if !ids.insert(item.id.clone()) {
            bail!("duplicate sys bootstrap item id `{}`", item.id);
        }
        let mut env_keys = BTreeSet::new();
        for key in &item.required_env {
            let mut chars = key.chars();
            let valid = chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !valid {
                bail!(
                    "sys item `{}` has invalid required_env key `{key}`",
                    item.id
                );
            }
            if !env_keys.insert(key) {
                bail!("sys item `{}` repeats required_env key `{key}`", item.id);
            }
        }
        validate_driver_config(item)?;
        validate_bootstrap_config(item)?;
        validate_shell_integrations(item)?;
    }

    if let Some(default_profile) = &manifest.default_profile
        && !manifest.profiles.contains_key(default_profile)
    {
        bail!("default profile `{default_profile}` is not defined");
    }

    for (profile_name, profile) in &manifest.profiles {
        for item_id in &profile.items {
            if !ids.contains(item_id) {
                bail!("profile `{profile_name}` references unknown item `{item_id}`");
            }
            if manifest
                .items
                .iter()
                .find(|item| item.id == *item_id)
                .is_some_and(|item| item.mode == SysItemMode::Managed)
            {
                bail!(
                    "profile `{profile_name}` references managed item `{item_id}`; enable it with `shine sys apply {item_id}`"
                );
            }
        }
    }

    Ok(())
}

fn validate_bootstrap_config(item: &SysItem) -> Result<()> {
    if item.mode == SysItemMode::Managed {
        if item.detect.is_some() || item.install.is_some() || !item.shell.is_empty() {
            bail!(
                "managed sys item `{}` cannot declare bootstrap detect/install/shell fields",
                item.id
            );
        }
        return Ok(());
    }

    let detect = item
        .detect
        .as_ref()
        .with_context(|| format!("sys bootstrap item `{}` must declare `detect`", item.id))?;
    let install = item
        .install
        .as_ref()
        .with_context(|| format!("sys bootstrap item `{}` must declare `install`", item.id))?;
    validate_detection(&item.id, detect)?;
    {
        match install {
            SysInstall::Package {
                package,
                success_status,
                success_hint,
                ..
            } => {
                validate_plain_value(&item.id, "package", package)?;
                if package.starts_with('-')
                    || !package
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
                {
                    bail!(
                        "sys item `{}` has invalid package identifier `{package}`",
                        item.id
                    );
                }
                validate_success_status(&item.id, *success_status)?;
                if !success_hint.is_empty() {
                    validate_plain_value(&item.id, "success hint", success_hint)?;
                }
            }
            SysInstall::Script {
                path,
                success_status,
                success_hint,
            } => {
                validate_relative_preset_path(&item.id, "install script", path)?;
                validate_success_status(&item.id, *success_status)?;
                if !success_hint.is_empty() {
                    validate_plain_value(&item.id, "success hint", success_hint)?;
                }
            }
        }
    }
    Ok(())
}

fn validate_detection(item_id: &str, detect: &SysDetection) -> Result<()> {
    match detect {
        SysDetection::Command {
            command,
            version_args,
        } => {
            validate_command_name(item_id, command)?;
            for arg in version_args {
                validate_plain_value(item_id, "version argument", arg)?;
            }
        }
        SysDetection::Path { path } => validate_plain_value(item_id, "detection path", path)?,
        SysDetection::Any { probes } => {
            if probes.is_empty() {
                bail!("sys item `{item_id}` detection `any` requires at least one probe");
            }
            for probe in probes {
                match probe {
                    SysDetectionProbe::Command { command } => {
                        validate_command_name(item_id, command)?
                    }
                    SysDetectionProbe::Path { path } => {
                        validate_plain_value(item_id, "detection path", path)?
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_success_status(item_id: &str, status: Option<SysItemStatus>) -> Result<()> {
    if status.is_some_and(|status| {
        !matches!(
            status,
            SysItemStatus::Installed | SysItemStatus::NeedsAction
        )
    }) {
        bail!("sys item `{item_id}` install success_status must be installed or needs-action");
    }
    Ok(())
}

fn validate_shell_integrations(item: &SysItem) -> Result<()> {
    for (index, integration) in item.shell.iter().enumerate() {
        if integration.shells.is_empty() {
            bail!(
                "sys item `{}` shell integration {} requires at least one shell",
                item.id,
                index + 1
            );
        }
        if let Some(command) = &integration.when_command {
            validate_command_name(&item.id, command)?;
        }
        let action_count = usize::from(integration.path.is_some())
            + usize::from(!integration.env.is_empty())
            + usize::from(!integration.eval_argv.is_empty())
            + usize::from(integration.source.is_some())
            + usize::from(!integration.aliases.is_empty())
            + usize::from(integration.fragment.is_some());
        if action_count != 1 {
            bail!(
                "sys item `{}` shell integration {} must declare exactly one of path, env, eval, source, aliases, or fragment",
                item.id,
                index + 1
            );
        }
        if let Some(path) = &integration.path {
            validate_plain_value(&item.id, "profile path", path)?;
        }
        for (key, value) in &integration.env {
            validate_env_key(&item.id, key)?;
            validate_plain_value(&item.id, "profile env value", value)?;
        }
        for arg in &integration.eval_argv {
            validate_plain_value(&item.id, "profile eval argument", arg)?;
        }
        if let Some(source) = &integration.source {
            validate_plain_value(&item.id, "profile source", source)?;
        }
        for (name, value) in &integration.aliases {
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
            {
                bail!("sys item `{}` has invalid alias name `{name}`", item.id);
            }
            validate_plain_value(&item.id, "profile alias", value)?;
        }
        if let Some(fragment) = &integration.fragment {
            validate_relative_preset_path(&item.id, "profile fragment", fragment)?;
        }
    }
    Ok(())
}

fn validate_command_name(item_id: &str, command: &str) -> Result<()> {
    if command.is_empty()
        || !command
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+'))
    {
        bail!("sys item `{item_id}` has invalid command name `{command}`");
    }
    Ok(())
}

fn validate_env_key(item_id: &str, key: &str) -> Result<()> {
    let mut chars = key.chars();
    let valid = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        bail!("sys item `{item_id}` has invalid profile env key `{key}`");
    }
    Ok(())
}

fn validate_plain_value(item_id: &str, label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        bail!("sys item `{item_id}` has invalid {label}");
    }
    Ok(())
}

fn validate_relative_preset_path(item_id: &str, label: &str, value: &str) -> Result<()> {
    validate_plain_value(item_id, label, value)?;
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        bail!("sys item `{item_id}` {label} must stay inside the preset: `{value}`");
    }
    Ok(())
}

fn validate_driver_config(item: &SysItem) -> Result<()> {
    if item.mode == SysItemMode::Init && item.driver != SysDriverKind::Script {
        bail!(
            "sys bootstrap item `{}` cannot use managed driver `{:?}`",
            item.id,
            item.driver
        );
    }
    let allowed: &[&str] = match item.driver {
        SysDriverKind::Script => &[],
        SysDriverKind::SplitDns => &["domain_env", "servers_env"],
        SysDriverKind::ManagedFile => &["source", "target", "transforms", "restart_hint"],
    };
    for key in item.config.keys() {
        if !allowed.contains(&key.as_str()) {
            bail!(
                "sys item `{}` has unknown {:?} driver config key `{key}`",
                item.id,
                item.driver
            );
        }
    }
    let require_string = |key: &str| -> Result<&str> {
        item.config
            .get(key)
            .and_then(toml::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .with_context(|| format!("sys item `{}` requires config `{key}`", item.id))
    };
    match item.driver {
        SysDriverKind::Script => {
            if !item.config.is_empty() {
                bail!(
                    "script sys item `{}` does not accept driver config",
                    item.id
                );
            }
        }
        SysDriverKind::SplitDns => {
            for key in ["domain_env", "servers_env"] {
                let env_key = require_string(key)?;
                if !item.required_env.iter().any(|required| required == env_key) {
                    bail!(
                        "sys item `{}` config `{key}` references `{env_key}` but required_env does not include it",
                        item.id
                    );
                }
            }
        }
        SysDriverKind::ManagedFile => {
            require_string("source")?;
            require_string("target")?;
            if let Some(transforms) = item.config.get("transforms") {
                let transforms = transforms.as_array().with_context(|| {
                    format!(
                        "sys item `{}` config `transforms` must be an array",
                        item.id
                    )
                })?;
                if transforms.iter().any(|value| value.as_str().is_none()) {
                    bail!(
                        "sys item `{}` config `transforms` must contain strings",
                        item.id
                    );
                }
            }
        }
    }
    Ok(())
}

fn validate_item_id(item_id: &str) -> Result<()> {
    if item_id.trim().is_empty() {
        bail!("sys bootstrap item ids must not be empty");
    }
    if !item_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!(
            "sys bootstrap item id `{item_id}` contains invalid characters (allowed: a-z A-Z 0-9 - _)"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_and_validate_manifest;

    const ITEM: &str = r#"
[[items]]
id = "tool"
label = "Tool"
detect = { kind = "command", command = "tool" }
install = { kind = "package", provider = "apt", package = "tool" }
"#;

    #[test]
    fn rejects_missing_or_unknown_sys_preset_versions() {
        let missing = parse_and_validate_manifest(ITEM).unwrap_err();
        assert!(missing.to_string().contains("v1 is unsupported"));
        let unknown = parse_and_validate_manifest(&format!("version = 3\n{ITEM}")).unwrap_err();
        assert!(unknown.to_string().contains("version 3 is not supported"));
    }

    #[test]
    fn v2_requires_detection_and_installer_for_init_items() {
        let missing_detect = parse_and_validate_manifest(
            "version = 2\n[[items]]\nid = 'tool'\nlabel = 'Tool'\ninstall = { kind = 'package', provider = 'apt', package = 'tool' }",
        )
        .unwrap_err();
        assert!(missing_detect.to_string().contains("must declare `detect`"));
        let missing_install = parse_and_validate_manifest(
            "version = 2\n[[items]]\nid = 'tool'\nlabel = 'Tool'\ndetect = { kind = 'command', command = 'tool' }",
        )
        .unwrap_err();
        assert!(
            missing_install
                .to_string()
                .contains("must declare `install`")
        );
    }

    #[test]
    fn built_in_sys_presets_are_all_executable_v2_manifests() {
        for manifest in [
            include_str!("../../../presets/sys/macos/shine.toml"),
            include_str!("../../../presets/sys/ubuntu/shine.toml"),
            include_str!("../../../presets/sys/windows/shine.toml"),
        ] {
            parse_and_validate_manifest(manifest).unwrap();
        }
    }

    #[test]
    fn built_in_ubuntu_all_profile_includes_bun() {
        let manifest =
            parse_and_validate_manifest(include_str!("../../../presets/sys/ubuntu/shine.toml"))
                .unwrap();

        assert!(manifest.items.iter().any(|item| item.id == "bun"));
        assert!(
            manifest.profiles["all"]
                .items
                .iter()
                .any(|item| item == "bun")
        );
        assert!(
            !manifest.profiles["recommended"]
                .items
                .iter()
                .any(|item| item == "bun")
        );
        assert!(!include_str!("../../../presets/sys/ubuntu/install/bun.sh").is_empty());
        assert!(!include_str!("../../../presets/sys/ubuntu/profile/bun.sh").is_empty());
    }
}
