use super::manifest::AppInstallStrategy;
use crate::config::Config;
use crate::platform::current_platform;
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
    pub list_mode: AppListMode,
    // Tracks whether the category came from an explicit metadata file vs. auto-collection;
    // reserved for future upgrade/list logic.
    #[allow(dead_code)]
    pub uses_metadata: bool,
    /// `true` when shine.toml has an explicit `[[files]]` section;
    /// `false` for auto-collected files and legacy categories.
    pub has_explicit_files: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppListMode {
    Category,
    Files,
}

#[derive(Debug, Clone)]
pub(crate) struct AppFile {
    pub source_rel: PathBuf,
    pub target_rel: PathBuf,
    pub description: Option<String>,
    pub display_name: Option<String>,
    pub legacy_dest_annotation: Option<String>,
    pub transforms: Vec<String>,
    pub install_strategy: AppInstallStrategy,
}

#[derive(Debug, Deserialize)]
struct CategoryToml {
    description: Option<String>,
    dest: DestToml,
    list_mode: Option<ListModeToml>,
    files: Option<Vec<FileToml>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum DestToml {
    Single(String),
    Platforms(PlatformDestToml),
}

#[derive(Debug, Deserialize)]
struct PlatformDestToml {
    windows: Option<String>,
    unix: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ListModeToml {
    Category,
    Files,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum InstallModeToml {
    Copy,
    JsonMerge,
}

impl From<ListModeToml> for AppListMode {
    fn from(value: ListModeToml) -> Self {
        match value {
            ListModeToml::Category => Self::Category,
            ListModeToml::Files => Self::Files,
        }
    }
}

#[derive(Debug, Deserialize)]
struct FileToml {
    source: String,
    target: Option<String>,
    description: Option<String>,
    display_name: Option<String>,
    #[serde(default)]
    platforms: Option<Vec<String>>,
    #[serde(default)]
    transform: Option<String>,
    #[serde(default)]
    transforms: Option<Vec<String>>,
    #[serde(default)]
    install_mode: Option<InstallModeToml>,
    #[serde(default)]
    managed_keys: Option<Vec<String>>,
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

fn resolve_install_strategy(file: &FileToml, context: &str) -> Result<AppInstallStrategy> {
    match file.install_mode.unwrap_or(InstallModeToml::Copy) {
        InstallModeToml::Copy => {
            if file.managed_keys.is_some() {
                bail!("{context}: 'managed_keys' requires install_mode = \"json-merge\"");
            }
            Ok(AppInstallStrategy::Copy)
        }
        InstallModeToml::JsonMerge => {
            let managed_keys = file
                .managed_keys
                .clone()
                .ok_or_else(|| anyhow::anyhow!("{context}: json-merge requires 'managed_keys'"))?;
            if managed_keys.is_empty() {
                bail!("{context}: managed_keys must not be empty");
            }
            for key in &managed_keys {
                if key.trim().is_empty() {
                    bail!("{context}: managed_keys must not contain empty keys");
                }
                if key.contains('.') {
                    bail!("{context}: managed_keys must be top-level JSON keys");
                }
            }
            Ok(AppInstallStrategy::JsonMerge { managed_keys })
        }
    }
}

fn default_list_mode(has_explicit_files: bool) -> AppListMode {
    if has_explicit_files {
        AppListMode::Files
    } else {
        AppListMode::Category
    }
}

pub(crate) fn load_embedded_categories(filter: Option<&str>) -> Result<Vec<AppCategory>> {
    let filter = filter.map(str::to_string);
    let names = collect_embedded_category_names(filter.as_deref());
    let mut categories = Vec::new();

    for name in names {
        if let Some(category) = load_embedded_category(&name)? {
            categories.push(category);
        }
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
        if let Some(category) = load_installed_category(config, &name).await? {
            categories.push(category);
        }
    }

    Ok(categories)
}

fn load_embedded_category(name: &str) -> Result<Option<AppCategory>> {
    let metadata_path = format!("app/{name}/shine.toml");
    if let Some(bytes) = presets::read_asset_bytes(&metadata_path) {
        let parsed = parse_category_toml(name, &bytes)?;
        let has_explicit_files = parsed.files.is_some();
        let Some(dest_root) = parsed.dest.select_for_current_platform(name)? else {
            return Ok(None);
        };
        let files = match parsed.files {
            Some(files) => {
                let mut filtered = Vec::new();
                for file in files {
                    if file_matches_current_platform(name, &file)? {
                        filtered.push(file);
                    }
                }
                filtered
                    .into_iter()
                    .map(|file| {
                        let context = format!("app/{name}/shine.toml");
                        let source_rel = normalize_relative(&file.source)
                            .with_context(|| format!("invalid source for {context}"))?;
                        let target_rel =
                            normalize_relative(file.target.as_deref().unwrap_or(&file.source))
                                .with_context(|| format!("invalid target for {context}"))?;
                        let transforms = resolve_transforms(&file, &context)?;
                        let install_strategy = resolve_install_strategy(&file, &context)?;
                        Ok(AppFile {
                            source_rel,
                            target_rel,
                            description: file.description,
                            display_name: file.display_name,
                            legacy_dest_annotation: None,
                            transforms,
                            install_strategy,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?
            }
            None => collect_embedded_files(name)?
                .into_iter()
                .map(|rel| AppFile {
                    source_rel: rel.clone(),
                    target_rel: rel,
                    description: None,
                    display_name: None,
                    legacy_dest_annotation: None,
                    transforms: vec![],
                    install_strategy: AppInstallStrategy::Copy,
                })
                .collect(),
        };
        if files.is_empty() {
            return Ok(None);
        }

        return Ok(Some(AppCategory {
            name: name.to_string(),
            description: parsed.description,
            destination_root: Some(dest_root),
            files,
            list_mode: parsed
                .list_mode
                .map(Into::into)
                .unwrap_or_else(|| default_list_mode(has_explicit_files)),
            uses_metadata: true,
            has_explicit_files,
        }));
    }

    Ok(Some(AppCategory {
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
                    install_strategy: AppInstallStrategy::Copy,
                }
            })
            .collect(),
        list_mode: AppListMode::Category,
        uses_metadata: false,
        has_explicit_files: false,
    }))
}

async fn load_installed_category(config: &Config, name: &str) -> Result<Option<AppCategory>> {
    let category_root = config.presets_dir().join("app").join(name);
    let metadata_path = category_root.join("shine.toml");

    if metadata_path.exists() {
        let bytes = fs::read(&metadata_path)
            .await
            .with_context(|| format!("reading metadata: {}", metadata_path.display()))?;
        let parsed = parse_category_toml(name, &bytes)?;
        let has_explicit_files = parsed.files.is_some();
        let Some(dest_root) = parsed.dest.select_for_current_platform(name)? else {
            return Ok(None);
        };
        let files = match parsed.files {
            Some(files) => {
                let mut filtered = Vec::new();
                for file in files {
                    if file_matches_current_platform(name, &file)? {
                        filtered.push(file);
                    }
                }
                filtered
                    .into_iter()
                    .map(|file| {
                        let context = metadata_path.display().to_string();
                        let source_rel = normalize_relative(&file.source)
                            .with_context(|| format!("invalid source for {context}"))?;
                        let target_rel =
                            normalize_relative(file.target.as_deref().unwrap_or(&file.source))
                                .with_context(|| format!("invalid target for {context}"))?;
                        let transforms = resolve_transforms(&file, &context)?;
                        let install_strategy = resolve_install_strategy(&file, &context)?;
                        Ok(AppFile {
                            source_rel,
                            target_rel,
                            description: file.description,
                            display_name: file.display_name,
                            legacy_dest_annotation: None,
                            transforms,
                            install_strategy,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?
            }
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
                    install_strategy: AppInstallStrategy::Copy,
                })
                .collect(),
        };
        if files.is_empty() {
            return Ok(None);
        }

        for file in &files {
            let source_path = category_root.join(&file.source_rel);
            if !source_path.exists() {
                bail!(
                    "app/{name}/shine.toml references missing file: {}",
                    file.source_rel.display()
                );
            }
        }

        return Ok(Some(AppCategory {
            name: name.to_string(),
            description: parsed.description,
            destination_root: Some(dest_root),
            files,
            list_mode: parsed
                .list_mode
                .map(Into::into)
                .unwrap_or_else(|| default_list_mode(has_explicit_files)),
            uses_metadata: true,
            has_explicit_files,
        }));
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
            install_strategy: AppInstallStrategy::Copy,
        });
    }

    Ok(Some(AppCategory {
        name: name.to_string(),
        description: None,
        destination_root: None,
        files,
        list_mode: AppListMode::Category,
        uses_metadata: false,
        has_explicit_files: false,
    }))
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

    if let Some(dest) = parsed.dest.select_for_current_platform(name)? {
        validate_dest(name, &dest)?;
    }
    if let Some(files) = &parsed.files {
        for file in files {
            file_matches_current_platform(name, file)?;
            let context = format!("app/{name}/shine.toml");
            resolve_transforms(file, &context)?;
            resolve_install_strategy(file, &context)?;
        }
    }
    Ok(parsed)
}

fn validate_dest(name: &str, dest: &str) -> Result<()> {
    let expanded = crate::config::full_expand(dest)
        .with_context(|| format!("failed to expand dest in app/{name}/shine.toml"))?;
    if !Path::new(&expanded).is_absolute() {
        bail!("app/{name}/shine.toml dest must be absolute after expansion");
    }
    let path = PathBuf::from(&expanded);
    if path.components().any(|c| c == Component::ParentDir) {
        bail!("app/{name}/shine.toml dest must not contain '..'");
    }
    Ok(())
}

impl DestToml {
    fn select_for_current_platform(&self, category: &str) -> Result<Option<String>> {
        self.select_for_platform(category, current_platform())
    }

    fn select_for_platform(&self, category: &str, current: &str) -> Result<Option<String>> {
        match self {
            Self::Single(dest) => Ok(Some(dest.clone())),
            Self::Platforms(dest) => dest.select_for_platform(category, current),
        }
    }
}

impl PlatformDestToml {
    fn select_for_platform(&self, category: &str, current: &str) -> Result<Option<String>> {
        match current {
            "windows" => Ok(self.windows.clone()),
            "unix" => Ok(self.unix.clone()),
            _ => bail!("app/{category}/shine.toml has unsupported current platform `{current}`"),
        }
    }
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
            "windows" | "unix" => matches |= normalized == current,
            _ => bail!(
                "app/{category}/shine.toml has unsupported platform `{platform}`; expected `windows` or `unix`"
            ),
        }
    }
    Ok(matches)
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
    fn embedded_docker_engine_has_jsonc_transform() {
        let categories = load_embedded_categories(Some("docker-engine")).unwrap();
        let docker = categories
            .iter()
            .find(|c| c.name == "docker-engine")
            .unwrap();
        assert!(docker.uses_metadata);
        #[cfg(windows)]
        assert_eq!(docker.destination_root.as_deref(), Some("~/.docker"));
        #[cfg(not(windows))]
        assert_eq!(docker.destination_root.as_deref(), Some("/etc/docker"));
        assert_eq!(docker.files.len(), 1);

        let file = &docker.files[0];
        assert_eq!(file.source_rel, std::path::Path::new("daemon.jsonc"));
        assert_eq!(file.target_rel, std::path::Path::new("daemon.json"));
        assert_eq!(file.transforms, vec!["template", "jsonc-to-json"]);
        assert_eq!(file.install_strategy, AppInstallStrategy::Copy);
    }

    #[test]
    fn embedded_docker_desktop_uses_json_merge_install_strategy() {
        let categories = load_embedded_categories(Some("docker-desktop")).unwrap();
        #[cfg(not(windows))]
        {
            assert!(categories.is_empty());
        }

        #[cfg(windows)]
        let docker = categories
            .iter()
            .find(|c| c.name == "docker-desktop")
            .unwrap();

        #[cfg(windows)]
        {
            assert!(docker.uses_metadata);
            assert_eq!(docker.files.len(), 1);
            let file = &docker.files[0];
            assert_eq!(file.target_rel, std::path::Path::new("settings-store.json"));
            assert_eq!(file.transforms, vec!["template", "jsonc-to-json"]);
            assert_eq!(
                file.install_strategy,
                AppInstallStrategy::JsonMerge {
                    managed_keys: vec!["proxy".to_string(), "containersProxy".to_string()],
                }
            );
        }
    }

    #[test]
    fn embedded_archey4_is_unix_only() {
        let categories = load_embedded_categories(Some("archey4")).unwrap();

        #[cfg(windows)]
        {
            assert!(categories.is_empty());
            return;
        }

        #[cfg(not(windows))]
        {
            let archey4 = categories.iter().find(|c| c.name == "archey4").unwrap();
            assert!(archey4.uses_metadata);
            assert_eq!(
                archey4.destination_root.as_deref(),
                Some("~/.config/archey4")
            );
        }
    }

    #[test]
    fn unix_absolute_dest_is_valid_on_all_platforms() {
        parse_category_toml("docker-engine", b"dest = \"/etc/docker\"\n").unwrap();
    }

    #[test]
    fn platform_dest_selects_current_platform() {
        let parsed = parse_category_toml(
            "docker-engine",
            b"[dest]\nwindows = \"~/.docker\"\nunix = \"/etc/docker\"\n",
        )
        .unwrap();

        #[cfg(windows)]
        assert_eq!(
            parsed
                .dest
                .select_for_current_platform("docker-engine")
                .unwrap(),
            Some("~/.docker".to_string())
        );
        #[cfg(not(windows))]
        assert_eq!(
            parsed
                .dest
                .select_for_current_platform("docker-engine")
                .unwrap(),
            Some("/etc/docker".to_string())
        );
    }

    #[test]
    fn unsupported_file_platform_is_rejected() {
        let err = parse_category_toml(
            "docker-engine",
            br#"
dest = "/etc/docker"

[[files]]
source = "daemon.jsonc"
platforms = ["plan9"]
"#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("unsupported platform"));
    }

    #[test]
    fn file_platform_filter_matches_expected_platforms() {
        let windows_only: FileToml = toml::from_str(
            r#"
source = "daemon.jsonc"
platforms = ["windows"]
"#,
        )
        .unwrap();
        let unix_only: FileToml = toml::from_str(
            r#"
source = "daemon.jsonc"
platforms = ["unix"]
"#,
        )
        .unwrap();

        assert!(file_matches_platform("docker-engine", &windows_only, "windows").unwrap());
        assert!(!file_matches_platform("docker-engine", &windows_only, "unix").unwrap());
        assert!(file_matches_platform("docker-engine", &unix_only, "unix").unwrap());
        assert!(!file_matches_platform("docker-engine", &unix_only, "windows").unwrap());
    }

    #[test]
    fn json_merge_requires_managed_keys() {
        let err = parse_category_toml(
            "docker-desktop",
            br#"
dest = "~/.docker/desktop"

[[files]]
source = "settings-store.jsonc"
target = "settings-store.json"
install_mode = "json-merge"
"#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("managed_keys"));
    }

    #[test]
    fn embedded_ghostty_has_theme_files_with_template_transform() {
        let categories = load_embedded_categories(Some("ghostty")).unwrap();

        #[cfg(windows)]
        {
            assert!(categories.is_empty());
            return;
        }

        #[cfg(not(windows))]
        {
            let ghostty = categories.iter().find(|c| c.name == "ghostty").unwrap();
            assert!(ghostty.uses_metadata);
            assert!(ghostty.has_explicit_files);
            assert_eq!(
                ghostty.destination_root.as_deref(),
                Some("~/.config/ghostty")
            );
            assert_eq!(ghostty.list_mode, AppListMode::Category);
            assert_eq!(ghostty.files.len(), 5);

            let light = ghostty
                .files
                .iter()
                .find(|f| f.source_rel == std::path::Path::new("themes/iTerm2 Solarized Light"))
                .unwrap();
            assert_eq!(
                light.target_rel,
                std::path::Path::new("themes/light_iTerm2 Solarized Light")
            );
            assert_eq!(light.transforms, vec!["template"]);

            let dark = ghostty
                .files
                .iter()
                .find(|f| f.source_rel == std::path::Path::new("themes/Alien Blood"))
                .unwrap();
            assert_eq!(
                dark.target_rel,
                std::path::Path::new("themes/dark_Alien Blood")
            );
            assert_eq!(dark.transforms, vec!["template"]);

            let atom = ghostty
                .files
                .iter()
                .find(|f| f.source_rel == std::path::Path::new("themes/Atom One Light"))
                .unwrap();
            assert_eq!(
                atom.target_rel,
                std::path::Path::new("themes/light_Atom One Light")
            );
            assert_eq!(atom.transforms, vec!["template"]);

            let github = ghostty
                .files
                .iter()
                .find(|f| f.source_rel == std::path::Path::new("themes/Github Light Default"))
                .unwrap();
            assert_eq!(
                github.target_rel,
                std::path::Path::new("themes/light_Github Light Default")
            );
            assert_eq!(github.transforms, vec!["template"]);
        }
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
                    platforms: None,
                    transform: Some("no-such-transform".to_string()),
                    transforms: None,
                    install_mode: None,
                    managed_keys: None,
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
            platforms: None,
            transform: Some("jsonc-to-json".to_string()),
            transforms: Some(vec!["jsonc-to-json".to_string()]),
            install_mode: None,
            managed_keys: None,
        };
        assert!(resolve_transforms(&file, "test").is_err());
    }
}
