//! Distribution adapter for the frontend-neutral Core runtime.
//!
//! `rust-embed` and CLI configuration stay in this crate. Every preset byte
//! and ambient path used by domain logic is captured before `CoreRuntime` is
//! constructed.

use crate::config::Config;
use anyhow::{Context, Result};
use directories::BaseDirs;
use std::path::Path;
use utils::runtime::{
    CoreRuntime, PresetSnapshot, PresetSourceKind, RealHost, RuntimeContext, RuntimePlatform,
};

pub(crate) fn from_config(config: &Config) -> Result<CoreRuntime<RealHost>> {
    from_config_with_preset_mode(config, config.is_external_presets)
}

pub(crate) fn from_installed_presets(config: &Config) -> Result<CoreRuntime<RealHost>> {
    from_config_with_preset_mode(config, true)
}

fn from_config_with_preset_mode(
    config: &Config,
    is_external_presets: bool,
) -> Result<CoreRuntime<RealHost>> {
    let bases = BaseDirs::new().context("resolving platform runtime directories")?;
    let mut builder = if is_external_presets {
        let mut builder = PresetSnapshot::builder(PresetSourceKind::External)
            .base_root(config.presets_dir().to_path_buf());
        for (logical, bytes) in collect_tree(config.presets_dir())? {
            builder = builder.file(logical, bytes);
        }
        builder
    } else {
        let mut builder = PresetSnapshot::builder(PresetSourceKind::Embedded);
        for logical in crate::presets::embedded_asset_paths("") {
            if let Some(bytes) = crate::presets::read_embedded_asset_bytes(&logical) {
                builder = builder.file(logical, bytes);
            }
        }
        builder
    };
    if let Some(overlay) = config.active_presets_overlay_dir() {
        builder = builder.overlay_root(overlay.to_path_buf());
        for (logical, bytes) in collect_tree(overlay)? {
            builder = builder.overlay_file(logical, bytes);
        }
    }
    let snapshot = builder.build();
    let context = RuntimeContext {
        home_dir: config.home_dir.clone(),
        shine_dir: config.shine_dir().to_path_buf(),
        presets_dir: config.presets_dir().to_path_buf(),
        bin_dir: config.bin_dir().to_path_buf(),
        cache_dir: bases.cache_dir().to_path_buf(),
        data_dir: bases.data_dir().to_path_buf(),
        app_default_dest_root: config.app_default_dest_root(),
        overlay_dir: config.active_presets_overlay_dir().map(Path::to_path_buf),
        platform: RuntimePlatform::current(),
        shell: config.shell_type,
        shell_config_paths: crate::shells::shell_config_paths_for_core(
            &config.shell_type,
            &config.home_dir,
        )?,
        external_shell_mode: config.external_shell_mode,
        is_external_presets,
        allow_app_hooks: config.allow_app_hooks,
        allow_sys_code: config.allow_sys_code,
        linux_split_dns_ready: systemd_resolved_stub_active(),
        running_as_admin: cfg!(windows) || std::env::var("USER").is_ok_and(|user| user == "root"),
        captured_unix_time: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        env: config.env.clone(),
        path_env: std::env::var("PATH").ok(),
        proxy_env: proxy_env(&config.env),
    };
    Ok(CoreRuntime::new(RealHost, context, snapshot))
}

pub(crate) fn from_embedded_presets() -> CoreRuntime<RealHost> {
    from_embedded_presets_for_platform(RuntimePlatform::current())
}

pub(crate) fn from_embedded_presets_for_platform(
    platform: RuntimePlatform,
) -> CoreRuntime<RealHost> {
    let mut builder = PresetSnapshot::builder(PresetSourceKind::Embedded);
    for logical in crate::presets::embedded_asset_paths("") {
        if let Some(bytes) = crate::presets::read_embedded_asset_bytes(&logical) {
            builder = builder.file(logical, bytes);
        }
    }
    let root = std::path::PathBuf::from(".shine-core-embedded");
    let context = RuntimeContext::isolated(
        root.join("home"),
        root.join("home/.shine"),
        root.join("presets"),
        root.join("home/.shine/bin"),
        platform,
    );
    CoreRuntime::new(RealHost, context, builder.build())
}

fn systemd_resolved_stub_active() -> bool {
    if !cfg!(target_os = "linux") {
        return true;
    }
    std::fs::read_to_string("/run/systemd/resolve/stub-resolv.conf")
        .map(|content| {
            content
                .lines()
                .any(|line| line.trim_start().starts_with("nameserver 127.0.0.53"))
        })
        .unwrap_or(true)
}

fn collect_tree(root: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut stack = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = stack.pop() {
        let entries = std::fs::read_dir(&directory)
            .with_context(|| format!("reading preset directory {}", directory.display()))?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                if entry.file_name() != "node_modules" {
                    stack.push(path);
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .context("preset file escaped its snapshot root")?;
            let logical = logical_path(relative);
            files.push((
                logical,
                std::fs::read(&path)
                    .with_context(|| format!("reading preset file {}", path.display()))?,
            ));
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

fn logical_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn proxy_env(
    env: &std::collections::BTreeMap<String, String>,
) -> std::collections::BTreeMap<String, String> {
    ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"]
        .into_iter()
        .filter_map(|key| env.get(key).map(|value| (key.to_string(), value.clone())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_captures_embedded_presets_without_exposing_rust_embed_to_core() {
        let root =
            std::env::temp_dir().join(format!("shine-core-adapter-{}", uuid::Uuid::new_v4()));
        let config = Config::new_for_test(&root);
        let runtime = from_config(&config).unwrap();
        assert!(runtime.presets().get("app/ghostty/shine.toml").is_some());
        assert_eq!(runtime.context().home_dir, root);
    }
}
