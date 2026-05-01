use crate::config::Config;
use crate::presets;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use tokio::fs;

#[derive(Debug, Clone)]
pub(crate) struct AppCategory {
    pub name: String,
    pub description: Option<String>,
    pub destination_root: Option<String>,
    pub files: Vec<AppFile>,
    #[allow(dead_code)]
    pub uses_metadata: bool,
    /// `true` when shine.toml has an explicit `[[files]]` section;
    /// `false` for auto-collected files and legacy categories.
    pub has_explicit_files: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct AppFile {
    pub source_rel: PathBuf,
    pub target_rel: PathBuf,
    pub description: Option<String>,
    pub display_name: Option<String>,
    pub legacy_dest_annotation: Option<String>,
    pub transforms: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CategoryToml {
    description: Option<String>,
    dest: String,
    files: Option<Vec<FileToml>>,
}

#[derive(Debug, Deserialize)]
struct FileToml {
    source: String,
    target: Option<String>,
    description: Option<String>,
    display_name: Option<String>,
    #[serde(default)]
    transform: Option<String>,
    #[serde(default)]
    transforms: Option<Vec<String>>,
}

fn resolve_transforms(file: &FileToml, context: &str) -> Result<Vec<String>> {
    let specs = match (&file.transform, &file.transforms) {
        (Some(_), Some(_)) => {
            bail!("{context}: use 'transform' or 'transforms', not both")
        }
        (Some(t), None) => vec![t.clone()],
        (None, Some(ts)) => ts.clone(),
        (None, None) => vec![],
    };
    super::transforms::validate(&specs).with_context(|| format!("{context}: invalid transform"))?;
    Ok(specs)
}

pub(crate) fn load_embedded_categories(filter: Option<&str>) -> Result<Vec<AppCategory>> {
    let filter = filter.map(str::to_string);
    let names = collect_embedded_category_names(filter.as_deref());
    let mut categories = Vec::new();

    for name in names {
        categories.push(load_embedded_category(&name)?);
    }

    Ok(categories)
}

pub(crate) async fn load_installed_categories(
    config: &Config,
    filter: Option<&str>,
) -> Result<Vec<AppCategory>> {
    let app_root = config.presets_dir().join("app");
    let category_names = collect_fs_category_names(&app_root, filter).await?;
    let mut categories = Vec::new();

    for name in category_names {
        categories.push(load_installed_category(config, &name).await?);
    }

    Ok(categories)
}

fn load_embedded_category(name: &str) -> Result<AppCategory> {
    let metadata_path = format!("app/{name}/shine.toml");
    if let Some(bytes) = presets::read_asset_bytes(&metadata_path) {
        let parsed = parse_category_toml(name, &bytes)?;
        let has_explicit_files = parsed.files.is_some();
        let files = match parsed.files {
            Some(files) => files
                .into_iter()
                .map(|file| {
                    let context = format!("app/{name}/shine.toml");
                    let source_rel = normalize_relative(&file.source)
                        .with_context(|| format!("invalid source for {context}"))?;
                    let target_rel =
                        normalize_relative(file.target.as_deref().unwrap_or(&file.source))
                            .with_context(|| format!("invalid target for {context}"))?;
                    let transforms = resolve_transforms(&file, &context)?;
                    Ok(AppFile {
                        source_rel,
                        target_rel,
                        description: file.description,
                        display_name: file.display_name,
                        legacy_dest_annotation: None,
                        transforms,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            None => collect_embedded_files(name)?
                .into_iter()
                .map(|rel| AppFile {
                    source_rel: rel.clone(),
                    target_rel: rel,
                    description: None,
                    display_name: None,
                    legacy_dest_annotation: None,
                    transforms: vec![],
                })
                .collect(),
        };

        return Ok(AppCategory {
            name: name.to_string(),
            description: parsed.description,
            destination_root: Some(parsed.dest),
            files,
            uses_metadata: true,
            has_explicit_files,
        });
    }

    Ok(AppCategory {
        name: name.to_string(),
        description: None,
        destination_root: None,
        files: collect_embedded_files(name)?
            .into_iter()
            .map(|rel| {
                let asset_path = format!("app/{name}/{}", rel.to_string_lossy());
                let bytes = presets::read_asset_bytes(&asset_path).unwrap_or_default();
                AppFile {
                    source_rel: rel.clone(),
                    target_rel: rel,
                    description: parse_legacy_description(&bytes),
                    display_name: None,
                    legacy_dest_annotation: presets::parse_dest_annotation(&bytes),
                    transforms: vec![],
                }
            })
            .collect(),
        uses_metadata: false,
        has_explicit_files: false,
    })
}

async fn load_installed_category(config: &Config, name: &str) -> Result<AppCategory> {
    let category_root = config.presets_dir().join("app").join(name);
    let metadata_path = category_root.join("shine.toml");

    if metadata_path.exists() {
        let bytes = fs::read(&metadata_path)
            .await
            .with_context(|| format!("reading metadata: {}", metadata_path.display()))?;
        let parsed = parse_category_toml(name, &bytes)?;
        let has_explicit_files = parsed.files.is_some();
        let files = match parsed.files {
            Some(files) => files
                .into_iter()
                .map(|file| {
                    let context = metadata_path.display().to_string();
                    let source_rel = normalize_relative(&file.source)
                        .with_context(|| format!("invalid source for {context}"))?;
                    let target_rel =
                        normalize_relative(file.target.as_deref().unwrap_or(&file.source))
                            .with_context(|| format!("invalid target for {context}"))?;
                    let transforms = resolve_transforms(&file, &context)?;
                    Ok(AppFile {
                        source_rel,
                        target_rel,
                        description: file.description,
                        display_name: file.display_name,
                        legacy_dest_annotation: None,
                        transforms,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            None => collect_fs_files(&category_root)
                .await?
                .into_iter()
                .map(|rel| AppFile {
                    source_rel: rel.clone(),
                    target_rel: rel,
                    description: None,
                    display_name: None,
                    legacy_dest_annotation: None,
                    transforms: vec![],
                })
                .collect(),
        };

        for file in &files {
            let source_path = category_root.join(&file.source_rel);
            if !source_path.exists() {
                bail!(
                    "app/{name}/shine.toml references missing file: {}",
                    file.source_rel.display()
                );
            }
        }

        return Ok(AppCategory {
            name: name.to_string(),
            description: parsed.description,
            destination_root: Some(parsed.dest),
            files,
            uses_metadata: true,
            has_explicit_files,
        });
    }

    let mut files = Vec::new();
    for rel in collect_fs_files(&category_root).await? {
        let source_path = category_root.join(&rel);
        let bytes = fs::read(&source_path)
            .await
            .with_context(|| format!("reading preset file: {}", source_path.display()))?;
        files.push(AppFile {
            source_rel: rel.clone(),
            target_rel: rel,
            description: parse_legacy_description(&bytes),
            display_name: None,
            legacy_dest_annotation: presets::parse_dest_annotation(&bytes),
            transforms: vec![],
        });
    }

    Ok(AppCategory {
        name: name.to_string(),
        description: None,
        destination_root: None,
        files,
        uses_metadata: false,
        has_explicit_files: false,
    })
}

fn collect_embedded_category_names(filter: Option<&str>) -> Vec<String> {
    let mut names = BTreeSet::new();
    for asset_path in presets::asset_paths("app") {
        let Some(rest) = asset_path.strip_prefix("app/") else {
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

async fn collect_fs_category_names(app_root: &Path, filter: Option<&str>) -> Result<Vec<String>> {
    if let Some(filter) = filter {
        let path = app_root.join(filter);
        if path.exists() {
            return Ok(vec![filter.to_string()]);
        }
        bail!("app preset category not found: {filter}");
    }

    if !app_root.exists() {
        return Ok(Vec::new());
    }

    let mut names = BTreeSet::new();
    let mut entries = fs::read_dir(app_root)
        .await
        .with_context(|| format!("reading app presets dir: {}", app_root.display()))?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_dir() {
            names.insert(entry.file_name().to_string_lossy().to_string());
        }
    }
    Ok(names.into_iter().collect())
}

fn collect_embedded_files(category: &str) -> Result<Vec<PathBuf>> {
    let prefix = format!("app/{category}/");
    let mut files = Vec::new();

    for asset_path in presets::asset_paths(&prefix) {
        let Some(rel) = asset_path.strip_prefix(&prefix) else {
            continue;
        };
        if rel.is_empty() || rel == "shine.toml" {
            continue;
        }
        files.push(normalize_relative(rel)?);
    }

    files.sort();
    Ok(files)
}

async fn collect_fs_files(category_root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![category_root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let mut entries = fs::read_dir(&dir)
            .await
            .with_context(|| format!("reading preset category: {}", dir.display()))?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let file_type = entry.file_type().await?;
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if file_type.is_file() {
                let rel = path
                    .strip_prefix(category_root)
                    .with_context(|| format!("file outside category root: {}", path.display()))?;
                let rel = normalize_relative(&rel.to_string_lossy())?;
                if rel == std::path::Path::new("shine.toml") {
                    continue;
                }
                files.push(rel);
            }
        }
    }

    files.sort();
    Ok(files)
}

fn parse_category_toml(name: &str, bytes: &[u8]) -> Result<CategoryToml> {
    let parsed: CategoryToml = toml::from_slice(bytes)
        .with_context(|| format!("failed to parse app/{name}/shine.toml"))?;

    let expanded = crate::config::full_expand(&parsed.dest)
        .with_context(|| format!("failed to expand dest in app/{name}/shine.toml"))?;
    let path = PathBuf::from(&expanded);
    if !path.is_absolute() {
        bail!("app/{name}/shine.toml dest must be absolute after expansion");
    }
    if path.components().any(|c| c == Component::ParentDir) {
        bail!("app/{name}/shine.toml dest must not contain '..'");
    }
    Ok(parsed)
}

fn normalize_relative(path: &str) -> Result<PathBuf> {
    let path = Path::new(path);
    if path.as_os_str().is_empty() {
        bail!("path must not be empty");
    }
    if path.is_absolute() {
        bail!("path must be relative");
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        bail!("path must not contain '..'");
    }
    Ok(path.to_path_buf())
}

fn parse_legacy_description(content: &[u8]) -> Option<String> {
    let description = presets::parse_script_description(content);
    if description.is_empty() {
        None
    } else {
        Some(description.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_vim_uses_metadata() {
        let categories = load_embedded_categories(Some("vim")).unwrap();
        let vim = categories.iter().find(|c| c.name == "vim").unwrap();
        assert!(vim.uses_metadata);
        assert_eq!(vim.destination_root.as_deref(), Some("~/.vim"));
        assert!(!vim.files.is_empty());
    }

    #[test]
    fn embedded_git_stays_legacy() {
        let categories = load_embedded_categories(Some("git")).unwrap();
        let git = categories.iter().find(|c| c.name == "git").unwrap();
        assert!(!git.uses_metadata);
        assert_eq!(git.files.len(), 1);
        assert_eq!(
            git.files[0].legacy_dest_annotation.as_deref(),
            Some("~/.gitconfig")
        );
    }

    #[test]
    fn embedded_docker_has_jsonc_transform() {
        let categories = load_embedded_categories(Some("docker")).unwrap();
        let docker = categories.iter().find(|c| c.name == "docker").unwrap();
        assert!(docker.uses_metadata);
        assert_eq!(docker.destination_root.as_deref(), Some("/etc/docker"));
        assert_eq!(docker.files.len(), 1);

        let file = &docker.files[0];
        assert_eq!(file.source_rel, std::path::Path::new("daemon.jsonc"));
        assert_eq!(file.target_rel, std::path::Path::new("daemon.json"));
        assert_eq!(file.transforms, vec!["jsonc-to-json"]);
    }

    #[test]
    fn unknown_transform_rejected_at_load() {
        let toml =
            b"dest = \"/tmp\"\n[[files]]\nsource = \"f\"\ntransform = \"no-such-transform\"\n";
        assert!(
            parse_category_toml("test", toml).is_err() || {
                // parse_category_toml only validates dest; full validation happens in load_embedded_category.
                // Ensure resolve_transforms rejects it.
                let file = FileToml {
                    source: "f".to_string(),
                    target: None,
                    description: None,
                    display_name: None,
                    transform: Some("no-such-transform".to_string()),
                    transforms: None,
                };
                resolve_transforms(&file, "test").is_err()
            }
        );
    }

    #[test]
    fn both_transform_and_transforms_rejected() {
        let file = FileToml {
            source: "f".to_string(),
            target: None,
            description: None,
            display_name: None,
            transform: Some("jsonc-to-json".to_string()),
            transforms: Some(vec!["jsonc-to-json".to_string()]),
        };
        assert!(resolve_transforms(&file, "test").is_err());
    }
}
