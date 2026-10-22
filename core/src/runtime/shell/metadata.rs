//! Snapshot-only Shell metadata parsing, platform admission and compatible native discovery.
pub(super) use super::super::path_text::preset_path as shell_logical_path;
use super::*;
use crate::runtime::{RuntimePlatform, link_stem};

#[derive(Debug, Deserialize)]
pub(super) struct ShellCategoryToml {
    pub(super) description: Option<String>,
    pub(super) permission_defaults: Option<PermissionDeclarationV1>,
    pub(super) files: Option<Vec<ShellFileToml>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ShellFileToml {
    pub(super) source: String,
    pub(super) target: Option<String>,
    pub(super) description: Option<String>,
    pub(super) needs_source: Option<bool>,
    pub(super) platforms: Option<Vec<String>>,
    pub(super) runtime: Option<String>,
    pub(super) transforms: Option<Vec<String>>,
    pub(super) env: Option<Vec<crate::env::EnvDeclaration>>,
    pub(super) permissions: Option<PermissionDeclarationV1>,
}

impl<H> CoreRuntime<H> {
    pub fn shell_categories(&self, filter: Option<&str>) -> Result<Vec<ShellCategory>> {
        let categories = self.shell_categories_or_missing(filter)?;
        if filter.is_some() && categories.is_empty() && self.context().is_external_presets {
            bail!(
                "shell preset category not found: {}",
                filter.unwrap_or_default()
            );
        }
        Ok(categories)
    }

    pub(in crate::runtime) fn shell_categories_or_missing(
        &self,
        filter: Option<&str>,
    ) -> Result<Vec<ShellCategory>> {
        let prefix = "shell/";
        let names = self
            .presets()
            .files()
            .keys()
            .filter_map(|path| path.strip_prefix(prefix))
            .filter_map(|rest| rest.split_once('/').map(|(category, _)| category))
            .filter(|category| filter.is_none_or(|filter| filter == *category))
            .map(str::to_string)
            .collect::<BTreeSet<_>>();
        names
            .into_iter()
            .map(|name| self.parse_shell_category(&name))
            .collect()
    }

    fn parse_shell_category(&self, name: &str) -> Result<ShellCategory> {
        let prefix = format!("shell/{name}/");
        let metadata_path = format!("{prefix}shine.toml");
        let metadata = self.presets().get(&metadata_path);
        let parsed = metadata
            .map(|bytes| {
                toml::from_slice::<ShellCategoryToml>(bytes)
                    .with_context(|| format!("failed to parse {metadata_path}"))
            })
            .transpose()?;
        if let Some(defaults) = parsed
            .as_ref()
            .and_then(|parsed| parsed.permission_defaults.as_ref())
        {
            defaults
                .validate()
                .with_context(|| format!("invalid permission_defaults in {metadata_path}"))?;
        }
        let defaults = parsed
            .as_ref()
            .and_then(|parsed| parsed.permission_defaults.as_ref());
        let mut files =
            if let Some(entries) = parsed.as_ref().and_then(|parsed| parsed.files.as_ref()) {
                self.declared_shell_files(name, &prefix, &metadata_path, entries, defaults)?
            } else {
                self.discovered_shell_files(&prefix, defaults)?
            };
        files.sort_by(|left, right| left.command_name.cmp(&right.command_name));
        let mut commands = BTreeSet::new();
        for file in &files {
            if !commands.insert(file.command_name.clone()) {
                bail!(MetadataDiagnosticError::new(
                    MetadataDiagnosticKind::DuplicateCommand,
                    format!(
                        "shell/{name} declares command `{}` more than once",
                        file.command_name
                    ),
                ));
            }
        }
        Ok(ShellCategory {
            name: name.to_string(),
            description: parsed.and_then(|parsed| parsed.description),
            files,
            uses_metadata: metadata.is_some(),
        })
    }
    fn declared_shell_files(
        &self,
        name: &str,
        prefix: &str,
        metadata_path: &str,
        entries: &[ShellFileToml],
        defaults: Option<&PermissionDeclarationV1>,
    ) -> Result<Vec<ShellFile>> {
        let mut files = Vec::new();
        for entry in entries {
            if !shell_platform_matches(
                entry.platforms.as_deref(),
                self.context().platform,
                metadata_path,
            )? {
                continue;
            }
            let runtime = match entry.runtime.as_deref() {
                None | Some("native") => LinkRuntime::Native,
                Some("bun") => LinkRuntime::Bun,
                Some(other) => bail!("unsupported runtime `{other}` (expected `bun`)"),
            };
            let source_rel = normalize_shell_metadata_source(&entry.source, runtime)
                .with_context(|| format!("invalid source in {metadata_path}"))?;
            if !shell_source_matches(runtime, self.context().shell, &source_rel) {
                continue;
            }
            let command_name = shell_command_name(&source_rel, entry.target.as_deref())?;
            let needs_source = entry.needs_source.unwrap_or(false);
            if runtime == LinkRuntime::Bun && needs_source {
                bail!(
                    "{metadata_path}: `runtime = \"bun\"` cannot be combined with `needs_source = true`"
                );
            }
            let transforms = entry.transforms.clone().unwrap_or_default();
            crate::install::transforms::validate(&transforms)
                .with_context(|| format!("invalid transforms in {metadata_path}"))?;
            let mut permissions = entry.permissions.as_ref().or(defaults).cloned();
            if let Some(permissions) = &permissions {
                permissions
                    .validate()
                    .with_context(|| format!("invalid permissions in {metadata_path}"))?;
            }
            let env = crate::env::resolve_env_declarations(
                entry.env.as_deref().unwrap_or_default(),
                &mut permissions,
            )
            .with_context(|| format!("invalid env in {metadata_path}"))?;
            if runtime != LinkRuntime::Bun && !env.is_empty() {
                bail!("{metadata_path}: `env` is only valid when `runtime = \"bun\"`");
            }
            let logical = format!("{prefix}{}", shell_logical_path(&source_rel));
            let bytes = self.presets().get(&logical).ok_or_else(|| {
                MetadataDiagnosticError::new(
                    MetadataDiagnosticKind::MissingReference,
                    format!(
                        "shell/{name}/shine.toml references missing file: {}",
                        source_rel.display()
                    ),
                )
            })?;
            let description = entry.description.clone().map_or_else(
                || shell_description(bytes, runtime),
                |description| vec![description],
            );
            files.push(ShellFile {
                source_rel,
                command_name,
                description,
                needs_source,
                runtime,
                transforms,
                env,
                permissions,
            });
        }
        Ok(files)
    }

    fn discovered_shell_files(
        &self,
        prefix: &str,
        defaults: Option<&PermissionDeclarationV1>,
    ) -> Result<Vec<ShellFile>> {
        let mut files = Vec::new();
        for path in self.presets().files().keys() {
            let Some(relative) = path.strip_prefix(prefix) else {
                continue;
            };
            if relative == "shine.toml" || !is_native_shell_script(Path::new(relative)) {
                continue;
            }
            let source_rel = normalize_shell_metadata_source(relative, LinkRuntime::Native)?;
            if !shell_source_matches(LinkRuntime::Native, self.context().shell, &source_rel) {
                continue;
            }
            let bytes = self.presets().get(path).unwrap_or_default();
            files.push(ShellFile {
                command_name: shell_command_name(&source_rel, None)?,
                description: shell_description(bytes, LinkRuntime::Native),
                needs_source: false,
                runtime: LinkRuntime::Native,
                transforms: Vec::new(),
                env: Vec::new(),
                permissions: defaults.cloned(),
                source_rel,
            });
        }
        Ok(files)
    }
}

pub(super) fn shell_platform_matches(
    platforms: Option<&[String]>,
    current: RuntimePlatform,
    context: &str,
) -> Result<bool> {
    let Some(platforms) = platforms else {
        return Ok(true);
    };
    if platforms.is_empty() {
        bail!(
            "{context} platforms must not be empty; expected `macos`, `linux`, `windows`, or `unix`"
        );
    }
    let mut matches = false;
    for platform in platforms {
        match platform.trim().to_ascii_lowercase().as_str() {
            "macos" => matches |= current == RuntimePlatform::Macos,
            "linux" => matches |= current == RuntimePlatform::Linux,
            "windows" => matches |= current == RuntimePlatform::Windows,
            "unix" => matches |= current.is_unix(),
            _ => bail!(
                "{context} has unsupported platform `{platform}`; expected `macos`, `linux`, `windows`, or `unix`"
            ),
        }
    }
    Ok(matches)
}

pub(super) fn normalize_shell_metadata_source(
    value: &str,
    runtime: LinkRuntime,
) -> Result<PathBuf> {
    let path = Path::new(value);
    if path.as_os_str().is_empty() || path.is_absolute() {
        bail!("source path must be a non-empty relative path");
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(value) => normalized.push(value),
            std::path::Component::CurDir => {}
            _ => bail!("source path must be relative and must not contain '..'"),
        }
    }
    if normalized.file_name().and_then(|value| value.to_str()) == Some("shine.toml") {
        bail!("source path must not point to shine.toml");
    }
    let valid = match runtime {
        LinkRuntime::Native => is_native_shell_script(&normalized),
        LinkRuntime::Bun => matches!(
            normalized.extension().and_then(|value| value.to_str()),
            Some("ts" | "js" | "mts" | "mjs")
        ),
    };
    if !valid {
        bail!("source path extension is incompatible with the declared runtime");
    }
    Ok(normalized)
}

pub(super) fn shell_command_name(source: &Path, target: Option<&str>) -> Result<String> {
    let command = target
        .map(str::to_string)
        .unwrap_or_else(|| link_stem(source).to_string_lossy().to_string());
    let trimmed = command.trim();
    let path = Path::new(trimmed);
    if trimmed.is_empty() || matches!(trimmed, "." | "..") || path.components().count() != 1 {
        bail!("command name must be a plain filename");
    }
    Ok(trimmed.to_string())
}

pub(super) fn is_native_shell_script(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|value| value.to_str()),
        Some("sh" | "ps1")
    )
}

pub(super) fn shell_source_matches(runtime: LinkRuntime, shell: ShellType, source: &Path) -> bool {
    if runtime == LinkRuntime::Bun {
        return true;
    }
    let is_powershell = source.extension().and_then(|value| value.to_str()) == Some("ps1");
    is_powershell == (shell == ShellType::PowerShell)
}

pub(super) fn shell_description(bytes: &[u8], runtime: LinkRuntime) -> Vec<String> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Vec::new();
    };
    let leader = if runtime == LinkRuntime::Bun {
        "//"
    } else {
        "#"
    };
    let mut description = Vec::new();
    for line in text.lines() {
        if line.starts_with("#!") {
            continue;
        }
        let trimmed = line.trim_start();
        if let Some(value) = trimmed.strip_prefix(leader) {
            let value = value.strip_prefix(' ').unwrap_or(value);
            if !value.starts_with("shine-") {
                description.push(value.to_string());
            }
        } else if !trimmed.is_empty() {
            break;
        }
    }
    while description.last().is_some_and(String::is_empty) {
        description.pop();
    }
    description
}

#[cfg(test)]
mod tests;
