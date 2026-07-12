use crate::config::Config;
use crate::platform::current_platform;
use crate::presets;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use tokio::fs;

#[derive(Debug, Clone)]
pub struct ShellCategory {
    pub name: String,
    // Parsed from shine.toml metadata for completeness; not yet surfaced by any command.
    #[allow(dead_code)]
    pub description: Option<String>,
    pub files: Vec<ShellFile>,
    // Tracks whether the category came from an explicit metadata file vs. auto-collection;
    // reserved for future upgrade/list logic, mirroring AppCategory::uses_metadata.
    #[allow(dead_code)]
    pub uses_metadata: bool,
}

#[derive(Debug, Clone)]
pub struct ShellFile {
    pub source_rel: PathBuf,
    pub command_name: String,
    pub description: Vec<String>,
    pub needs_source: bool,
}

#[derive(Debug, Deserialize)]
struct CategoryToml {
    description: Option<String>,
    files: Option<Vec<FileToml>>,
}

#[derive(Debug, Deserialize)]
struct FileToml {
    source: String,
    target: Option<String>,
    needs_source: Option<bool>,
    platforms: Option<Vec<String>>,
}

pub fn load_embedded_categories(filter: Option<&str>) -> Result<Vec<ShellCategory>> {
    let names = collect_embedded_category_names(filter);
    let mut categories = Vec::new();
    for name in names {
        categories.push(load_embedded_category(&name)?);
    }
    Ok(categories)
}

pub async fn load_installed_categories(
    config: &Config,
    filter: Option<&str>,
) -> Result<Vec<ShellCategory>> {
    let shell_root = config.presets_dir().join("shell");
    let mut names: BTreeSet<String> = collect_fs_category_names(&shell_root, filter)
        .await?
        .into_iter()
        .collect();
    if let Some(overlay) = config.active_presets_overlay_dir() {
        names.extend(collect_fs_category_names(&overlay.join("shell"), filter).await?);
    }
    let mut categories = Vec::new();
    for name in names {
        categories.push(load_installed_category(config, &name).await?);
    }
    Ok(categories)
}

/// Loads categories from whichever source is active: installed (external
/// presets mode) or embedded. Replaces the `if config.is_external_presets {
/// load_installed_categories } else { load_embedded_categories }` branch
/// repeated at every call site.
pub async fn load_active_categories(
    config: &Config,
    filter: Option<&str>,
) -> Result<Vec<ShellCategory>> {
    if config.is_external_presets {
        load_installed_categories(config, filter).await
    } else {
        load_embedded_categories(filter)
    }
}

fn load_embedded_category(name: &str) -> Result<ShellCategory> {
    let metadata_path = format!("shell/{name}/shine.toml");
    if let Some(bytes) = presets::read_asset_bytes(&metadata_path) {
        let parsed = parse_category_toml(name, &bytes)?;
        let files = match parsed.files {
            Some(files) => files
                .into_iter()
                .filter_map(|file| match file_matches_current_platform(name, &file) {
                    Ok(true) => Some(Ok(file)),
                    Ok(false) => None,
                    Err(err) => Some(Err(err)),
                })
                .map(|file| {
                    let file = file?;
                    let needs_source = file.needs_source.unwrap_or(false);
                    let source_rel = normalize_shell_source(&file.source)
                        .with_context(|| format!("invalid source in shell/{name}/shine.toml"))?;
                    let command_name = resolve_command_name(&source_rel, file.target.as_deref())
                        .with_context(|| format!("invalid target in shell/{name}/shine.toml"))?;
                    let asset_path = format!("shell/{name}/{}", source_rel.display());
                    let bytes = presets::read_asset_bytes(&asset_path).with_context(|| {
                        format!("shell/{name}/shine.toml references missing file: {source_rel:?}")
                    })?;
                    Ok(ShellFile {
                        source_rel,
                        command_name,
                        description: presets::parse_script_description(&bytes),
                        needs_source,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            None => collect_embedded_scripts(name)?
                .into_iter()
                .map(|source_rel| {
                    let asset_path = format!("shell/{name}/{}", source_rel.display());
                    let bytes = presets::read_asset_bytes(&asset_path).unwrap_or_default();
                    let command_name = default_command_name(&source_rel)?;
                    Ok(ShellFile {
                        source_rel,
                        command_name,
                        description: presets::parse_script_description(&bytes),
                        needs_source: false,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        };

        return Ok(ShellCategory {
            name: name.to_string(),
            description: parsed.description,
            files,
            uses_metadata: true,
        });
    }

    Ok(ShellCategory {
        name: name.to_string(),
        description: None,
        files: collect_embedded_scripts(name)?
            .into_iter()
            .map(|source_rel| {
                let asset_path = format!("shell/{name}/{}", source_rel.display());
                let bytes = presets::read_asset_bytes(&asset_path).unwrap_or_default();
                Ok(ShellFile {
                    command_name: default_command_name(&source_rel)?,
                    description: presets::parse_script_description(&bytes),
                    needs_source: false,
                    source_rel,
                })
            })
            .collect::<Result<Vec<_>>>()?,
        uses_metadata: false,
    })
}

async fn load_installed_category(config: &Config, name: &str) -> Result<ShellCategory> {
    let category_rel = Path::new("shell").join(name);
    let metadata_path = config.preset_path(category_rel.join("shine.toml"));

    if metadata_path.exists() {
        let bytes = fs::read(&metadata_path)
            .await
            .with_context(|| format!("reading metadata: {}", metadata_path.display()))?;
        let parsed = parse_category_toml(name, &bytes)?;
        let files = match parsed.files {
            Some(files) => files
                .into_iter()
                .filter_map(|file| match file_matches_current_platform(name, &file) {
                    Ok(true) => Some(Ok(file)),
                    Ok(false) => None,
                    Err(err) => Some(Err(err)),
                })
                .map(|file| {
                    let file = file?;
                    let needs_source = file.needs_source.unwrap_or(false);
                    let source_rel = normalize_shell_source(&file.source).with_context(|| {
                        format!("invalid source in {}", metadata_path.display())
                    })?;
                    let command_name = resolve_command_name(&source_rel, file.target.as_deref())
                        .with_context(|| {
                            format!("invalid target in {}", metadata_path.display())
                        })?;
                    Ok((source_rel, command_name, needs_source))
                })
                .collect::<Result<Vec<_>>>()?,
            None => collect_merged_fs_scripts(config, &category_rel)
                .await?
                .into_iter()
                .map(|source_rel| {
                    let command_name = default_command_name(&source_rel)?;
                    Ok((source_rel, command_name, false))
                })
                .collect::<Result<Vec<_>>>()?,
        };

        let mut shell_files = Vec::new();
        for (source_rel, command_name, needs_source) in files {
            let source_path = config.preset_path(category_rel.join(&source_rel));
            if !source_path.exists() {
                bail!(
                    "shell/{name}/shine.toml references missing file: {}",
                    source_rel.display()
                );
            }
            let bytes = fs::read(&source_path)
                .await
                .with_context(|| format!("reading preset file: {}", source_path.display()))?;
            shell_files.push(ShellFile {
                source_rel,
                command_name,
                description: presets::parse_script_description(&bytes),
                needs_source,
            });
        }

        return Ok(ShellCategory {
            name: name.to_string(),
            description: parsed.description,
            files: shell_files,
            uses_metadata: true,
        });
    }

    let mut files = Vec::new();
    for source_rel in collect_merged_fs_scripts(config, &category_rel).await? {
        let source_path = config.preset_path(category_rel.join(&source_rel));
        let bytes = fs::read(&source_path)
            .await
            .with_context(|| format!("reading preset file: {}", source_path.display()))?;
        files.push(ShellFile {
            command_name: default_command_name(&source_rel)?,
            description: presets::parse_script_description(&bytes),
            needs_source: false,
            source_rel,
        });
    }

    Ok(ShellCategory {
        name: name.to_string(),
        description: None,
        files,
        uses_metadata: false,
    })
}

async fn collect_merged_fs_scripts(config: &Config, category_rel: &Path) -> Result<Vec<PathBuf>> {
    crate::preset_meta::merge_fs_tree(config, category_rel, "directory", |rel| {
        if !is_shell_script(rel) {
            return Ok(None);
        }
        Ok(Some(normalize_shell_source(rel)?))
    })
    .await
}

fn parse_category_toml(name: &str, bytes: &[u8]) -> Result<CategoryToml> {
    toml::from_slice(bytes).with_context(|| format!("failed to parse shell/{name}/shine.toml"))
}

fn file_matches_current_platform(category: &str, file: &FileToml) -> Result<bool> {
    file_matches_platform(category, file, current_platform())
}

fn file_matches_platform(category: &str, file: &FileToml, current: &str) -> Result<bool> {
    crate::preset_meta::platform_matches(
        file.platforms.as_deref(),
        current,
        &format!("shell/{category}/shine.toml"),
    )
}

fn collect_embedded_category_names(filter: Option<&str>) -> Vec<String> {
    crate::preset_meta::collect_embedded_category_names("shell", filter)
}

async fn collect_fs_category_names(shell_root: &Path, filter: Option<&str>) -> Result<Vec<String>> {
    crate::preset_meta::collect_fs_category_names(shell_root, filter, "shell presets directory")
        .await
}

fn collect_embedded_scripts(name: &str) -> Result<Vec<PathBuf>> {
    let prefix = format!("shell/{name}/");
    let mut scripts = BTreeSet::new();
    for asset_path in presets::asset_paths(&format!("shell/{name}")) {
        let Some(rest) = asset_path.strip_prefix(&prefix) else {
            continue;
        };
        if rest == "shine.toml" {
            continue;
        }
        let rel = PathBuf::from(rest);
        if !is_shell_script(&rel) {
            continue;
        }
        scripts.insert(normalize_shell_source(rest)?);
    }
    Ok(scripts.into_iter().collect())
}

fn normalize_shell_source(path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    if path.as_os_str().is_empty() {
        bail!("source path must not be empty");
    }
    if path.is_absolute() {
        bail!("source path must be relative");
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => bail!("source path must not contain '..'"),
            _ => bail!("source path must be relative"),
        }
    }

    if normalized.as_os_str().is_empty() {
        bail!("source path must not be empty");
    }
    if normalized.file_name().and_then(|name| name.to_str()) == Some("shine.toml") {
        bail!("source path must not point to shine.toml");
    }
    if !is_shell_script(&normalized) {
        bail!("source path must end with .sh or .ps1");
    }
    Ok(normalized)
}

fn is_shell_script(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("sh" | "ps1")
    )
}

fn resolve_command_name(source_rel: &Path, target: Option<&str>) -> Result<String> {
    match target {
        Some(target) => validate_command_name(target),
        None => default_command_name(source_rel),
    }
}

fn default_command_name(source_rel: &Path) -> Result<String> {
    let stem = crate::bin_links::link_stem(source_rel);
    let stem = stem
        .into_string()
        .map_err(|_| anyhow::anyhow!("command name must be valid UTF-8"))?;
    validate_command_name(&stem)
}

fn validate_command_name(target: &str) -> Result<String> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        bail!("command name must not be empty");
    }
    if trimmed == "." || trimmed == ".." {
        bail!("command name must be a plain filename");
    }
    let path = Path::new(trimmed);
    match path.components().next() {
        Some(Component::Normal(_)) if path.components().count() == 1 => Ok(trimmed.to_string()),
        _ => bail!("command name must be a plain filename"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::fs;

    async fn make_temp_dir() -> PathBuf {
        crate::test_support::make_temp_dir("shine-shell-meta").await
    }

    #[test]
    fn embedded_proxy_category_uses_renamed_commands() {
        let categories = load_embedded_categories(Some("proxy")).unwrap();
        let proxy = categories.iter().find(|cat| cat.name == "proxy").unwrap();
        let names: Vec<_> = proxy
            .files
            .iter()
            .map(|file| file.command_name.as_str())
            .collect();
        assert!(names.contains(&"setproxy"));
        assert!(names.contains(&"usetproxy"));
        assert!(!names.contains(&"set_proxy"));
    }

    #[test]
    fn embedded_proxy_category_uses_platform_specific_scripts() {
        let categories = load_embedded_categories(Some("proxy")).unwrap();
        let proxy = categories.iter().find(|cat| cat.name == "proxy").unwrap();
        let sources: Vec<_> = proxy
            .files
            .iter()
            .map(|file| file.source_rel.as_path())
            .collect();

        if cfg!(windows) {
            assert!(sources.contains(&Path::new("set_proxy.ps1")));
            assert!(sources.contains(&Path::new("uset_proxy.ps1")));
            assert!(!sources.contains(&Path::new("set_proxy.sh")));
            assert!(!sources.contains(&Path::new("uset_proxy.sh")));
        } else {
            assert!(sources.contains(&Path::new("set_proxy.sh")));
            assert!(sources.contains(&Path::new("uset_proxy.sh")));
            assert!(!sources.contains(&Path::new("set_proxy.ps1")));
            assert!(!sources.contains(&Path::new("uset_proxy.ps1")));
        }
    }

    #[test]
    fn metadata_platform_filter_accepts_current_platform() {
        let file = FileToml {
            source: "set_proxy.ps1".to_string(),
            target: Some("setproxy".to_string()),
            needs_source: Some(true),
            platforms: Some(vec!["windows".to_string()]),
        };

        assert!(file_matches_platform("proxy", &file, "windows").unwrap());
        assert!(!file_matches_platform("proxy", &file, "unix").unwrap());
    }

    #[test]
    fn metadata_platform_filter_defaults_to_all_platforms() {
        let file = FileToml {
            source: "set_proxy.sh".to_string(),
            target: Some("setproxy".to_string()),
            needs_source: Some(true),
            platforms: None,
        };

        assert!(file_matches_platform("proxy", &file, "windows").unwrap());
        assert!(file_matches_platform("proxy", &file, "unix").unwrap());
    }

    #[test]
    fn metadata_platform_filter_rejects_unknown_platforms() {
        let file = FileToml {
            source: "set_proxy.sh".to_string(),
            target: Some("setproxy".to_string()),
            needs_source: Some(true),
            platforms: Some(vec!["plan9".to_string()]),
        };

        let err = file_matches_platform("proxy", &file, "unix")
            .unwrap_err()
            .to_string();
        assert!(err.contains("unsupported platform `plan9`"));
    }

    #[test]
    fn embedded_agent_category_uses_ccenv_source_wrapper() {
        let categories = load_embedded_categories(Some("agent")).unwrap();
        let agent = categories.iter().find(|cat| cat.name == "agent").unwrap();

        assert_eq!(agent.files.len(), 1);
        assert_eq!(agent.files[0].command_name, "ccenv");
        let expected = if cfg!(windows) { "cc.ps1" } else { "cc.sh" };
        assert_eq!(agent.files[0].source_rel, PathBuf::from(expected));
        assert!(agent.files[0].needs_source);

        let bytes = presets::read_asset_bytes(&format!("shell/agent/{expected}")).unwrap();
        assert!(presets::parse_template_annotation(&bytes));
    }

    #[test]
    fn embedded_utils_category_exposes_copyfile_command() {
        let categories = load_embedded_categories(Some("utils")).unwrap();
        let utils = categories.iter().find(|cat| cat.name == "utils").unwrap();

        if cfg!(windows) {
            assert_eq!(utils.files.len(), 1);
            assert_eq!(utils.files[0].command_name, "shine-env-export");
            assert!(utils.files[0].needs_source);
        } else {
            assert_eq!(utils.files.len(), 2);
            let copyfile = utils
                .files
                .iter()
                .find(|f| f.command_name == "copyfile")
                .expect("copyfile should be present");
            assert_eq!(copyfile.source_rel, PathBuf::from("copyfile.sh"));
            assert!(!copyfile.needs_source);
            assert!(
                copyfile.description.contains(
                    &"Copy a file's contents to the local clipboard via OSC52.".to_string()
                )
            );

            let env_export = utils
                .files
                .iter()
                .find(|f| f.command_name == "shine-env-export")
                .expect("shine-env-export should be present");
            assert_eq!(env_export.source_rel, PathBuf::from("shine-env-export.sh"));
            assert!(env_export.needs_source);
        }
    }

    #[tokio::test]
    async fn installed_metadata_applies_target_names() {
        let dir = make_temp_dir().await;
        let category_root = dir.join("presets/shell/custom");
        fs::create_dir_all(&category_root).await.unwrap();
        fs::write(
            category_root.join("shine.toml"),
            b"[[files]]\nsource = \"set_proxy.sh\"\ntarget = \"setproxy\"\n",
        )
        .await
        .unwrap();
        fs::write(
            category_root.join("set_proxy.sh"),
            b"#!/bin/bash\n# Set proxy.\n",
        )
        .await
        .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        let categories = load_installed_categories(&config, Some("custom"))
            .await
            .unwrap();
        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].files[0].command_name, "setproxy");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn external_presets_and_overlay_categories_are_merged() {
        let dir = make_temp_dir().await;
        let base_root = dir.join("presets/shell/custom");
        let overlay = dir.join("overlay");
        let overlay_root = overlay.join("shell/custom");
        let overlay_only = overlay.join("shell/personal");
        fs::create_dir_all(&base_root).await.unwrap();
        fs::create_dir_all(&overlay_root).await.unwrap();
        fs::create_dir_all(&overlay_only).await.unwrap();
        fs::write(
            base_root.join("shine.toml"),
            b"[[files]]\nsource = \"tool.sh\"\n",
        )
        .await
        .unwrap();
        fs::write(base_root.join("tool.sh"), b"#!/bin/bash\n# Base tool.\n")
            .await
            .unwrap();
        fs::write(
            overlay_root.join("tool.sh"),
            b"#!/bin/bash\n# Overlay tool.\n",
        )
        .await
        .unwrap();
        fs::write(
            overlay_only.join("personal.sh"),
            b"#!/bin/bash\n# Personal tool.\n",
        )
        .await
        .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config.presets_overlay_dir_override = Some(overlay);
        let categories = load_installed_categories(&config, None).await.unwrap();

        let custom = categories.iter().find(|cat| cat.name == "custom").unwrap();
        assert_eq!(custom.files[0].description, vec!["Overlay tool."]);
        assert!(categories.iter().any(|cat| cat.name == "personal"));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn installed_category_accepts_powershell_scripts() {
        let dir = make_temp_dir().await;
        let category_root = dir.join("presets/shell/custom");
        fs::create_dir_all(&category_root).await.unwrap();
        fs::write(
            category_root.join("tool.ps1"),
            b"# Tool.\nWrite-Output hi\n",
        )
        .await
        .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        let categories = load_installed_categories(&config, Some("custom"))
            .await
            .unwrap();

        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].files[0].source_rel, PathBuf::from("tool.ps1"));
        assert_eq!(categories[0].files[0].command_name, "tool");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn installed_metadata_filters_platform_specific_files() {
        let dir = make_temp_dir().await;
        let category_root = dir.join("presets/shell/custom");
        fs::create_dir_all(&category_root).await.unwrap();
        fs::write(
            category_root.join("shine.toml"),
            b"[[files]]\nsource = \"tool.sh\"\ntarget = \"tool\"\nplatforms = [\"unix\"]\n\n[[files]]\nsource = \"tool.ps1\"\ntarget = \"tool\"\nplatforms = [\"windows\"]\n",
        )
        .await
        .unwrap();
        fs::write(category_root.join("tool.sh"), b"#!/bin/bash\n")
            .await
            .unwrap();
        fs::write(category_root.join("tool.ps1"), b"Write-Output hi\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        let categories = load_installed_categories(&config, Some("custom"))
            .await
            .unwrap();

        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].files.len(), 1);
        let expected = if cfg!(windows) { "tool.ps1" } else { "tool.sh" };
        assert_eq!(categories[0].files[0].source_rel, PathBuf::from(expected));
        assert_eq!(categories[0].files[0].command_name, "tool");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn rejects_invalid_command_names() {
        let err = validate_command_name("bin/setproxy")
            .unwrap_err()
            .to_string();
        assert!(err.contains("plain filename"));
    }
}
