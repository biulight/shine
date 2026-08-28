use crate::config::Config;
use crate::platform::{OperatingSystem, current_platform};
use crate::presets;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
#[cfg(test)]
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use tokio::fs;

use crate::preset_validation::PresetValidationFailure;

#[derive(Debug, Clone)]
pub struct ShellCategory {
    pub name: String,
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
    /// How the command is invoked: native (symlink/shim) or via the `bun` runtime.
    pub runtime: crate::bin_links::LinkRuntime,
    /// Declared install-time transforms (e.g. `["template"]`) applied to the source
    /// before linking. Empty means "no metadata-declared transform" — native scripts
    /// may still opt into templating via the `# shine-template: true` annotation.
    pub transforms: Vec<String>,
    /// Runtime environment values injected into a Bun launcher via `Bun.env`,
    /// using the `env run --with` grammar (ordered). Empty for native entries;
    /// only `runtime = "bun"` entries may declare it (enforced at metadata load).
    pub env: Vec<crate::env::EnvVarSpec>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShellTarget<'a> {
    pub category: &'a str,
    pub command: Option<&'a str>,
}

/// Parse the scoped shell lifecycle grammar: `category` or `category/command`.
/// Bare command aliases remain inspection-only so mutation targets stay unambiguous.
pub fn parse_lifecycle_target(target: &str) -> Result<ShellTarget<'_>> {
    let target = target.trim();
    if target.is_empty() {
        bail!("shell preset target must not be empty");
    }
    let mut parts = target.split('/');
    let category = parts.next().unwrap_or_default();
    let command = parts.next();
    if category.is_empty() || command.is_some_and(str::is_empty) || parts.next().is_some() {
        bail!(
            "invalid shell preset target `{target}`; expected <category> or <category>/<command>"
        );
    }
    Ok(ShellTarget { category, command })
}

/// Select and validate one shell lifecycle target from the active preset namespace.
pub async fn load_active_target(
    config: &Config,
    target: ShellTarget<'_>,
) -> Result<Vec<ShellCategory>> {
    let mut categories = load_active_categories(config, Some(target.category)).await?;
    let Some(category) = categories.first_mut() else {
        bail!("shell preset category not found: {}", target.category);
    };
    if let Some(command) = target.command {
        category.files.retain(|file| file.command_name == command);
        if category.files.is_empty() {
            bail!(
                "shell preset command not found: {}/{}",
                target.category,
                command
            );
        }
    }
    Ok(categories)
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
    description: Option<String>,
    needs_source: Option<bool>,
    platforms: Option<Vec<String>>,
    runtime: Option<String>,
    transforms: Option<Vec<String>>,
    env: Option<Vec<String>>,
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
                    let ctx = format!("shell/{name}/shine.toml");
                    let resolved = resolve_metadata_file(&file, &ctx)?;
                    let asset_path = format!("shell/{name}/{}", resolved.source_rel.display());
                    let bytes = presets::read_asset_bytes(&asset_path).with_context(|| {
                        format!(
                            "shell/{name}/shine.toml references missing file: {:?}",
                            resolved.source_rel
                        )
                    })?;
                    let description = resolved.describe(&bytes);
                    Ok(resolved.into_shell_file(description))
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
                        runtime: crate::bin_links::LinkRuntime::Native,
                        transforms: Vec::new(),
                        env: Vec::new(),
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
                    runtime: crate::bin_links::LinkRuntime::Native,
                    transforms: Vec::new(),
                    env: Vec::new(),
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
                    let ctx = metadata_path.display().to_string();
                    resolve_metadata_file(&file, &ctx)
                })
                .collect::<Result<Vec<_>>>()?,
            None => collect_merged_fs_scripts(config, &category_rel)
                .await?
                .into_iter()
                .map(|source_rel| {
                    let command_name = default_command_name(&source_rel)?;
                    Ok(ResolvedFile::native(source_rel, command_name))
                })
                .collect::<Result<Vec<_>>>()?,
        };

        let mut shell_files = Vec::new();
        for resolved in files {
            let source_path = config.preset_path(category_rel.join(&resolved.source_rel));
            if !source_path.exists() {
                bail!(
                    "shell/{name}/shine.toml references missing file: {}",
                    resolved.source_rel.display()
                );
            }
            let bytes = fs::read(&source_path)
                .await
                .with_context(|| format!("reading preset file: {}", source_path.display()))?;
            let description = resolved.describe(&bytes);
            shell_files.push(resolved.into_shell_file(description));
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
            runtime: crate::bin_links::LinkRuntime::Native,
            transforms: Vec::new(),
            env: Vec::new(),
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

fn file_matches_platform(
    category: &str,
    file: &FileToml,
    current: OperatingSystem,
) -> Result<bool> {
    crate::preset_meta::platform_matches(
        file.platforms.as_deref(),
        current,
        &format!("shell/{category}/shine.toml"),
    )
}

#[cfg(test)]
pub(crate) fn built_in_platform_availability() -> Result<BTreeMap<String, BTreeSet<OperatingSystem>>>
{
    let mut capabilities: BTreeMap<String, BTreeSet<OperatingSystem>> = BTreeMap::new();
    for name in crate::preset_meta::collect_pristine_embedded_category_names("shell") {
        let metadata_path = format!("shell/{name}/shine.toml");
        if let Some(bytes) = presets::read_embedded_asset_bytes(&metadata_path) {
            let parsed = parse_category_toml(&name, &bytes)?;
            if let Some(files) = parsed.files {
                for file in files {
                    let command = resolve_metadata_file(&file, &metadata_path)?.command_name;
                    let platforms = capabilities
                        .entry(format!("shell/{name}/{command}"))
                        .or_default();
                    for platform in OperatingSystem::ALL {
                        if file_matches_platform(&name, &file, platform)? {
                            platforms.insert(platform);
                        }
                    }
                }
                continue;
            }
        }

        let prefix = format!("shell/{name}/");
        for asset_path in presets::embedded_asset_paths(&format!("shell/{name}")) {
            let Some(relative) = asset_path.strip_prefix(&prefix) else {
                continue;
            };
            let source = PathBuf::from(relative);
            if relative == "shine.toml" || !is_shell_script(&source) {
                continue;
            }
            let command = default_command_name(&source)?;
            capabilities.insert(
                format!("shell/{name}/{command}"),
                OperatingSystem::ALL.into_iter().collect(),
            );
        }
    }
    Ok(capabilities)
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

/// Validate a `[[files]]` `source` as a safe relative path (no absolute, no `..`,
/// not `shine.toml`) without checking its extension.
fn normalize_relative_source(path: impl AsRef<Path>) -> Result<PathBuf> {
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
    Ok(normalized)
}

/// Native (auto-collected or `runtime = "native"`) source: relative + `.sh`/`.ps1`.
fn normalize_shell_source(path: impl AsRef<Path>) -> Result<PathBuf> {
    let normalized = normalize_relative_source(path)?;
    if !is_shell_script(&normalized) {
        bail!("source path must end with .sh or .ps1");
    }
    Ok(normalized)
}

/// Metadata source validated against the declared runtime's allowed extensions.
fn normalize_source(
    path: impl AsRef<Path>,
    runtime: crate::bin_links::LinkRuntime,
) -> Result<PathBuf> {
    let normalized = normalize_relative_source(path)?;
    match runtime {
        crate::bin_links::LinkRuntime::Native => {
            if !is_shell_script(&normalized) {
                bail!("source path must end with .sh or .ps1");
            }
        }
        crate::bin_links::LinkRuntime::Bun => {
            if !is_bun_script(&normalized) {
                bail!("bun source path must end with .ts, .js, .mts, or .mjs");
            }
        }
    }
    Ok(normalized)
}

fn is_shell_script(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("sh" | "ps1")
    )
}

fn is_bun_script(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("ts" | "js" | "mts" | "mjs")
    )
}

fn parse_runtime(value: Option<&str>) -> Result<crate::bin_links::LinkRuntime> {
    match value {
        None | Some("native") => Ok(crate::bin_links::LinkRuntime::Native),
        Some("bun") => Ok(crate::bin_links::LinkRuntime::Bun),
        Some(other) => bail!("unsupported runtime `{other}` (expected `bun`)"),
    }
}

/// A validated `[[files]]` entry, before its script bytes are read for the
/// description. Shared by the embedded and installed metadata loaders.
struct ResolvedFile {
    source_rel: PathBuf,
    command_name: String,
    /// Optional `description = "..."` from `[[files]]`; when set it overrides the
    /// description parsed from the source's leading comment block.
    description: Option<String>,
    needs_source: bool,
    runtime: crate::bin_links::LinkRuntime,
    transforms: Vec<String>,
    env: Vec<crate::env::EnvVarSpec>,
}

impl ResolvedFile {
    fn native(source_rel: PathBuf, command_name: String) -> Self {
        Self {
            source_rel,
            command_name,
            description: None,
            needs_source: false,
            runtime: crate::bin_links::LinkRuntime::Native,
            transforms: Vec::new(),
            env: Vec::new(),
        }
    }

    /// Resolve the command description from the source `bytes`: an explicit
    /// metadata `description` wins; otherwise parse the source's leading comment
    /// block with the runtime-correct leader (`//` for bun, `#` for native).
    fn describe(&self, bytes: &[u8]) -> Vec<String> {
        if let Some(description) = &self.description {
            return vec![description.clone()];
        }
        match self.runtime {
            crate::bin_links::LinkRuntime::Bun => presets::parse_bun_description(bytes),
            crate::bin_links::LinkRuntime::Native => presets::parse_script_description(bytes),
        }
    }

    fn into_shell_file(self, description: Vec<String>) -> ShellFile {
        ShellFile {
            source_rel: self.source_rel,
            command_name: self.command_name,
            description,
            needs_source: self.needs_source,
            runtime: self.runtime,
            transforms: self.transforms,
            env: self.env,
        }
    }
}

fn resolve_metadata_file(file: &FileToml, ctx: &str) -> Result<ResolvedFile> {
    let runtime = parse_runtime(file.runtime.as_deref())
        .with_context(|| format!("invalid runtime in {ctx}"))?;
    let needs_source = file.needs_source.unwrap_or(false);
    if runtime == crate::bin_links::LinkRuntime::Bun && needs_source {
        bail!("{ctx}: `runtime = \"bun\"` cannot be combined with `needs_source = true`");
    }
    let source_rel = normalize_source(&file.source, runtime)
        .with_context(|| format!("invalid source in {ctx}"))?;
    let command_name = resolve_command_name(&source_rel, file.target.as_deref())
        .with_context(|| format!("invalid target in {ctx}"))?;
    let transforms = file.transforms.clone().unwrap_or_default();
    let env = crate::env::parse_env_specs(file.env.as_deref().unwrap_or_default())
        .with_context(|| format!("invalid env in {ctx}"))?;
    if runtime != crate::bin_links::LinkRuntime::Bun && !env.is_empty() {
        bail!("{ctx}: `env` is only valid when `runtime = \"bun\"`");
    }
    Ok(ResolvedFile {
        source_rel,
        command_name,
        description: file.description.clone(),
        needs_source,
        runtime,
        transforms,
        env,
    })
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

/// Validate one on-disk shell category without loading configuration or
/// executing any command. Returns whether the category has explicit metadata.
pub(crate) fn validate_preset_category(
    name: &str,
    root: &Path,
) -> std::result::Result<bool, PresetValidationFailure> {
    let manifest_path = root.join("shine.toml");
    if !manifest_path.is_file() {
        let sources = collect_local_scripts(root)?;
        validate_legacy_commands(root, &sources)?;
        return Ok(false);
    }

    let bytes = std::fs::read(&manifest_path).map_err(|error| {
        PresetValidationFailure::at(
            "read_failed",
            format!("cannot read shell metadata: {error}"),
            &manifest_path,
        )
    })?;
    let parsed: CategoryToml = toml::from_slice(&bytes).map_err(|error| {
        PresetValidationFailure::at(
            "invalid_metadata",
            format!("failed to parse shell/{name}/shine.toml: {error}"),
            &manifest_path,
        )
    })?;
    let context = format!("shell/{name}/shine.toml");

    let mut resolved = Vec::new();
    match &parsed.files {
        Some(files) if files.is_empty() => {
            return Err(PresetValidationFailure::at(
                "invalid_metadata",
                format!("{context} files must not be empty"),
                &manifest_path,
            ));
        }
        Some(files) => {
            for file in files {
                let entry = resolve_metadata_file(file, &context)
                    .map_err(|error| invalid_metadata(error, &manifest_path))?;
                crate::install_core::transforms::validate(&entry.transforms)
                    .with_context(|| format!("invalid transforms in {context}"))
                    .map_err(|error| invalid_metadata(error, &manifest_path))?;
                validate_reference(root, &entry.source_rel, "shell source")?;
                for platform in OperatingSystem::ALL {
                    file_matches_platform(name, file, platform)
                        .map_err(|error| invalid_metadata(error, &manifest_path))?;
                }
                resolved.push((file.platforms.clone(), entry));
            }
        }
        None => {
            for source in collect_local_scripts(root)? {
                let command_name = default_command_name(&source)
                    .map_err(|error| invalid_metadata(error, &manifest_path))?;
                // Auto-collected entries apply to both platforms, matching the
                // runtime loader's compatibility behavior.
                resolved.push((None, ResolvedFile::native(source, command_name)));
            }
        }
    }

    let uses_bun = resolved
        .iter()
        .any(|(_, entry)| entry.runtime == crate::bin_links::LinkRuntime::Bun);
    for platform in OperatingSystem::ALL {
        let mut commands = BTreeSet::new();
        for (platforms, entry) in &resolved {
            if !crate::preset_meta::platform_matches(platforms.as_deref(), platform, &context)
                .map_err(|error| invalid_metadata(error, &manifest_path))?
            {
                continue;
            }
            if !commands.insert(entry.command_name.clone()) {
                return Err(PresetValidationFailure::at(
                    "duplicate_command",
                    format!(
                        "shell/{name} declares command `{}` more than once for {}",
                        entry.command_name,
                        platform.as_str()
                    ),
                    &manifest_path,
                ));
            }
        }
    }

    if uses_bun {
        crate::bun_runtime::resolve(root, true).map_err(|error| {
            PresetValidationFailure::at("bun_dependency_policy", error.to_string(), root)
        })?;
    }
    Ok(true)
}

fn invalid_metadata(error: anyhow::Error, path: &Path) -> PresetValidationFailure {
    PresetValidationFailure::at("invalid_metadata", error.to_string(), path)
}

fn collect_local_scripts(
    root: &Path,
) -> std::result::Result<Vec<PathBuf>, PresetValidationFailure> {
    let mut scripts = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = std::fs::read_dir(&directory).map_err(|error| {
            PresetValidationFailure::at(
                "read_failed",
                format!("cannot read shell category: {error}"),
                &directory,
            )
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                PresetValidationFailure::at("read_failed", error.to_string(), &directory)
            })?;
            let file_type = entry.file_type().map_err(|error| {
                PresetValidationFailure::at("read_failed", error.to_string(), entry.path())
            })?;
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .map(Path::to_path_buf)
                    .map_err(|error| {
                        PresetValidationFailure::at("read_failed", error.to_string(), entry.path())
                    })?;
                if is_shell_script(&relative) {
                    scripts.push(relative);
                }
            }
        }
    }
    scripts.sort();
    if scripts.is_empty() {
        return Err(PresetValidationFailure::at(
            "no_files",
            "shell preset category contains no .sh or .ps1 files",
            root,
        ));
    }
    Ok(scripts)
}

fn validate_legacy_commands(
    root: &Path,
    sources: &[PathBuf],
) -> std::result::Result<(), PresetValidationFailure> {
    let mut commands = BTreeSet::new();
    for source in sources {
        let command = default_command_name(source).map_err(|error| {
            PresetValidationFailure::at("invalid_metadata", error.to_string(), root.join(source))
        })?;
        if !commands.insert(command.clone()) {
            return Err(PresetValidationFailure::at(
                "duplicate_command",
                format!("legacy shell category resolves more than one file to `{command}`"),
                root,
            ));
        }
    }
    Ok(())
}

fn validate_reference(
    root: &Path,
    relative: &Path,
    label: &str,
) -> std::result::Result<(), PresetValidationFailure> {
    let path = root.join(relative);
    let canonical = std::fs::canonicalize(&path).map_err(|error| {
        PresetValidationFailure::at(
            "missing_reference",
            format!("{label} is missing or unreadable: {error}"),
            &path,
        )
    })?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(PresetValidationFailure::at(
            "invalid_reference",
            format!("{label} must be a file inside the preset category"),
            path,
        ));
    }
    Ok(())
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
            description: None,
            needs_source: Some(true),
            platforms: Some(vec!["windows".to_string()]),
            runtime: None,
            transforms: None,
            env: None,
        };

        assert!(file_matches_platform("proxy", &file, OperatingSystem::Windows).unwrap());
        assert!(!file_matches_platform("proxy", &file, OperatingSystem::Linux).unwrap());
    }

    #[test]
    fn metadata_platform_filter_defaults_to_all_platforms() {
        let file = FileToml {
            source: "set_proxy.sh".to_string(),
            target: Some("setproxy".to_string()),
            description: None,
            needs_source: Some(true),
            platforms: None,
            runtime: None,
            transforms: None,
            env: None,
        };

        assert!(file_matches_platform("proxy", &file, OperatingSystem::Windows).unwrap());
        assert!(file_matches_platform("proxy", &file, OperatingSystem::Macos).unwrap());
        assert!(file_matches_platform("proxy", &file, OperatingSystem::Linux).unwrap());
    }

    #[test]
    fn metadata_platform_filter_rejects_unknown_platforms() {
        let file = FileToml {
            source: "set_proxy.sh".to_string(),
            target: Some("setproxy".to_string()),
            description: None,
            needs_source: Some(true),
            platforms: Some(vec!["plan9".to_string()]),
            runtime: None,
            transforms: None,
            env: None,
        };

        let err = file_matches_platform("proxy", &file, OperatingSystem::Linux)
            .unwrap_err()
            .to_string();
        assert!(err.contains("unsupported platform `plan9`"));
    }

    #[test]
    fn embedded_agent_category_uses_cross_platform_bun_entry() {
        let categories = load_embedded_categories(Some("agent")).unwrap();
        let agent = categories.iter().find(|cat| cat.name == "agent").unwrap();

        assert_eq!(agent.files.len(), 1);
        assert_eq!(agent.files[0].command_name, "ccenv");
        assert_eq!(agent.files[0].source_rel, PathBuf::from("cc.ts"));
        assert!(!agent.files[0].needs_source);
        assert_eq!(agent.files[0].runtime, crate::bin_links::LinkRuntime::Bun);
        assert!(agent.files[0].transforms.is_empty());
        assert!(agent.files[0].env.is_empty());
    }

    #[test]
    fn embedded_image_tools_category_exposes_cross_platform_bun_commands() {
        let categories = load_embedded_categories(Some("image-tools")).unwrap();
        let category = categories
            .iter()
            .find(|category| category.name == "image-tools")
            .unwrap();
        let commands: Vec<_> = category
            .files
            .iter()
            .map(|file| {
                (
                    file.command_name.as_str(),
                    file.runtime,
                    file.env
                        .iter()
                        .map(|spec| spec.source.as_str())
                        .collect::<Vec<_>>(),
                )
            })
            .collect();

        assert_eq!(
            commands,
            vec![
                (
                    "img-compress",
                    crate::bin_links::LinkRuntime::Bun,
                    vec!["IMAGE_QUALITY"]
                ),
                (
                    "img-resize",
                    crate::bin_links::LinkRuntime::Bun,
                    vec!["IMAGE_QUALITY", "IMAGE_MAX_WIDTH", "IMAGE_MAX_HEIGHT"],
                ),
                (
                    "img-convert",
                    crate::bin_links::LinkRuntime::Bun,
                    vec!["IMAGE_QUALITY"]
                ),
            ]
        );
    }

    #[test]
    fn embedded_utils_category_exposes_copyfile_command() {
        let categories = load_embedded_categories(Some("utils")).unwrap();
        let utils = categories.iter().find(|cat| cat.name == "utils").unwrap();

        if cfg!(windows) {
            assert_eq!(utils.files.len(), 2);
            let env_export = utils
                .files
                .iter()
                .find(|f| f.command_name == "shine-env-export")
                .expect("shine-env-export should be present");
            assert!(env_export.needs_source);

            let theme_sync = utils
                .files
                .iter()
                .find(|f| f.command_name == "shine-theme-sync")
                .expect("shine-theme-sync should be present");
            assert_eq!(theme_sync.source_rel, PathBuf::from("shine-theme-sync.ps1"));
            assert!(theme_sync.needs_source);
        } else {
            assert_eq!(utils.files.len(), 3);
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

            let theme_sync = utils
                .files
                .iter()
                .find(|f| f.command_name == "shine-theme-sync")
                .expect("shine-theme-sync should be present");
            assert_eq!(theme_sync.source_rel, PathBuf::from("shine-theme-sync.sh"));
            assert!(theme_sync.needs_source);
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

    #[test]
    fn parse_runtime_accepts_native_and_bun_rejects_others() {
        use crate::bin_links::LinkRuntime;
        assert_eq!(parse_runtime(None).unwrap(), LinkRuntime::Native);
        assert_eq!(parse_runtime(Some("native")).unwrap(), LinkRuntime::Native);
        assert_eq!(parse_runtime(Some("bun")).unwrap(), LinkRuntime::Bun);
        let err = parse_runtime(Some("deno")).unwrap_err().to_string();
        assert!(err.contains("unsupported runtime"));
    }

    #[test]
    fn normalize_source_enforces_extension_per_runtime() {
        use crate::bin_links::LinkRuntime;
        for ext in ["ts", "js", "mts", "mjs"] {
            assert!(
                normalize_source(format!("tool.{ext}"), LinkRuntime::Bun).is_ok(),
                ".{ext} should be a valid bun source"
            );
        }
        assert!(normalize_source("tool.sh", LinkRuntime::Bun).is_err());
        assert!(normalize_source("tool.ts", LinkRuntime::Native).is_err());
        assert!(normalize_source("tool.sh", LinkRuntime::Native).is_ok());
        // Path traversal is rejected regardless of runtime.
        assert!(normalize_source("../evil.ts", LinkRuntime::Bun).is_err());
    }

    async fn write_bun_category(dir: &Path, shine_toml: &[u8]) -> Config {
        let category_root = dir.join("presets/shell/custom");
        fs::create_dir_all(&category_root).await.unwrap();
        fs::write(category_root.join("shine.toml"), shine_toml)
            .await
            .unwrap();
        fs::write(category_root.join("tool.ts"), b"// tool\n")
            .await
            .unwrap();
        let mut config = Config::new_for_test(dir);
        config.is_external_presets = true;
        config
    }

    #[tokio::test]
    async fn installed_metadata_accepts_bun_runtime_with_transforms() {
        let dir = make_temp_dir().await;
        let config = write_bun_category(
            &dir,
            b"[[files]]\nsource = \"tool.ts\"\ntarget = \"mytool\"\nruntime = \"bun\"\ntransforms = [\"template\"]\n",
        )
        .await;

        let categories = load_installed_categories(&config, Some("custom"))
            .await
            .unwrap();
        let file = &categories[0].files[0];
        assert_eq!(file.command_name, "mytool");
        assert_eq!(file.source_rel, PathBuf::from("tool.ts"));
        assert_eq!(file.runtime, crate::bin_links::LinkRuntime::Bun);
        assert_eq!(file.transforms, vec!["template".to_string()]);
        assert!(!file.needs_source);

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn installed_metadata_defaults_bun_command_name_to_stem() {
        let dir = make_temp_dir().await;
        let config = write_bun_category(
            &dir,
            b"[[files]]\nsource = \"tool.ts\"\nruntime = \"bun\"\n",
        )
        .await;

        let categories = load_installed_categories(&config, Some("custom"))
            .await
            .unwrap();
        assert_eq!(categories[0].files[0].command_name, "tool");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn installed_metadata_rejects_bun_with_needs_source() {
        let dir = make_temp_dir().await;
        let config = write_bun_category(
            &dir,
            b"[[files]]\nsource = \"tool.ts\"\nruntime = \"bun\"\nneeds_source = true\n",
        )
        .await;

        let err = load_installed_categories(&config, Some("custom"))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("needs_source"), "unexpected error: {err}");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn installed_metadata_rejects_unknown_runtime() {
        let dir = make_temp_dir().await;
        let config = write_bun_category(
            &dir,
            b"[[files]]\nsource = \"tool.ts\"\nruntime = \"deno\"\n",
        )
        .await;

        let err = format!(
            "{:#}",
            load_installed_categories(&config, Some("custom"))
                .await
                .unwrap_err()
        );
        assert!(
            err.contains("unsupported runtime"),
            "unexpected error: {err}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn installed_metadata_parses_bun_env_declarations_in_order() {
        let dir = make_temp_dir().await;
        let config = write_bun_category(
            &dir,
            b"[[files]]\nsource = \"tool.ts\"\ntarget = \"mytool\"\nruntime = \"bun\"\nenv = [\"API_URL\", \"SERVICE_TOKEN=API_TOKEN\"]\n",
        )
        .await;

        let categories = load_installed_categories(&config, Some("custom"))
            .await
            .unwrap();
        let env = &categories[0].files[0].env;
        assert_eq!(env.len(), 2);
        assert_eq!(env[0].to_with_arg(), "API_URL");
        assert_eq!(env[1].to_with_arg(), "SERVICE_TOKEN=API_TOKEN");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn installed_metadata_rejects_env_on_native_entry() {
        let dir = make_temp_dir().await;
        let category_root = dir.join("presets/shell/custom");
        fs::create_dir_all(&category_root).await.unwrap();
        fs::write(
            category_root.join("shine.toml"),
            b"[[files]]\nsource = \"tool.sh\"\nenv = [\"API_URL\"]\n",
        )
        .await
        .unwrap();
        fs::write(category_root.join("tool.sh"), b"#!/bin/bash\n")
            .await
            .unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        let err = format!(
            "{:#}",
            load_installed_categories(&config, Some("custom"))
                .await
                .unwrap_err()
        );
        assert!(
            err.contains("`env` is only valid when `runtime = \"bun\"`"),
            "unexpected error: {err}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn installed_metadata_rejects_bun_env_invalid_name() {
        let dir = make_temp_dir().await;
        let config = write_bun_category(
            &dir,
            b"[[files]]\nsource = \"tool.ts\"\nruntime = \"bun\"\nenv = [\"BAD-NAME\"]\n",
        )
        .await;

        let err = format!(
            "{:#}",
            load_installed_categories(&config, Some("custom"))
                .await
                .unwrap_err()
        );
        assert!(
            err.contains("invalid environment variable name"),
            "unexpected error: {err}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn installed_metadata_rejects_bun_env_duplicate_target() {
        let dir = make_temp_dir().await;
        let config = write_bun_category(
            &dir,
            b"[[files]]\nsource = \"tool.ts\"\nruntime = \"bun\"\nenv = [\"A=TOKEN\", \"B=TOKEN\"]\n",
        )
        .await;

        let err = format!(
            "{:#}",
            load_installed_categories(&config, Some("custom"))
                .await
                .unwrap_err()
        );
        assert!(
            err.contains("duplicate target variable"),
            "unexpected error: {err}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn installed_metadata_bun_description_from_slash_header() {
        let dir = make_temp_dir().await;
        let category_root = dir.join("presets/shell/custom");
        fs::create_dir_all(&category_root).await.unwrap();
        fs::write(
            category_root.join("shine.toml"),
            b"[[files]]\nsource = \"tool.ts\"\ntarget = \"mytool\"\nruntime = \"bun\"\n",
        )
        .await
        .unwrap();
        fs::write(
            category_root.join("tool.ts"),
            b"// Fetch and print status.\n// Reads Bun.env.API_URL.\nconsole.log('hi')\n",
        )
        .await
        .unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        let categories = load_installed_categories(&config, Some("custom"))
            .await
            .unwrap();
        assert_eq!(
            categories[0].files[0].description,
            vec!["Fetch and print status.", "Reads Bun.env.API_URL."]
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn installed_metadata_file_description_overrides_bun_header() {
        let dir = make_temp_dir().await;
        let category_root = dir.join("presets/shell/custom");
        fs::create_dir_all(&category_root).await.unwrap();
        fs::write(
            category_root.join("shine.toml"),
            b"[[files]]\nsource = \"tool.ts\"\ntarget = \"mytool\"\nruntime = \"bun\"\ndescription = \"Explicit metadata description.\"\n",
        )
        .await
        .unwrap();
        fs::write(
            category_root.join("tool.ts"),
            b"// header that should be overridden\nconsole.log('hi')\n",
        )
        .await
        .unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        let categories = load_installed_categories(&config, Some("custom"))
            .await
            .unwrap();
        assert_eq!(
            categories[0].files[0].description,
            vec!["Explicit metadata description."]
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn installed_metadata_file_description_overrides_native_hash_header() {
        let dir = make_temp_dir().await;
        let category_root = dir.join("presets/shell/custom");
        fs::create_dir_all(&category_root).await.unwrap();
        fs::write(
            category_root.join("shine.toml"),
            b"[[files]]\nsource = \"tool.sh\"\ntarget = \"mytool\"\ndescription = \"From metadata.\"\n",
        )
        .await
        .unwrap();
        fs::write(
            category_root.join("tool.sh"),
            b"#!/bin/bash\n# hash header that should be overridden\necho hi\n",
        )
        .await
        .unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        let categories = load_installed_categories(&config, Some("custom"))
            .await
            .unwrap();
        assert_eq!(categories[0].files[0].description, vec!["From metadata."]);

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn installed_metadata_rejects_bun_source_with_shell_extension() {
        let dir = make_temp_dir().await;
        let category_root = dir.join("presets/shell/custom");
        fs::create_dir_all(&category_root).await.unwrap();
        fs::write(
            category_root.join("shine.toml"),
            b"[[files]]\nsource = \"tool.sh\"\nruntime = \"bun\"\n",
        )
        .await
        .unwrap();
        fs::write(category_root.join("tool.sh"), b"#!/bin/bash\n")
            .await
            .unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        let err = format!(
            "{:#}",
            load_installed_categories(&config, Some("custom"))
                .await
                .unwrap_err()
        );
        assert!(err.contains("bun source path"), "unexpected error: {err}");

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
