use crate::config::Config;
use crate::platform::current_platform;
use crate::presets;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use tokio::fs;

#[derive(Debug, Clone)]
pub(crate) struct ShellCategory {
    pub name: String,
    #[allow(dead_code)]
    pub description: Option<String>,
    pub files: Vec<ShellFile>,
    #[allow(dead_code)]
    pub uses_metadata: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ShellFile {
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

pub(crate) fn load_embedded_categories(filter: Option<&str>) -> Result<Vec<ShellCategory>> {
    let names = collect_embedded_category_names(filter);
    let mut categories = Vec::new();
    for name in names {
        categories.push(load_embedded_category(&name)?);
    }
    Ok(categories)
}

pub(crate) async fn load_installed_categories(
    config: &Config,
    filter: Option<&str>,
) -> Result<Vec<ShellCategory>> {
    let shell_root = config.presets_dir().join("shell");
    let names = collect_fs_category_names(&shell_root, filter).await?;
    let mut categories = Vec::new();
    for name in names {
        categories.push(load_installed_category(config, &name).await?);
    }
    Ok(categories)
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
    let category_root = config.presets_dir().join("shell").join(name);
    let metadata_path = category_root.join("shine.toml");

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
            None => collect_fs_scripts(&category_root)
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
            let source_path = category_root.join(&source_rel);
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
    for source_rel in collect_fs_scripts(&category_root).await? {
        let source_path = category_root.join(&source_rel);
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

fn parse_category_toml(name: &str, bytes: &[u8]) -> Result<CategoryToml> {
    toml::from_slice(bytes).with_context(|| format!("failed to parse shell/{name}/shine.toml"))
}

fn file_matches_current_platform(category: &str, file: &FileToml) -> Result<bool> {
    file_matches_platform(category, file, current_platform())
}

fn file_matches_platform(category: &str, file: &FileToml, current: &str) -> Result<bool> {
    let Some(platforms) = &file.platforms else {
        return Ok(true);
    };

    let mut matches = false;
    for platform in platforms {
        let normalized = platform.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "windows" | "unix" => {
                matches |= normalized == current;
            }
            _ => bail!(
                "shell/{category}/shine.toml has unsupported platform `{platform}`; expected `windows` or `unix`"
            ),
        }
    }
    Ok(matches)
}

fn collect_embedded_category_names(filter: Option<&str>) -> Vec<String> {
    let mut names = BTreeSet::new();
    for asset_path in presets::asset_paths("shell") {
        let Some(rest) = asset_path.strip_prefix("shell/") else {
            continue;
        };
        let Some((category, _)) = rest.split_once('/') else {
            continue;
        };
        if filter.is_none_or(|f| f == category) {
            names.insert(category.to_string());
        }
    }
    names.into_iter().collect()
}

async fn collect_fs_category_names(shell_root: &Path, filter: Option<&str>) -> Result<Vec<String>> {
    if let Some(filter) = filter {
        let path = shell_root.join(filter);
        if path.exists() {
            return Ok(vec![filter.to_string()]);
        }
        return Ok(Vec::new());
    }

    if !shell_root.exists() {
        return Ok(Vec::new());
    }

    let mut names = BTreeSet::new();
    let mut entries = fs::read_dir(shell_root)
        .await
        .with_context(|| format!("reading shell presets directory: {}", shell_root.display()))?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_dir() {
            names.insert(entry.file_name().to_string_lossy().to_string());
        }
    }
    Ok(names.into_iter().collect())
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

async fn collect_fs_scripts(category_root: &Path) -> Result<Vec<PathBuf>> {
    if !category_root.is_dir() {
        return Ok(Vec::new());
    }

    let mut scripts = BTreeSet::new();
    let mut stack = vec![category_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = fs::read_dir(&dir)
            .await
            .with_context(|| format!("reading directory: {}", dir.display()))?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let ft = entry.file_type().await?;
            if ft.is_dir() {
                stack.push(path);
                continue;
            }
            if !ft.is_file() || !is_shell_script(&path) {
                continue;
            }
            let rel = path.strip_prefix(category_root).with_context(|| {
                format!(
                    "failed to strip category root {} from {}",
                    category_root.display(),
                    path.display()
                )
            })?;
            scripts.insert(normalize_shell_source(rel)?);
        }
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
        let dir = std::env::temp_dir().join(format!("shine-shell-meta-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();
        dir
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
            assert!(utils.files.is_empty());
        } else {
            assert_eq!(utils.files.len(), 1);
            assert_eq!(utils.files[0].command_name, "copyfile");
            assert_eq!(utils.files[0].source_rel, PathBuf::from("copyfile.sh"));
            assert!(!utils.files[0].needs_source);
            assert!(
                utils.files[0].description.contains(
                    &"Copy a file's contents to the local clipboard via OSC52.".to_string()
                )
            );
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
