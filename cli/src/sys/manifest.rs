use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::path::Path;

use crate::config::Config;

use super::{LoadedSysPreset, SysDriverKind, SysItem, SysItemMode, SysManifest};

pub(super) async fn load_sys_preset(config: &Config, os_id: &str) -> Result<LoadedSysPreset> {
    if os_id.contains('/') || os_id.contains('\\') || os_id.contains("..") {
        bail!("invalid os id: {os_id:?}");
    }
    let prefix = format!("sys/{os_id}");
    if !config.is_external_presets {
        crate::presets::extract_prefix(&prefix, config.presets_dir(), true).await?;
    }

    let root = Path::new("sys").join(os_id);
    let script_path = config.preset_path(root.join(sys_init_script_name(os_id)));
    if !script_path.exists() {
        bail!(
            "No init script found for '{}'. Expected: {}",
            os_id,
            script_path.display()
        );
    }

    let manifest_path = config.preset_path(root.join("shine.toml"));
    let content = tokio::fs::read_to_string(&manifest_path)
        .await
        .with_context(|| format!("reading {}", manifest_path.display()))?;
    let manifest = parse_and_validate_manifest(&content)
        .with_context(|| format!("parsing {}", manifest_path.display()))?;

    Ok(LoadedSysPreset {
        manifest,
        script_path,
    })
}

pub(super) fn sys_init_script_name(os_id: &str) -> &'static str {
    if os_id == "windows" {
        "init.ps1"
    } else {
        "init.sh"
    }
}

pub(super) fn parse_and_validate_manifest(content: &str) -> Result<SysManifest> {
    let manifest: SysManifest = toml::from_str(content)?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn validate_manifest(manifest: &SysManifest) -> Result<()> {
    let mut ids = BTreeSet::new();
    for item in &manifest.items {
        validate_item_id(&item.id)?;
        if item.label.trim().is_empty() {
            bail!("sys init item `{}` must have a label", item.id);
        }
        if !ids.insert(item.id.clone()) {
            bail!("duplicate sys init item id `{}`", item.id);
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

fn validate_driver_config(item: &SysItem) -> Result<()> {
    if item.mode == SysItemMode::Init && item.driver != SysDriverKind::Script {
        bail!(
            "sys init item `{}` cannot use managed driver `{:?}`",
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
        bail!("sys init item ids must not be empty");
    }
    if !item_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!(
            "sys init item id `{item_id}` contains invalid characters (allowed: a-z A-Z 0-9 - _)"
        );
    }
    Ok(())
}
