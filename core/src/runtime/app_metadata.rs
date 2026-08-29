use super::{
    AppArtifact, AppCategory, AppDestinationRoot, AppFile, AppGenerator, AppHook, AppListMode,
    ArtifactRuntime, CoreRuntime, RuntimePlatform,
};
use crate::install::AppInstallStrategy;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

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
    teardown: Option<String>,
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
    Rooted(RootedDestToml),
    Platforms(PlatformDestToml),
}

#[derive(Debug, Deserialize)]
struct RootedDestToml {
    base: DestBaseToml,
    path: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum DestBaseToml {
    DataDir,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlatformDestToml {
    macos: Option<String>,
    linux: Option<String>,
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

#[derive(Debug, Deserialize)]
struct FileToml {
    source: String,
    target: Option<String>,
    dest: Option<DestToml>,
    description: Option<String>,
    display_name: Option<String>,
    platforms: Option<Vec<String>>,
    transform: Option<String>,
    transforms: Option<Vec<String>>,
    install_mode: Option<InstallModeToml>,
    managed_keys: Option<Vec<String>>,
    #[serde(default)]
    requires_admin: bool,
    restart_hint: Option<String>,
    generator: Option<GeneratorToml>,
}

#[derive(Debug, Clone, Deserialize)]
struct GeneratorToml {
    script: String,
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

impl<H> CoreRuntime<H> {
    /// Parse App metadata and legacy categories exclusively from the immutable
    /// effective preset snapshot. Overlay/source choice is therefore fixed for
    /// the whole command.
    pub fn app_categories(&self, filter: Option<&str>) -> Result<Vec<AppCategory>> {
        let names = category_names(self.presets().files().keys(), "app", filter);
        if filter.is_some() && names.is_empty() && self.context().is_external_presets {
            bail!(
                "app preset category not found: {}",
                filter.unwrap_or_default()
            );
        }
        names
            .into_iter()
            .filter_map(|name| self.parse_app_category(&name).transpose())
            .collect()
    }

    fn parse_app_category(&self, name: &str) -> Result<Option<AppCategory>> {
        let prefix = format!("app/{name}/");
        let metadata_path = format!("{prefix}shine.toml");
        let Some(metadata) = self.presets().get(&metadata_path) else {
            let files = collect_category_files(self.presets().files().keys(), &prefix)
                .into_iter()
                .map(|source_rel| {
                    let bytes = self
                        .presets()
                        .get(&format!("{prefix}{}", logical(&source_rel)))
                        .unwrap_or_default();
                    AppFile {
                        target_rel: source_rel.clone(),
                        source_rel,
                        destination_root: None,
                        description: legacy_description(bytes),
                        display_name: None,
                        legacy_dest_annotation: dest_annotation(bytes),
                        transforms: Vec::new(),
                        install_strategy: AppInstallStrategy::Copy,
                        requires_admin: false,
                        restart_hint: None,
                        generator: None,
                    }
                })
                .collect::<Vec<_>>();
            return Ok((!files.is_empty()).then(|| AppCategory {
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
            }));
        };

        let parsed: CategoryToml = toml::from_slice(metadata)
            .with_context(|| format!("failed to parse app/{name}/shine.toml"))?;
        let Some(destination_root) = parsed.dest.select(name, self.context().platform)? else {
            return Ok(None);
        };
        let explicit = parsed.files.is_some();
        let files = if let Some(files) = parsed.files {
            let mut resolved = Vec::new();
            for file in files {
                if !platform_matches(
                    file.platforms.as_deref(),
                    self.context().platform,
                    &metadata_path,
                )? {
                    continue;
                }
                let destination = file
                    .dest
                    .as_ref()
                    .map(|dest| dest.select_file(name, self.context().platform))
                    .transpose()?
                    .flatten();
                if file.dest.is_some() && destination.is_none() {
                    continue;
                }
                let source_rel = normalize_relative(&file.source)
                    .with_context(|| format!("invalid source for {metadata_path}"))?;
                let target_rel = normalize_relative(file.target.as_deref().unwrap_or(&file.source))
                    .with_context(|| format!("invalid target for {metadata_path}"))?;
                let source_logical = format!("{prefix}{}", logical(&source_rel));
                if self.presets().get(&source_logical).is_none() {
                    bail!(
                        "app/{name}/shine.toml references missing file: {}",
                        source_rel.display()
                    );
                }
                let transforms = transforms(&file, &metadata_path)?;
                let install_strategy = install_strategy(&file, &metadata_path)?;
                let generator = generator(file.generator, &metadata_path)?;
                if let Some(generator) = &generator {
                    let generator_path = format!("{prefix}{}", logical(&generator.script));
                    if self.presets().get(&generator_path).is_none() {
                        bail!(
                            "app/{name}/shine.toml references missing generator script: {}",
                            generator.script.display()
                        );
                    }
                }
                resolved.push(AppFile {
                    source_rel,
                    target_rel,
                    destination_root: destination,
                    description: file.description,
                    display_name: file.display_name,
                    legacy_dest_annotation: None,
                    transforms,
                    install_strategy,
                    requires_admin: file.requires_admin,
                    restart_hint: file.restart_hint,
                    generator,
                });
            }
            resolved
        } else {
            collect_category_files(self.presets().files().keys(), &prefix)
                .into_iter()
                .map(|source_rel| AppFile {
                    target_rel: source_rel.clone(),
                    source_rel,
                    destination_root: None,
                    description: None,
                    display_name: None,
                    legacy_dest_annotation: None,
                    transforms: Vec::new(),
                    install_strategy: AppInstallStrategy::Copy,
                    requires_admin: false,
                    restart_hint: None,
                    generator: None,
                })
                .collect()
        };
        if files.is_empty() {
            return Ok(None);
        }
        Ok(Some(AppCategory {
            name: name.to_string(),
            description: parsed.description,
            destination_root: Some(destination_root),
            files,
            list_mode: parsed.list_mode.map_or_else(
                || {
                    if explicit {
                        AppListMode::Files
                    } else {
                        AppListMode::Category
                    }
                },
                |mode| match mode {
                    ListModeToml::Category => AppListMode::Category,
                    ListModeToml::Files => AppListMode::Files,
                },
            ),
            post_upgrade: hooks(parsed.post_upgrade, "post_upgrade", &metadata_path)?,
            post_install: hooks(parsed.post_install, "post_install", &metadata_path)?,
            uses_metadata: true,
            has_explicit_files: explicit,
            artifact: artifact(parsed.artifact, &metadata_path)?,
        }))
    }

    pub fn app_source_bytes(&self, category: &str, file: &AppFile) -> Result<&[u8]> {
        let path = format!("app/{category}/{}", logical(&file.source_rel));
        self.presets()
            .get(&path)
            .with_context(|| format!("missing preset file {path}"))
    }

    pub fn app_destination(&self, category: &AppCategory, file: &AppFile) -> Result<PathBuf> {
        let root = file.destination_root.as_ref().map_or_else(
            || {
                category
                    .destination_root
                    .as_ref()
                    .map(|path| AppDestinationRoot::Path(path.clone()))
            },
            |root| Some(root.clone()),
        );
        let base = match root {
            Some(AppDestinationRoot::DataDir(relative)) => self.context().data_dir.join(relative),
            Some(AppDestinationRoot::Path(raw)) => expand_path(&raw, self.context())?,
            None => {
                if let Some(annotation) = &file.legacy_dest_annotation {
                    return expand_path(annotation, self.context());
                }
                self.context().app_default_dest_root.join(&category.name)
            }
        };
        let destination = base.join(&file.target_rel);
        if destination
            .components()
            .any(|component| component == Component::ParentDir)
        {
            bail!(
                "destination path must not contain '..': {}",
                destination.display()
            );
        }
        Ok(destination)
    }
}

fn category_names<'a>(
    paths: impl Iterator<Item = &'a String>,
    kind: &str,
    filter: Option<&str>,
) -> Vec<String> {
    let prefix = format!("{kind}/");
    paths
        .filter_map(|path| path.strip_prefix(&prefix))
        .filter_map(|rest| rest.split_once('/').map(|(category, _)| category))
        .filter(|category| filter.is_none_or(|filter| filter == *category))
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn collect_category_files<'a>(
    paths: impl Iterator<Item = &'a String>,
    prefix: &str,
) -> Vec<PathBuf> {
    paths
        .filter_map(|path| path.strip_prefix(prefix))
        .filter(|relative| !relative.is_empty() && *relative != "shine.toml")
        .map(PathBuf::from)
        .collect()
}

fn logical(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn normalize_relative(value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    if path.as_os_str().is_empty() {
        bail!("path must not be empty");
    }
    if path.is_absolute() {
        bail!("path must be relative");
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("path must not contain '..'");
    }
    Ok(path.to_path_buf())
}

fn transforms(file: &FileToml, context: &str) -> Result<Vec<String>> {
    let values = match (&file.transform, &file.transforms) {
        (Some(_), Some(_)) => bail!("{context}: use 'transform' or 'transforms', not both"),
        (Some(value), None) => vec![value.clone()],
        (None, Some(values)) => values.clone(),
        (None, None) => Vec::new(),
    };
    crate::install::transforms::validate(&values)
        .with_context(|| format!("{context}: invalid transform"))?;
    Ok(values)
}

fn install_strategy(file: &FileToml, context: &str) -> Result<AppInstallStrategy> {
    match file.install_mode.unwrap_or(InstallModeToml::Copy) {
        InstallModeToml::Copy => {
            if file.managed_keys.is_some() {
                bail!("{context}: 'managed_keys' requires install_mode = \"json-merge\"");
            }
            Ok(AppInstallStrategy::Copy)
        }
        InstallModeToml::JsonMerge => {
            let keys = file
                .managed_keys
                .clone()
                .context("json-merge requires 'managed_keys'")?;
            if keys.is_empty()
                || keys
                    .iter()
                    .any(|key| key.trim().is_empty() || key.contains('.'))
            {
                bail!("{context}: managed_keys must contain non-empty top-level JSON keys");
            }
            Ok(AppInstallStrategy::JsonMerge { managed_keys: keys })
        }
    }
}

fn hooks(value: Option<HookSpecToml>, field: &str, context: &str) -> Result<Vec<AppHook>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let hooks = match value {
        HookSpecToml::Single(hook) => vec![hook],
        HookSpecToml::Multiple(hooks) => hooks,
    };
    if hooks.is_empty() {
        bail!("{context}: {field} must not be empty");
    }
    hooks
        .into_iter()
        .map(|hook| {
            if hook.command.trim().is_empty() {
                bail!("{context}: {field}.command must not be empty");
            }
            Ok(AppHook {
                command: hook.command,
                args: hook.args,
                show_output: hook.show_output,
            })
        })
        .collect()
}

fn artifact(value: Option<ArtifactToml>, context: &str) -> Result<Option<AppArtifact>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.script.trim().is_empty()
        || value
            .teardown
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
    {
        bail!("{context}: artifact scripts must not be empty");
    }
    let runtime = artifact_runtime(value.runtime, &value.script, context)?;
    if runtime == ArtifactRuntime::Bun
        && let Some(teardown) = &value.teardown
    {
        require_bun_extension(teardown, context)?;
    }
    Ok(Some(AppArtifact {
        script: value.script,
        teardown: value.teardown,
        runtime,
    }))
}

fn generator(value: Option<GeneratorToml>, context: &str) -> Result<Option<AppGenerator>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let script = normalize_relative(&value.script)
        .with_context(|| format!("{context}: invalid generator.script"))?;
    let runtime = artifact_runtime(value.runtime, &value.script, context)?;
    let env = crate::env::parse_env_specs(&value.env)
        .with_context(|| format!("{context}: invalid generator.env"))?;
    crate::env::validate_env_key(&value.when_env)
        .with_context(|| format!("{context}: invalid generator.when_env"))?;
    if !env.iter().any(|spec| spec.source == value.when_env) {
        bail!(
            "{context}: generator.when_env '{}' must be declared in generator.env",
            value.when_env
        );
    }
    Ok(Some(AppGenerator {
        script,
        runtime,
        env,
        when_env: value.when_env,
        auto: value.auto,
    }))
}

fn artifact_runtime(
    value: Option<ArtifactRuntimeToml>,
    script: &str,
    context: &str,
) -> Result<ArtifactRuntime> {
    match value.unwrap_or(ArtifactRuntimeToml::Native) {
        ArtifactRuntimeToml::Native => Ok(ArtifactRuntime::Native),
        ArtifactRuntimeToml::Bun => {
            require_bun_extension(script, context)?;
            Ok(ArtifactRuntime::Bun)
        }
    }
}

fn require_bun_extension(script: &str, context: &str) -> Result<()> {
    if !matches!(
        Path::new(script)
            .extension()
            .and_then(|value| value.to_str()),
        Some("ts" | "js" | "mts" | "mjs")
    ) {
        bail!("{context}: runtime = \"bun\" requires a .ts/.js/.mts/.mjs script, got '{script}'");
    }
    Ok(())
}

impl DestToml {
    fn select(&self, category: &str, platform: RuntimePlatform) -> Result<Option<String>> {
        match self {
            Self::Single(path) => {
                validate_destination(path, platform, category)?;
                Ok(Some(path.clone()))
            }
            Self::Rooted(_) => bail!(
                "app/{category}/shine.toml rooted destinations are supported only in [[files]]"
            ),
            Self::Platforms(paths) => paths.select(category, platform),
        }
    }

    fn select_file(
        &self,
        category: &str,
        platform: RuntimePlatform,
    ) -> Result<Option<AppDestinationRoot>> {
        match self {
            Self::Single(path) => {
                validate_destination(path, platform, category)?;
                Ok(Some(AppDestinationRoot::Path(path.clone())))
            }
            Self::Rooted(rooted) => Ok(Some(match rooted.base {
                DestBaseToml::DataDir => {
                    AppDestinationRoot::DataDir(normalize_relative(&rooted.path)?)
                }
            })),
            Self::Platforms(paths) => Ok(paths
                .select(category, platform)?
                .map(AppDestinationRoot::Path)),
        }
    }
}

impl PlatformDestToml {
    fn select(&self, category: &str, platform: RuntimePlatform) -> Result<Option<String>> {
        if self.macos.is_none()
            && self.linux.is_none()
            && self.windows.is_none()
            && self.unix.is_none()
        {
            bail!("app/{category}/shine.toml platform destination map must not be empty");
        }
        let selected = match platform {
            RuntimePlatform::Macos => self.macos.clone().or_else(|| self.unix.clone()),
            RuntimePlatform::Linux => self.linux.clone().or_else(|| self.unix.clone()),
            RuntimePlatform::Windows => self.windows.clone(),
        };
        if let Some(path) = &selected {
            validate_destination(path, platform, category)?;
        }
        Ok(selected)
    }
}

fn validate_destination(path: &str, platform: RuntimePlatform, category: &str) -> Result<()> {
    let home = path == "~"
        || path.starts_with("~/")
        || path.starts_with("~\\")
        || path.starts_with("$HOME/");
    let unix = path.starts_with('/');
    let bytes = path.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    let unc = path.starts_with("\\\\") || path.starts_with("//");
    if !(home
        || match platform {
            RuntimePlatform::Windows => drive || unc,
            _ => unix,
        })
    {
        bail!("app/{category}/shine.toml dest must be absolute after expansion");
    }
    if path.split(['/', '\\']).any(|part| part == "..") {
        bail!("app/{category}/shine.toml dest must not contain '..'");
    }
    Ok(())
}

fn platform_matches(
    values: Option<&[String]>,
    platform: RuntimePlatform,
    context: &str,
) -> Result<bool> {
    let Some(values) = values else {
        return Ok(true);
    };
    if values.is_empty() {
        bail!(
            "{context} platforms must not be empty; expected `macos`, `linux`, `windows`, or `unix`"
        );
    }
    let mut matches = false;
    for value in values {
        match value.trim().to_ascii_lowercase().as_str() {
            "macos" => matches |= platform == RuntimePlatform::Macos,
            "linux" => matches |= platform == RuntimePlatform::Linux,
            "windows" => matches |= platform == RuntimePlatform::Windows,
            "unix" => matches |= platform.is_unix(),
            _ => bail!(
                "{context} has unsupported platform `{value}`; expected `macos`, `linux`, `windows`, or `unix`"
            ),
        }
    }
    Ok(matches)
}

fn expand_path(raw: &str, context: &super::RuntimeContext) -> Result<PathBuf> {
    let home = context.home_dir.to_string_lossy();
    let expanded = if raw == "~" || raw == "$HOME" {
        home.to_string()
    } else if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        context.home_dir.join(rest).display().to_string()
    } else if let Some(rest) = raw.strip_prefix("$HOME/") {
        context.home_dir.join(rest).display().to_string()
    } else {
        raw.to_string()
    };
    let path = PathBuf::from(expanded);
    if !path.is_absolute() {
        bail!(
            "destination path must be absolute after expansion, got: {}",
            path.display()
        );
    }
    Ok(path)
}

fn dest_annotation(content: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(content).ok()?;
    let mut lines = text.lines();
    let first = lines.next()?;
    let line = if first.starts_with("#!") {
        lines.next()?
    } else {
        first
    };
    line.trim()
        .strip_prefix("# shine-dest:")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn legacy_description(content: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(content).ok()?;
    text.lines()
        .filter(|line| !line.starts_with("#!"))
        .map(str::trim)
        .find_map(|line| {
            line.strip_prefix("# ")
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{InMemoryHost, PresetSnapshot, PresetSourceKind, RuntimeContext};

    #[test]
    fn parses_app_category_from_snapshot_and_selects_platform() {
        let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "app/demo/shine.toml",
                b"dest = { linux = \"~/.config/demo\" }\n[[files]]\nsource = \"config.toml\"\n"
                    .to_vec(),
            )
            .file("app/demo/config.toml", b"value = true\n".to_vec())
            .build();
        let runtime = CoreRuntime::new(
            InMemoryHost::new(),
            RuntimeContext::isolated(
                PathBuf::from("/home/me"),
                PathBuf::from("/home/me/.shine"),
                PathBuf::from("/presets"),
                PathBuf::from("/bin"),
                RuntimePlatform::Linux,
            ),
            snapshot,
        );
        let categories = runtime.app_categories(Some("demo")).unwrap();
        assert_eq!(categories[0].files.len(), 1);
        assert_eq!(
            runtime
                .app_destination(&categories[0], &categories[0].files[0])
                .unwrap(),
            PathBuf::from("/home/me/.config/demo/config.toml")
        );
    }
}
