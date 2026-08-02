use super::manifest::AppInstallStrategy;
use crate::config::Config;
use crate::env::EnvVarSpec;
use crate::platform::current_platform;
use crate::presets;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use tokio::fs;

#[derive(Debug, Clone)]
pub struct AppCategory {
    pub name: String,
    pub description: Option<String>,
    pub destination_root: Option<String>,
    pub files: Vec<AppFile>,
    pub list_mode: AppListMode,
    pub post_upgrade: Vec<AppHook>,
    /// Hooks run after `shine app install` (including `--replace-managed`) when at least one file in
    /// this category actually changed — the install-time counterpart to
    /// `post_upgrade` (which only fires on `shine upgrade`).
    pub post_install: Vec<AppHook>,
    // Tracks whether the category came from an explicit metadata file vs. auto-collection;
    // reserved for future upgrade/list logic.
    #[allow(dead_code)]
    pub uses_metadata: bool,
    /// `true` when shine.toml has an explicit `[[files]]` section;
    /// `false` for auto-collected files and legacy categories.
    pub has_explicit_files: bool,
    pub artifact: Option<AppArtifact>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppListMode {
    Category,
    Files,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppHook {
    pub command: String,
    pub args: Vec<String>,
    /// Print this hook's stdout to the user when it succeeds. Defaults to
    /// `false` (silent) — most hooks (e.g. `surge-cli reload`) have nothing
    /// worth surfacing; opt in for hooks whose stdout is a deliberate note.
    pub show_output: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArtifactRuntime {
    /// The script is executed directly (`Command::new(script)`), relying on its
    /// shebang. Unix-only in practice — fine for macOS-only presets (e.g. surge).
    #[default]
    Native,
    /// The script is run via `bun <script>`, so it works on macOS/Windows/Linux
    /// (like shine's bun shell presets). `bun` is an external prerequisite.
    Bun,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppArtifact {
    pub script: String,
    /// Optional companion script that reverses `script`'s side-effects. Run
    /// explicitly via `shine app artifact remove <id>` and implicitly (best-effort)
    /// during `shine app uninstall`. Shares `build`'s full env contract.
    pub teardown: Option<String>,
    /// How `script`/`teardown` are launched. `Bun` makes the artifact
    /// cross-platform; `Native` execs the file directly (Unix-only).
    pub runtime: ArtifactRuntime,
}

#[derive(Debug, Clone)]
pub struct AppFile {
    pub source_rel: PathBuf,
    pub target_rel: PathBuf,
    pub description: Option<String>,
    pub display_name: Option<String>,
    pub legacy_dest_annotation: Option<String>,
    pub transforms: Vec<String>,
    pub install_strategy: AppInstallStrategy,
    pub requires_admin: bool,
    pub restart_hint: Option<String>,
    pub generator: Option<AppGenerator>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppGenerator {
    pub script: PathBuf,
    pub runtime: ArtifactRuntime,
    pub env: Vec<EnvVarSpec>,
    pub when_env: String,
    /// Whether read-oriented status checks and `shine upgrade` may run this
    /// generator implicitly. Install (including `--replace-managed`) and `app refresh` ignore
    /// this switch. Defaults to true for compatibility with existing presets.
    pub auto: bool,
}

#[derive(Debug, Deserialize)]
struct CategoryToml {
    description: Option<String>,
    dest: DestToml,
    list_mode: Option<ListModeToml>,
    post_upgrade: Option<HookSpecToml>,
    post_install: Option<HookSpecToml>,
    artifact: Option<ArtifactToml>,
    files: Option<Vec<FileToml>>,
}

#[derive(Debug, Clone, Deserialize)]
struct ArtifactToml {
    script: String,
    #[serde(default)]
    teardown: Option<String>,
    #[serde(default)]
    runtime: Option<ArtifactRuntimeToml>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ArtifactRuntimeToml {
    Native,
    Bun,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum HookSpecToml {
    Single(HookToml),
    Multiple(Vec<HookToml>),
}

#[derive(Debug, Clone, Deserialize)]
struct HookToml {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    show_output: bool,
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
    #[serde(default)]
    requires_admin: bool,
    restart_hint: Option<String>,
    generator: Option<GeneratorToml>,
}

#[derive(Debug, Clone, Deserialize)]
struct GeneratorToml {
    script: String,
    #[serde(default)]
    runtime: Option<ArtifactRuntimeToml>,
    #[serde(default)]
    env: Vec<String>,
    when_env: String,
    #[serde(default = "default_true")]
    auto: bool,
}

fn default_true() -> bool {
    true
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

fn resolve_hooks(hook: Option<HookSpecToml>, field: &str, context: &str) -> Result<Vec<AppHook>> {
    let Some(hook) = hook else {
        return Ok(Vec::new());
    };
    let hooks = match hook {
        HookSpecToml::Single(hook) => vec![hook],
        HookSpecToml::Multiple(hooks) => hooks,
    };
    if hooks.is_empty() {
        bail!("{context}: {field} must not be empty");
    }
    let mut resolved = Vec::with_capacity(hooks.len());
    for hook in hooks {
        if hook.command.trim().is_empty() {
            bail!("{context}: {field}.command must not be empty");
        }
        resolved.push(AppHook {
            command: hook.command,
            args: hook.args,
            show_output: hook.show_output,
        });
    }
    Ok(resolved)
}

fn resolve_artifact(artifact: Option<ArtifactToml>, context: &str) -> Result<Option<AppArtifact>> {
    let Some(artifact) = artifact else {
        return Ok(None);
    };
    if artifact.script.trim().is_empty() {
        bail!("{context}: artifact.script must not be empty");
    }
    if let Some(teardown) = &artifact.teardown
        && teardown.trim().is_empty()
    {
        bail!("{context}: artifact.teardown must not be empty");
    }
    let runtime = match artifact.runtime.unwrap_or(ArtifactRuntimeToml::Native) {
        ArtifactRuntimeToml::Native => ArtifactRuntime::Native,
        ArtifactRuntimeToml::Bun => {
            // A bun artifact is run via `bun <script>`, so the script (and any
            // teardown) must be a bun source file.
            for name in
                std::iter::once(artifact.script.as_str()).chain(artifact.teardown.as_deref())
            {
                if !has_bun_extension(name) {
                    bail!(
                        "{context}: artifact runtime = \"bun\" requires a .ts/.js/.mts/.mjs script, got '{name}'"
                    );
                }
            }
            ArtifactRuntime::Bun
        }
    };
    Ok(Some(AppArtifact {
        script: artifact.script,
        teardown: artifact.teardown,
        runtime,
    }))
}

fn resolve_generator(
    generator: Option<GeneratorToml>,
    context: &str,
) -> Result<Option<AppGenerator>> {
    let Some(generator) = generator else {
        return Ok(None);
    };
    let script = normalize_relative(&generator.script)
        .with_context(|| format!("{context}: invalid generator.script"))?;
    let runtime = match generator.runtime.unwrap_or(ArtifactRuntimeToml::Native) {
        ArtifactRuntimeToml::Native => ArtifactRuntime::Native,
        ArtifactRuntimeToml::Bun => {
            if !has_bun_extension(&generator.script) {
                bail!(
                    "{context}: generator runtime = \"bun\" requires a .ts/.js/.mts/.mjs script, got '{}'",
                    generator.script
                );
            }
            ArtifactRuntime::Bun
        }
    };
    let env = crate::env::parse_env_specs(&generator.env)
        .with_context(|| format!("{context}: invalid generator.env"))?;
    crate::env::validate_env_key(&generator.when_env)
        .with_context(|| format!("{context}: invalid generator.when_env"))?;
    if !env.iter().any(|spec| spec.source == generator.when_env) {
        bail!(
            "{context}: generator.when_env '{}' must be declared in generator.env",
            generator.when_env
        );
    }
    Ok(Some(AppGenerator {
        script,
        runtime,
        env,
        when_env: generator.when_env,
        auto: generator.auto,
    }))
}

fn has_bun_extension(name: &str) -> bool {
    matches!(
        Path::new(name).extension().and_then(|e| e.to_str()),
        Some("ts" | "js" | "mts" | "mjs")
    )
}

fn default_list_mode(has_explicit_files: bool) -> AppListMode {
    if has_explicit_files {
        AppListMode::Files
    } else {
        AppListMode::Category
    }
}

pub fn load_embedded_categories(filter: Option<&str>) -> Result<Vec<AppCategory>> {
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

pub async fn load_installed_categories(
    config: &Config,
    filter: Option<&str>,
) -> Result<Vec<AppCategory>> {
    let app_root = config.presets_dir().join("app");
    let mut category_names: BTreeSet<String> = collect_fs_category_names(&app_root, filter)
        .await?
        .into_iter()
        .collect();
    if let Some(overlay) = config.active_presets_overlay_dir() {
        category_names.extend(collect_fs_category_names(&overlay.join("app"), filter).await?);
    }
    if let Some(filter) = filter
        && category_names.is_empty()
    {
        bail!("app preset category not found: {filter}");
    }
    let mut categories = Vec::new();

    for name in category_names {
        if let Some(category) = load_installed_category(config, &name).await? {
            categories.push(category);
        }
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
) -> Result<Vec<AppCategory>> {
    if config.is_external_presets {
        load_installed_categories(config, filter).await
    } else {
        load_embedded_categories(filter)
    }
}

fn load_embedded_category(name: &str) -> Result<Option<AppCategory>> {
    let metadata_path = format!("app/{name}/shine.toml");
    if let Some(bytes) = presets::read_asset_bytes(&metadata_path) {
        let parsed = parse_category_toml(name, &bytes)?;
        let has_explicit_files = parsed.files.is_some();
        let post_upgrade = resolve_hooks(parsed.post_upgrade, "post_upgrade", &metadata_path)?;
        let post_install = resolve_hooks(parsed.post_install, "post_install", &metadata_path)?;
        let artifact = resolve_artifact(parsed.artifact, &metadata_path)?;
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
                        let generator = resolve_generator(file.generator.clone(), &context)?;
                        Ok(AppFile {
                            source_rel,
                            target_rel,
                            description: file.description,
                            display_name: file.display_name,
                            legacy_dest_annotation: None,
                            transforms,
                            install_strategy,
                            requires_admin: file.requires_admin,
                            restart_hint: file.restart_hint,
                            generator,
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
                    requires_admin: false,
                    restart_hint: None,
                    generator: None,
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
            post_upgrade,
            post_install,
            uses_metadata: true,
            has_explicit_files,
            artifact,
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
                    requires_admin: false,
                    restart_hint: None,
                    generator: None,
                }
            })
            .collect(),
        list_mode: AppListMode::Category,
        post_upgrade: Vec::new(),
        post_install: Vec::new(),
        uses_metadata: false,
        has_explicit_files: false,
        artifact: None,
    }))
}

async fn load_installed_category(config: &Config, name: &str) -> Result<Option<AppCategory>> {
    let category_rel = Path::new("app").join(name);
    let metadata_path = config.preset_path(category_rel.join("shine.toml"));

    if metadata_path.exists() {
        let bytes = fs::read(&metadata_path)
            .await
            .with_context(|| format!("reading metadata: {}", metadata_path.display()))?;
        let parsed = parse_category_toml(name, &bytes)?;
        let has_explicit_files = parsed.files.is_some();
        let post_upgrade = resolve_hooks(
            parsed.post_upgrade,
            "post_upgrade",
            &metadata_path.display().to_string(),
        )?;
        let post_install = resolve_hooks(
            parsed.post_install,
            "post_install",
            &metadata_path.display().to_string(),
        )?;
        let artifact = resolve_artifact(parsed.artifact, &metadata_path.display().to_string())?;
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
                        let generator = resolve_generator(file.generator.clone(), &context)?;
                        Ok(AppFile {
                            source_rel,
                            target_rel,
                            description: file.description,
                            display_name: file.display_name,
                            legacy_dest_annotation: None,
                            transforms,
                            install_strategy,
                            requires_admin: file.requires_admin,
                            restart_hint: file.restart_hint,
                            generator,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?
            }
            None => collect_merged_fs_files(config, &category_rel)
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
                    requires_admin: false,
                    restart_hint: None,
                    generator: None,
                })
                .collect(),
        };
        if files.is_empty() {
            return Ok(None);
        }

        for file in &files {
            let source_path = config.preset_path(category_rel.join(&file.source_rel));
            if !source_path.exists() {
                bail!(
                    "app/{name}/shine.toml references missing file: {}",
                    file.source_rel.display()
                );
            }
            if let Some(generator) = &file.generator {
                let script_path = config.preset_path(category_rel.join(&generator.script));
                if !script_path.exists() {
                    bail!(
                        "app/{name}/shine.toml references missing generator script: {}",
                        generator.script.display()
                    );
                }
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
            post_upgrade,
            post_install,
            uses_metadata: true,
            has_explicit_files,
            artifact,
        }));
    }

    let mut files = Vec::new();
    for rel in collect_merged_fs_files(config, &category_rel).await? {
        let source_path = config.preset_path(category_rel.join(&rel));
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
            requires_admin: false,
            restart_hint: None,
            generator: None,
        });
    }

    Ok(Some(AppCategory {
        name: name.to_string(),
        description: None,
        destination_root: None,
        files,
        list_mode: AppListMode::Category,
        post_upgrade: Vec::new(),
        post_install: Vec::new(),
        uses_metadata: false,
        has_explicit_files: false,
        artifact: None,
    }))
}

async fn collect_merged_fs_files(config: &Config, category_rel: &Path) -> Result<Vec<PathBuf>> {
    crate::preset_meta::merge_fs_tree(config, category_rel, "preset category", |rel| {
        if rel == Path::new("shine.toml") {
            return Ok(None);
        }
        Ok(Some(normalize_relative(&rel.to_string_lossy())?))
    })
    .await
}

fn collect_embedded_category_names(filter: Option<&str>) -> Vec<String> {
    crate::preset_meta::collect_embedded_category_names("app", filter)
}

async fn collect_fs_category_names(app_root: &Path, filter: Option<&str>) -> Result<Vec<String>> {
    crate::preset_meta::collect_fs_category_names(app_root, filter, "app presets dir").await
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
            resolve_generator(file.generator.clone(), &context)?;
        }
    }
    resolve_hooks(
        parsed.post_upgrade.clone(),
        "post_upgrade",
        &format!("app/{name}/shine.toml"),
    )?;
    resolve_hooks(
        parsed.post_install.clone(),
        "post_install",
        &format!("app/{name}/shine.toml"),
    )?;
    resolve_artifact(parsed.artifact.clone(), &format!("app/{name}/shine.toml"))?;
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
    crate::preset_meta::platform_matches(
        file.platforms.as_deref(),
        current,
        &format!("app/{category}/shine.toml"),
    )
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
    // Only the first comment line is the one-line summary. A collected data file
    // (e.g. an overlay's merge.yaml) can carry a long multi-paragraph `#` header;
    // joining the whole block used to leak it as the listed category description
    // when the base ships no shine.toml (see docs/kb/lessons.md 2026-07-17).
    // parse_script_description keeps blank comment lines as empty strings, so the
    // first non-empty entry is the summary line.
    presets::parse_script_description(content)
        .into_iter()
        .find(|line| !line.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn write_test_category(root: &Path, name: &str) {
        let category = root.join("app").join(name);
        fs::create_dir_all(&category).await.unwrap();
        fs::write(category.join("shine.toml"), "dest = \"~/.config/test\"\n")
            .await
            .unwrap();
        fs::write(category.join("config.toml"), "test = true\n")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn filtered_category_may_exist_in_only_one_merged_presets_root() {
        let dir = std::env::temp_dir().join(format!(
            "shine-app-metadata-merged-filter-{}",
            uuid::Uuid::new_v4()
        ));
        let overlay = dir.join("overlay");
        let mut config = Config::new_for_test(&dir);
        config.presets_overlay_dir_override = Some(overlay.clone());

        write_test_category(config.presets_dir(), "base-only").await;
        write_test_category(&overlay, "overlay-only").await;

        let base = load_installed_categories(&config, Some("base-only"))
            .await
            .unwrap();
        let overlaid = load_installed_categories(&config, Some("overlay-only"))
            .await
            .unwrap();

        assert_eq!(base.len(), 1);
        assert_eq!(base[0].name, "base-only");
        assert_eq!(overlaid.len(), 1);
        assert_eq!(overlaid[0].name, "overlay-only");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn embedded_vim_uses_metadata() {
        let categories = load_embedded_categories(Some("vim")).unwrap();
        let vim = categories.iter().find(|c| c.name == "vim").unwrap();
        assert!(vim.uses_metadata);
        assert_eq!(vim.destination_root.as_deref(), Some("~/.vim"));
        assert!(!vim.files.is_empty());
    }

    #[test]
    fn embedded_surge_installs_local_profile_resources() {
        let categories = load_embedded_categories(Some("surge")).unwrap();
        let surge = categories.iter().find(|c| c.name == "surge").unwrap();
        assert!(surge.uses_metadata);
        assert_eq!(
            surge.destination_root.as_deref(),
            Some("~/Library/Application Support/Surge/Profiles")
        );
        let files: Vec<_> = surge
            .files
            .iter()
            .map(|file| {
                (
                    file.source_rel.display().to_string(),
                    file.target_rel.display().to_string(),
                )
            })
            .collect();
        assert_eq!(
            files,
            vec![
                (
                    "local-proxies.conf".to_string(),
                    "local-proxies.conf".to_string()
                ),
                (
                    "local-rules.conf".to_string(),
                    "local-rules.conf".to_string()
                ),
                ("rules/lan.list".to_string(), "rules/lan.list".to_string()),
                (
                    "rules/lan-socks.list".to_string(),
                    "rules/lan-socks.list".to_string()
                ),
                (
                    "rules/other-direct.list".to_string(),
                    "rules/other-direct.list".to_string()
                ),
                (
                    "local-proxy-groups.conf".to_string(),
                    "local-proxy-groups.conf".to_string()
                ),
                (
                    "subscription-proxies.conf".to_string(),
                    "subscription-proxies.conf".to_string()
                ),
            ]
        );
        let subscription = surge
            .files
            .iter()
            .find(|file| file.source_rel == Path::new("subscription-proxies.conf"))
            .unwrap();
        assert_eq!(
            subscription.generator,
            Some(AppGenerator {
                script: PathBuf::from("generate-subscription.ts"),
                runtime: ArtifactRuntime::Bun,
                env: vec![EnvVarSpec {
                    source: "SURGE_SUBSCRIPTION_URL".to_string(),
                    target: "SURGE_SUBSCRIPTION_URL".to_string(),
                }],
                when_env: "SURGE_SUBSCRIPTION_URL".to_string(),
                auto: false,
            })
        );
        assert_eq!(
            surge.post_upgrade,
            vec![AppHook {
                command: "/Applications/Surge.app/Contents/Applications/surge-cli".to_string(),
                args: vec!["reload".to_string()],
                show_output: false,
            }]
        );
        assert_eq!(
            surge.artifact,
            Some(AppArtifact {
                script: "build.ts".to_string(),
                teardown: Some("unbuild.ts".to_string()),
                runtime: ArtifactRuntime::Bun,
            })
        );
    }

    #[test]
    fn post_upgrade_hook_parses_command_and_args() {
        let parsed = parse_category_toml(
            "sample",
            br#"
dest = "~/.config/sample"
post_upgrade = { command = "/bin/echo", args = ["updated"] }

[[files]]
source = "config.toml"
"#,
        )
        .unwrap();
        let hooks = resolve_hooks(parsed.post_upgrade, "post_upgrade", "sample").unwrap();
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].command, "/bin/echo");
        assert_eq!(hooks[0].args, vec!["updated"]);
        assert!(
            !hooks[0].show_output,
            "show_output must default to false when omitted"
        );
    }

    #[test]
    fn post_upgrade_hook_parses_show_output_flag() {
        let parsed = parse_category_toml(
            "sample",
            br#"
dest = "~/.config/sample"
post_upgrade = { command = "/bin/echo", args = ["updated"], show_output = true }

[[files]]
source = "config.toml"
"#,
        )
        .unwrap();
        let hooks = resolve_hooks(parsed.post_upgrade, "post_upgrade", "sample").unwrap();
        assert_eq!(hooks.len(), 1);
        assert!(hooks[0].show_output);
    }

    #[test]
    fn post_upgrade_hook_parses_multiple_commands() {
        let parsed = parse_category_toml(
            "sample",
            br#"
dest = "~/.config/sample"
post_upgrade = [
  { command = "/bin/echo", args = ["updated"] },
  { command = "/bin/echo", args = ["reloaded"] },
]

[[files]]
source = "config.toml"
"#,
        )
        .unwrap();
        let hooks = resolve_hooks(parsed.post_upgrade, "post_upgrade", "sample").unwrap();
        assert_eq!(hooks.len(), 2);
        assert_eq!(hooks[0].args, vec!["updated"]);
        assert_eq!(hooks[1].args, vec!["reloaded"]);
    }

    #[test]
    fn artifact_script_parses() {
        let parsed = parse_category_toml(
            "sample",
            br#"
dest = "~/.config/sample"

[artifact]
script = "build.sh"

[[files]]
source = "config.toml"
"#,
        )
        .unwrap();
        let artifact = resolve_artifact(parsed.artifact, "sample").unwrap();
        assert_eq!(
            artifact,
            Some(AppArtifact {
                script: "build.sh".to_string(),
                teardown: None,
                runtime: ArtifactRuntime::Native,
            })
        );
    }

    #[test]
    fn artifact_teardown_parses() {
        let parsed = parse_category_toml(
            "sample",
            br#"
dest = "~/.config/sample"

[artifact]
script = "build.sh"
teardown = "unbuild.sh"

[[files]]
source = "config.toml"
"#,
        )
        .unwrap();
        let artifact = resolve_artifact(parsed.artifact, "sample").unwrap();
        assert_eq!(
            artifact,
            Some(AppArtifact {
                script: "build.sh".to_string(),
                teardown: Some("unbuild.sh".to_string()),
                runtime: ArtifactRuntime::Native,
            })
        );
    }

    #[test]
    fn artifact_empty_teardown_is_rejected() {
        let parsed = parse_category_toml(
            "sample",
            br#"
dest = "~/.config/sample"

[artifact]
script = "build.sh"
teardown = "  "

[[files]]
source = "config.toml"
"#,
        );
        let err = parsed.unwrap_err();
        assert!(
            err.to_string()
                .contains("artifact.teardown must not be empty")
        );
    }

    #[test]
    fn post_install_hook_parses_single_and_array() {
        let single = parse_category_toml(
            "sample",
            br#"
dest = "~/.config/sample"
post_install = { command = "/bin/echo", args = ["installed"] }

[[files]]
source = "config.toml"
"#,
        )
        .unwrap();
        let hooks = resolve_hooks(single.post_install, "post_install", "sample").unwrap();
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].command, "/bin/echo");
        assert_eq!(hooks[0].args, vec!["installed"]);

        let multiple = parse_category_toml(
            "sample",
            br#"
dest = "~/.config/sample"
post_install = [
  { command = "/bin/echo", args = ["a"] },
  { command = "/bin/echo", args = ["b"] },
]

[[files]]
source = "config.toml"
"#,
        )
        .unwrap();
        let hooks = resolve_hooks(multiple.post_install, "post_install", "sample").unwrap();
        assert_eq!(hooks.len(), 2);
    }

    #[test]
    fn post_install_empty_command_is_rejected() {
        let err = parse_category_toml(
            "sample",
            br#"
dest = "~/.config/sample"
post_install = { command = "  " }

[[files]]
source = "config.toml"
"#,
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("post_install.command must not be empty")
        );
    }

    #[test]
    fn artifact_section_absent_is_none() {
        let parsed = parse_category_toml(
            "sample",
            br#"
dest = "~/.config/sample"

[[files]]
source = "config.toml"
"#,
        )
        .unwrap();
        assert!(
            resolve_artifact(parsed.artifact, "sample")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn artifact_empty_script_is_rejected() {
        let parsed = parse_category_toml(
            "sample",
            br#"
dest = "~/.config/sample"

[artifact]
script = ""

[[files]]
source = "config.toml"
"#,
        );
        let err = parsed.unwrap_err();
        assert!(err.to_string().contains("artifact.script"));
    }

    #[test]
    fn artifact_runtime_defaults_native_and_bun_requires_bun_extension() {
        let parse = |body: &str| -> CategoryToml {
            toml::from_str(&format!(
                "description = \"S\"\ndest = \"~/x\"\n\n{body}\n\n[[files]]\nsource = \"c\"\n"
            ))
            .unwrap()
        };

        // Default (no runtime) is Native.
        let native = resolve_artifact(parse("[artifact]\nscript = \"build.sh\"").artifact, "s")
            .unwrap()
            .unwrap();
        assert_eq!(native.runtime, ArtifactRuntime::Native);

        // runtime = "bun" with a .ts script parses to Bun.
        let bun = resolve_artifact(
            parse(
                "[artifact]\nscript = \"build.ts\"\nteardown = \"unbuild.ts\"\nruntime = \"bun\"",
            )
            .artifact,
            "s",
        )
        .unwrap()
        .unwrap();
        assert_eq!(bun.runtime, ArtifactRuntime::Bun);

        // runtime = "bun" with a non-bun script (or teardown) is rejected.
        assert!(
            resolve_artifact(
                parse("[artifact]\nscript = \"build.sh\"\nruntime = \"bun\"").artifact,
                "s"
            )
            .is_err()
        );
        assert!(
            resolve_artifact(
                parse(
                    "[artifact]\nscript = \"build.ts\"\nteardown = \"unbuild.sh\"\nruntime = \"bun\""
                )
                .artifact,
                "s"
            )
            .is_err()
        );
    }

    #[test]
    fn embedded_surge_declares_artifact_script() {
        let categories = load_embedded_categories(Some("surge")).unwrap();
        let surge = categories.iter().find(|c| c.name == "surge").unwrap();
        assert_eq!(
            surge.artifact,
            Some(AppArtifact {
                script: "build.ts".to_string(),
                teardown: Some("unbuild.ts".to_string()),
                runtime: ArtifactRuntime::Bun,
            })
        );
    }

    #[test]
    fn embedded_clash_verge_installs_plain_merge_profile() {
        let categories = load_embedded_categories(Some("clash-verge")).unwrap();
        let clash = categories.iter().find(|c| c.name == "clash-verge").unwrap();
        assert!(clash.uses_metadata);
        assert_eq!(
            clash.destination_root.as_deref(),
            Some("~/.shine/clash-verge")
        );

        assert_eq!(clash.files.len(), 1);
        let file = &clash.files[0];
        assert_eq!(file.source_rel, std::path::Path::new("merge.yaml"));
        assert_eq!(file.target_rel, std::path::Path::new("merge.yaml"));
        // No templating: merge.yaml is installed verbatim (plain Copy) so the file
        // stays valid YAML. Real values are hardcoded in the overlay copy.
        assert!(file.transforms.is_empty());
        assert_eq!(file.install_strategy, AppInstallStrategy::Copy);

        let merge = include_str!("../../../presets/app/clash-verge/merge.yaml");
        assert!(merge.contains("# proxies:"));
        assert!(merge.contains("# proxy-groups:"));
        assert!(merge.contains("# prepend-rules:"));
        assert!(merge.contains("type: file, behavior: classical, format: text"));
        assert!(merge.contains("http://127.0.0.1:8080/rules/lan.list"));
        assert!(merge.contains("https://rules.example.com/surge/lan.list"));

        // post_install/post_upgrade re-invoke `shine app artifact apply clash-verge` so the
        // artifact writes the bound CVR subscription Extend Config after an
        // install/upgrade that changes merge.yaml, then refreshes once applied.
        let build_hook = vec![AppHook {
            command: "shine".to_string(),
            args: vec![
                "app".to_string(),
                "artifact".to_string(),
                "apply".to_string(),
                "clash-verge".to_string(),
            ],
            show_output: true,
        }];
        assert_eq!(clash.post_install, build_hook);
        assert_eq!(clash.post_upgrade, build_hook);
        assert_eq!(
            clash.artifact,
            Some(AppArtifact {
                script: "build.ts".to_string(),
                teardown: Some("unbuild.ts".to_string()),
                runtime: ArtifactRuntime::Bun,
            })
        );
    }

    #[test]
    fn legacy_description_is_first_comment_line_only() {
        // A long multi-paragraph header (like an overlay's merge.yaml) must not
        // leak as the listed description — only the first comment line is used.
        let yaml = b"# Clash Verge Rev merge profile. Summary line.\n#\n# A second paragraph\n# that keeps going and going.\nproxies:\n  - name: X\n";
        assert_eq!(
            parse_legacy_description(yaml).as_deref(),
            Some("Clash Verge Rev merge profile. Summary line.")
        );
        // The `# shine-dest:` annotation is skipped; a single-line summary is unchanged.
        let gitconfig = b"# shine-dest: ~/.gitconfig\n# Personal git configuration.\n\n[pull]\n";
        assert_eq!(
            parse_legacy_description(gitconfig).as_deref(),
            Some("Personal git configuration.")
        );
        // No comment header -> no description.
        assert_eq!(parse_legacy_description(b"proxies: []\n"), None);
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
        assert!(file.requires_admin);
        assert!(
            file.restart_hint
                .as_deref()
                .is_some_and(|hint| hint.contains("Restart Docker Engine"))
        );
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
            assert_eq!(ghostty.files.len(), 6);

            let shine_light = ghostty
                .files
                .iter()
                .find(|f| f.source_rel == std::path::Path::new("themes/Shine Light"))
                .unwrap();
            assert_eq!(
                shine_light.target_rel,
                std::path::Path::new("themes/Shine Light")
            );
            assert_eq!(shine_light.transforms, vec!["template"]);

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
                    requires_admin: false,
                    restart_hint: None,
                    generator: None,
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
            requires_admin: false,
            restart_hint: None,
            generator: None,
        };
        assert!(resolve_transforms(&file, "test").is_err());
    }

    #[test]
    fn generator_metadata_parses_and_validates_condition_env() {
        let parsed = parse_category_toml(
            "sample",
            br#"
dest = "/tmp"
[[files]]
source = "fallback.conf"
generator = { script = "generate.ts", runtime = "bun", env = ["SOURCE_URL"], when_env = "SOURCE_URL" }
"#,
        )
        .unwrap();
        let generator = resolve_generator(
            parsed.files.unwrap().remove(0).generator,
            "app/sample/shine.toml",
        )
        .unwrap()
        .unwrap();
        assert_eq!(generator.script, Path::new("generate.ts"));
        assert_eq!(generator.runtime, ArtifactRuntime::Bun);
        assert_eq!(generator.when_env, "SOURCE_URL");
        assert!(generator.auto);
    }

    #[test]
    fn generator_auto_can_be_disabled() {
        let parsed = parse_category_toml(
            "sample",
            br#"
description = "sample"
dest = "~/.config/sample"

[[files]]
source = "fallback.txt"
generator = { script = "generate.ts", runtime = "bun", env = ["SOURCE_URL"], when_env = "SOURCE_URL", auto = false }
"#,
        )
        .unwrap();
        let generator = resolve_generator(
            parsed.files.unwrap().remove(0).generator,
            "app/sample/shine.toml",
        )
        .unwrap()
        .unwrap();
        assert!(!generator.auto);
    }

    #[test]
    fn generator_condition_must_be_in_declared_env() {
        let error = parse_category_toml(
            "sample",
            br#"
dest = "/tmp"
[[files]]
source = "fallback.conf"
generator = { script = "generate.ts", runtime = "bun", env = ["OTHER_URL"], when_env = "SOURCE_URL" }
"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("must be declared"));
    }
}
