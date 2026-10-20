//! Explicit, snapshot-checked single-key writes. Encryption happens outside locks.
use super::{Config, ConfigInput, EnvLayerKind};
use anyhow::{Context, Result, bail};
use clap::Args;
use shine_core::runtime::{FileSystemHost, RealHost};
use std::path::{Path, PathBuf};

#[derive(Args, Clone, Debug, Default)]
pub struct EnvWriteOptions {
    /// Write to global configuration, even inside a project
    #[arg(long, conflicts_with_all = ["project", "overlay"])]
    pub global: bool,
    /// Write to the nearest existing project; never create a project
    #[arg(long, conflicts_with = "overlay")]
    pub project: bool,
    /// Write to the active manual overlay (managed mirrors are read-only)
    #[arg(long)]
    pub overlay: bool,
    /// Use shine.env.toml in the selected scope instead of [env]
    #[arg(long)]
    pub env_file: bool,
    /// Retired: use an explicit scope and file option
    #[arg(long, hide = true)]
    pub force: bool,
}

impl EnvWriteOptions {
    fn explicit(&self) -> bool {
        self.global || self.project || self.overlay || self.env_file
    }
}

#[derive(Debug)]
pub struct EnvWritePlan {
    pub kind: EnvLayerKind,
    pub target: ConfigInput,
    inputs: Vec<ConfigInput>,
    cwd: PathBuf,
    project: Option<PathBuf>,
    context: Vec<Option<std::ffi::OsString>>,
    overlay_exists: bool,
    key: String,
    delete: bool,
}

fn context() -> Vec<Option<std::ffi::OsString>> {
    [
        "SHINE_CONFIG_DIR",
        "SHINE_PRESETS",
        "HOME",
        "SUDO_USER",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "HOMEDRIVE",
        "HOMEPATH",
    ]
    .iter()
    .map(std::env::var_os)
    .collect()
}

pub(crate) async fn lock_target(
    path: &Path,
) -> Result<shine_core::runtime::PrivilegedOperationGuard> {
    let parent = path.parent().context("write target has no parent")?;
    tokio::fs::create_dir_all(parent).await?;
    // Canonical parent makes directory aliases share the same persistent OS lock.
    let parent = tokio::fs::canonicalize(parent).await?;
    let name = path
        .file_name()
        .context("write target has no filename")?
        .to_string_lossy();
    RealHost
        .acquire_operation_lock(&parent.join(format!(".{name}.shine.lock")))
        .await
}

impl EnvWritePlan {
    pub async fn prepare(
        config: &Config,
        key: &str,
        options: &EnvWriteOptions,
        delete: bool,
    ) -> Result<Self> {
        if [options.global, options.project, options.overlay]
            .into_iter()
            .filter(|v| *v)
            .count()
            > 1
        {
            bail!("--global, --project and --overlay are mutually exclusive");
        }
        let kind = if options.overlay {
            EnvLayerKind::Overlay
        } else if options.global {
            if options.env_file {
                EnvLayerKind::GlobalEnv
            } else {
                EnvLayerKind::GlobalConfig
            }
        } else if options.project || config.is_project_config() {
            if !config.is_project_config() {
                bail!("--project requires an existing shine.config.toml; no project was created");
            }
            if options.env_file {
                EnvLayerKind::ProjectEnv
            } else {
                EnvLayerKind::ProjectConfig
            }
        } else if options.env_file {
            EnvLayerKind::GlobalEnv
        } else {
            EnvLayerKind::GlobalConfig
        };
        let path = match kind {
            EnvLayerKind::GlobalConfig => config.global_config_path(),
            EnvLayerKind::GlobalEnv => config.shine_dir().join(super::PROJECT_ENV_FILE),
            EnvLayerKind::ProjectConfig => config.config_path().to_path_buf(),
            EnvLayerKind::ProjectEnv => {
                config.config_path().with_file_name(super::PROJECT_ENV_FILE)
            }
            EnvLayerKind::Overlay => {
                if config.presets_overlay_dir_override.is_none() {
                    bail!(
                        "--overlay requires a manual overlay; managed mirrors are read-only. Edit upstream, link a manual directory, or use --global --env-file"
                    );
                }
                let overlay = config
                    .active_presets_overlay_dir()
                    .context("no active overlay")?;
                if let Some((_, _, managed)) = config.overlay_git_source()
                    && let (Ok(overlay), Ok(managed)) = (
                        tokio::fs::canonicalize(overlay).await,
                        tokio::fs::canonicalize(managed).await,
                    )
                    && overlay == managed
                {
                    bail!(
                        "managed mirrors are read-only even when linked as a manual overlay; edit upstream or use --global --env-file"
                    );
                }
                overlay.join(super::PROJECT_ENV_FILE)
            }
            EnvLayerKind::Builtin => unreachable!(),
        };
        let flags = match kind {
            EnvLayerKind::Overlay => "--overlay",
            EnvLayerKind::GlobalEnv => "--global --env-file",
            EnvLayerKind::ProjectEnv => "--project --env-file",
            EnvLayerKind::GlobalConfig => "--global",
            _ => "--project",
        };
        let example = |flags: &str| {
            if delete {
                format!("shine env delete {key} {flags}")
            } else {
                format!("shine env set {key} <VALUE> {flags}")
            }
        };
        if options.force {
            // Diagnose the file the retired redirection would have written.
            // This never computes an old effective value or changes runtime ordering.
            let former = [
                EnvLayerKind::ProjectEnv,
                EnvLayerKind::Overlay,
                EnvLayerKind::GlobalEnv,
            ]
            .into_iter()
            .find_map(|kind| {
                config
                    .env_layers
                    .iter()
                    .find(|layer| layer.kind == kind && layer.values.contains_key(key))
            })
            .or_else(|| {
                config.env_layers.iter().find(|layer| {
                    layer.kind
                        == if config.is_project_config() {
                            EnvLayerKind::ProjectConfig
                        } else {
                            EnvLayerKind::GlobalConfig
                        }
                })
            });
            let former_kind = former.map(|layer| layer.kind).unwrap_or(kind);
            let former_path = former
                .and_then(|layer| layer.path.as_deref())
                .unwrap_or(&path);
            if former_kind == EnvLayerKind::Overlay && config.presets_overlay_dir_override.is_none()
            {
                bail!(
                    "--force is retired; former target {} is a managed mirror and cannot be written. Edit upstream, use a manual --overlay, or select --global --env-file; nothing was written",
                    former_path.display()
                );
            }
            let flags = match former_kind {
                EnvLayerKind::Overlay => "--overlay",
                EnvLayerKind::GlobalEnv => "--global --env-file",
                EnvLayerKind::ProjectEnv => "--project --env-file",
                EnvLayerKind::GlobalConfig => "--global",
                EnvLayerKind::ProjectConfig => "--project",
                EnvLayerKind::Builtin => flags,
            };
            bail!(
                "--force is retired; nothing was written. Former target: {}. Replace --force with {flags} for storage key {key}. Alternative: `{}`; for encryption use `shine env secret encrypt --set {key}` with the same target options",
                former_path.display(),
                example(flags)
            );
        }
        if !options.explicit()
            && let Some(winner) = config.env_value_layer(key)
            && winner.kind > kind
        {
            let winner_flags = match winner.kind {
                EnvLayerKind::ProjectEnv => "--project --env-file",
                EnvLayerKind::GlobalEnv => "--global --env-file",
                EnvLayerKind::ProjectConfig => "--project",
                _ => "--overlay",
            };
            bail!(
                "{key} in {} would be shadowed by {} at {}. Choose the target explicitly: add {flags} to maintain this layer, or {winner_flags} to modify the effective layer; nothing was written.\nAlternative: `{}` or `{}`",
                path.display(),
                winner.kind.label(),
                winner.path.as_ref().unwrap().display(),
                example(flags),
                example(winner_flags)
            );
        }
        for input in &config.config_inputs {
            input.verify().await?;
        }
        let target = ConfigInput::capture(&path).await?;
        if let Ok(metadata) = tokio::fs::metadata(&path).await
            && metadata.permissions().readonly()
        {
            bail!("{} is not writable", path.display());
        }
        let mut parent = path.parent();
        while let Some(directory) = parent {
            match tokio::fs::metadata(directory).await {
                Ok(metadata) => {
                    if !metadata.is_dir() || (cfg!(unix) && metadata.permissions().readonly()) {
                        bail!("{} is not a writable directory", directory.display());
                    }
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    parent = directory.parent()
                }
                Err(error) => return Err(error).context("checking target directory"),
            }
        }
        let table: toml::Table = target
            .contents
            .as_deref()
            .map(|contents| toml::from_str(contents).map_err(super::config_parse_error))
            .transpose()
            .with_context(|| format!("parsing {}", path.display()))?
            .unwrap_or_default();
        let entries = if kind.is_config() {
            table.get("env").and_then(toml::Value::as_table)
        } else {
            Some(&table)
        };
        if delete && !entries.is_some_and(|table| table.contains_key(key)) {
            bail!(
                "{key} has no local entry in {}; nothing was deleted",
                path.display()
            );
        }
        let cwd = std::env::current_dir()?;
        Ok(Self {
            kind,
            target,
            inputs: config.config_inputs.clone(),
            project: super::find_project_config(&cwd).map(|p| p.path),
            cwd,
            context: context(),
            overlay_exists: config.active_presets_overlay_dir().is_some(),
            key: key.to_string(),
            delete,
        })
    }

    pub async fn commit(&self, config: &Config, key: &str, value: Option<&str>) -> Result<()> {
        if key != self.key || value.is_none() != self.delete {
            bail!("env write request differs from preflight; retry the command");
        }
        let _guard = lock_target(&self.target.path).await?;
        if std::env::current_dir()? != self.cwd
            || context() != self.context
            || super::find_project_config(&self.cwd).map(|p| p.path) != self.project
            || config.active_presets_overlay_dir().is_some() != self.overlay_exists
        {
            bail!("configuration context changed; retry the command");
        }
        for input in &self.inputs {
            input.verify().await?;
        }
        self.target.verify().await?;
        let existing = self.target.contents.as_deref().unwrap_or_default();
        let mut doc: toml_edit::DocumentMut = existing
            .parse()
            .map_err(super::config_parse_error)
            .context("parsing env write target")?;
        if self.kind == EnvLayerKind::GlobalConfig && self.target.contents.is_none() {
            doc["schema_version"] =
                toml_edit::value(i64::from(super::CURRENT_RUNTIME_SCHEMA_VERSION));
        }
        let table = if self.kind.is_config() {
            if !doc.contains_key("env") {
                doc["env"] = toml_edit::Item::Table(toml_edit::Table::new());
            }
            doc["env"]
                .as_table_like_mut()
                .context("[env] must be a table")?
        } else {
            doc.as_table_mut() as &mut dyn toml_edit::TableLike
        };
        match value {
            None => {
                table.remove(key);
            }
            Some(value) => {
                if let Some(entry) = table
                    .get_mut(key)
                    .and_then(toml_edit::Item::as_table_like_mut)
                {
                    entry.insert("value", toml_edit::value(value));
                } else {
                    table.insert(key, toml_edit::value(value));
                }
            }
        }
        crate::persist::atomic_write_private(&self.target.path, doc.to_string().as_bytes()).await
    }
}
