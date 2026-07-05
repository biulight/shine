use crate::shells::ShellType;
use anyhow::{Context, Result, bail};
use directories::UserDirs;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;

mod discovery;
use discovery::{
    ProjectConfig, find_project_config, preliminary_shine_dir_from_env,
    read_presets_override_from_toml, resolve_config_presets_path, resolve_runtime_config_dirs,
};

const LEGACY_ENV_FILE: &str = "env.toml";
const GLOBAL_CONFIG_FILE: &str = "config.toml";
const PROJECT_CONFIG_FILE: &str = "shine.config.toml";
const LEGACY_PROJECT_CONFIG_FILE: &str = "config.toml";
const PROJECT_ENV_FILE: &str = "shine.env.toml";
const LEGACY_PROJECT_ENV_FILE: &str = ".env.toml";
const LEGACY_PROJECT_FILES_REMOVAL_VERSION: &str = "v0.40.0";

pub const CURRENT_RUNTIME_SCHEMA_VERSION: u32 = 1;

pub const DEFAULT_ENV_VARS: &[(&str, &str)] = &[
    ("HTTP_PROXY_PORT", "6152"),
    ("SOCKS5_PROXY_PORT", "6153"),
    ("PROXY_HOST", "127.0.0.1"),
    ("PROXY_NO_PROXY", "localhost,127.0.0.1,::1"),
    ("GHOSTTY_BG_LIGHT", ""),
    ("GHOSTTY_BG_DARK", ""),
];

pub fn default_env_map() -> BTreeMap<String, String> {
    DEFAULT_ENV_VARS
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    /// Presets directory - computed at runtime, not serialized
    #[serde(skip)]
    presets_dir: PathBuf,
    /// Bin directory for symlinks - computed from home
    #[serde(skip)]
    bin_dir: PathBuf,
    /// Path to the active config file.
    #[serde(skip)]
    config_path: PathBuf,
    /// True when this config was loaded from a project presets config.
    #[serde(skip)]
    is_project_config: bool,
    /// Original sparse project table plus the effective config at load time.
    /// Used to avoid materializing inherited global values when saving.
    #[serde(skip)]
    project_save_state: Option<ProjectSaveState>,
    /// Directory used for shine runtime state.
    #[serde(skip)]
    shine_dir: PathBuf,
    #[serde(skip)]
    pub home_dir: PathBuf,
    /// Filename
    // Kept for Debug/diagnostics parity with the other path fields above;
    // not currently read elsewhere.
    #[serde(skip)]
    #[allow(dead_code)]
    file_name: String,
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_cleared_schema_version: Option<u32>,
    #[serde(skip)]
    pub shell_type: ShellType,
    /// Optional persistent presets_dir override stored in the active config.
    /// Takes effect when neither SHINE_CONFIG_DIR nor SHINE_PRESETS is set.
    #[serde(
        rename = "presets_dir",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub presets_dir_override: Option<PathBuf>,
    /// Optional overlay directory merged over embedded presets.
    /// Ignored when a full external presets source is active.
    #[serde(
        rename = "presets_overlay_dir",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub presets_overlay_dir_override: Option<PathBuf>,
    /// Optional override for the default destination root used by `shine app install`
    /// when a preset file carries no `shine-dest:` annotation.
    /// Defaults to `~/.config` when not set.
    #[serde(
        rename = "app_default_dest_root",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub app_default_dest_root_override: Option<PathBuf>,
    /// `true` when the presets directory is provided by the user (via env var or config
    /// `presets_dir` key) rather than the default `~/.shine/presets/`.
    /// When `true`, install commands read presets from disk directly without extracting
    /// embedded assets, and list commands enumerate the on-disk folder.
    #[serde(skip)]
    pub is_external_presets: bool,
    /// Path where `shine self install` last copied the binary.
    /// When set, `shine self upgrade` will try to sync the new binary there automatically.
    #[serde(
        rename = "self_install_dest",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub self_install_dest: Option<PathBuf>,
    /// Default GPG recipient key used by `shine env encrypt` when the command
    /// does not provide `-r/--recipient`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpg_key_id: Option<String>,
    /// Environment variables substituted into template-enabled presets.
    #[serde(
        default = "default_env_map",
        deserialize_with = "deserialize_env_values"
    )]
    pub env: BTreeMap<String, String>,
    /// Per-variable descriptions read from detailed `[env]` entries.
    #[serde(skip)]
    pub env_descriptions: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
struct ProjectSaveState {
    original: toml::Table,
    loaded: toml::Table,
}

#[derive(Default, Deserialize)]
struct ProjectOverrides {
    #[serde(default)]
    presets_dir: Option<PathBuf>,
    #[serde(default)]
    presets_overlay_dir: Option<PathBuf>,
    #[serde(default)]
    app_default_dest_root: Option<PathBuf>,
    #[serde(default)]
    self_install_dest: Option<PathBuf>,
    #[serde(default)]
    gpg_key_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_env_values")]
    env: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum EnvValue {
    Plain(String),
    Detailed {
        value: String,
        #[allow(dead_code)]
        description: Option<String>,
    },
}

fn deserialize_env_values<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values = BTreeMap::<String, EnvValue>::deserialize(deserializer)?;
    Ok(values
        .into_iter()
        .map(|(key, value)| {
            let value = match value {
                EnvValue::Plain(value) | EnvValue::Detailed { value, .. } => value,
            };
            (key, value)
        })
        .collect())
}

fn parse_env_descriptions(contents: &str) -> BTreeMap<String, String> {
    let Ok(table) = toml::from_str::<toml::Table>(contents) else {
        return BTreeMap::new();
    };
    let Some(env) = table.get("env").and_then(toml::Value::as_table) else {
        return BTreeMap::new();
    };
    env.iter()
        .filter_map(|(key, value)| {
            value
                .as_table()
                .and_then(|entry| entry.get("description"))
                .and_then(toml::Value::as_str)
                .map(|description| (key.clone(), description.to_string()))
        })
        .collect()
}

impl Config {
    pub async fn init_current_dir_config() -> Result<PathBuf> {
        let current_dir = std::env::current_dir().context("resolving current directory")?;
        let (default_shine_dir, _) = default_config_and_presets_dir()?;
        let preliminary_shine_dir = preliminary_shine_dir_from_env(&default_shine_dir);
        let global_config_path = preliminary_shine_dir.join(GLOBAL_CONFIG_FILE);
        if let Some(project_config) = find_project_config(&current_dir, &global_config_path) {
            bail!(
                "{} already exists; current directory is already under a shine project at {}",
                project_config.path.display(),
                project_config.root.display()
            );
        }

        let config_path = current_dir.join(PROJECT_CONFIG_FILE);

        let presets_dir = tokio::fs::canonicalize(&current_dir)
            .await
            .unwrap_or_else(|_| current_dir.clone());
        let shine_dir = preliminary_shine_dir;

        let config = Config {
            config_path: config_path.clone(),
            is_project_config: true,
            project_save_state: None,
            shine_dir: shine_dir.clone(),
            presets_dir: presets_dir.clone(),
            bin_dir: shine_dir.join("bin"),
            home_dir: effective_home_dir(),
            presets_dir_override: Some(PathBuf::from(".")),
            presets_overlay_dir_override: None,
            is_external_presets: true,
            ..Config::default()
        };

        config.save().await?;
        Ok(config_path)
    }

    pub async fn load_or_init() -> Result<Self> {
        let (default_shine_dir, default_presets_dir) = default_config_and_presets_dir()?;
        let current_dir = std::env::current_dir().context("resolving current directory")?;
        let preliminary_shine_dir = preliminary_shine_dir_from_env(&default_shine_dir);
        let global_config_path = preliminary_shine_dir.join(GLOBAL_CONFIG_FILE);
        let project_config = find_project_config(&current_dir, &global_config_path);
        let Some(project_config) = project_config else {
            return Self::load_global_runtime_or_init().await;
        };
        if project_config.is_legacy {
            eprintln!(
                "Warning: project config {} is deprecated and will no longer be supported in {}; rename it to {}.",
                project_config.path.display(),
                LEGACY_PROJECT_FILES_REMOVAL_VERSION,
                project_config.root.join(PROJECT_CONFIG_FILE).display()
            );
        }
        // Initialize and migrate the global layer before applying the sparse project layer.
        let (mut config, global_exists) = Self::load_global_runtime_base().await?;
        fs::create_dir_all(config.shine_dir()).await?;
        fs::create_dir_all(config.presets_dir()).await?;
        fs::create_dir_all(config.bin_dir()).await?;
        let global_has_env = if global_exists {
            let contents = fs::read_to_string(config.config_path()).await?;
            config_toml_has_env_table(&contents)
        } else {
            false
        };
        config.migrate_env(global_has_env).await?;

        let contents = fs::read_to_string(&project_config.path)
            .await
            .context("Failed to read project config file")?;
        let original: toml::Table =
            toml::from_str(&contents).context("Failed to parse project config file")?;
        let overrides: ProjectOverrides =
            toml::from_str(&contents).context("Failed to parse project config file")?;

        let project_presets = overrides
            .presets_dir
            .map(|path| resolve_config_presets_path(&path, &project_config.root));
        if let Some(path) = overrides.presets_overlay_dir {
            config.presets_overlay_dir_override =
                Some(resolve_config_presets_path(&path, &project_config.root));
        }
        if let Some(path) = overrides.app_default_dest_root {
            config.app_default_dest_root_override =
                Some(resolve_config_presets_path(&path, &project_config.root));
        }
        if let Some(path) = overrides.self_install_dest {
            config.self_install_dest =
                Some(resolve_config_presets_path(&path, &project_config.root));
        }
        if overrides.gpg_key_id.is_some() {
            config.gpg_key_id = overrides.gpg_key_id;
        }
        config.env.extend(overrides.env);
        config
            .env_descriptions
            .extend(parse_env_descriptions(&contents));

        let effective_presets = project_presets.clone().or_else(|| {
            config
                .is_external_presets
                .then(|| config.presets_dir().to_path_buf())
        });
        if let Some(path) = &effective_presets {
            config.presets_dir_override = Some(path.clone());
        }
        // SHINE_CONFIG_DIR's presets default outranks an inherited global setting,
        // while an explicit project setting keeps the established local override behavior.
        let runtime_presets = if project_presets.is_none()
            && std::env::var("SHINE_CONFIG_DIR").is_ok_and(|value| !value.trim().is_empty())
        {
            None
        } else {
            effective_presets.clone()
        };
        let (shine_dir, presets_dir, is_external_presets) = resolve_runtime_config_dirs(
            &default_shine_dir,
            &default_presets_dir,
            runtime_presets.as_deref(),
            true,
        );
        config.config_path = project_config.path.clone();
        config.is_project_config = true;
        config.shine_dir = shine_dir;
        config.presets_dir = presets_dir;
        config.bin_dir = config.shine_dir.join("bin");
        config.is_external_presets = is_external_presets;
        fs::create_dir_all(config.presets_dir()).await?;
        fs::create_dir_all(config.bin_dir()).await?;

        // Environment override files deliberately sit above both TOML layers.
        config.apply_global_env_override().await?;
        config.apply_project_env_override(&project_config).await?;
        crate::presets::set_overlay_dir(config.active_presets_overlay_dir());

        let loaded = config.serialize_effective_table()?;
        config.project_save_state = Some(ProjectSaveState { original, loaded });
        Ok(config)
    }

    pub async fn load_global_runtime_or_init() -> Result<Self> {
        let (mut config, exists) = Self::load_global_runtime_base().await?;

        fs::create_dir_all(config.shine_dir())
            .await
            .with_context(|| "creating shine config dir")?;
        fs::create_dir_all(config.presets_dir())
            .await
            .with_context(|| "creating presets dir")?;
        fs::create_dir_all(config.bin_dir())
            .await
            .with_context(|| "creating bin dir")?;

        let config_has_env = if exists {
            let contents = fs::read_to_string(config.config_path())
                .await
                .context("Failed to read global config file")?;
            config_toml_has_env_table(&contents)
        } else {
            false
        };
        config.migrate_env(config_has_env).await?;
        config.apply_global_env_override().await?;
        Ok(config)
    }

    pub async fn load_global_runtime_for_dry_run() -> Result<Self> {
        let (config, _) = Self::load_global_runtime_base().await?;
        Ok(config)
    }

    async fn load_global_runtime_base() -> Result<(Self, bool)> {
        let home_dir = effective_home_dir();
        let (default_shine_dir, default_presets_dir) = default_config_and_presets_dir()?;
        let preliminary_shine_dir = preliminary_shine_dir_from_env(&default_shine_dir);
        let config_path = preliminary_shine_dir.join(GLOBAL_CONFIG_FILE);
        let config_dir = config_path
            .parent()
            .context("Config path must have a parent directory")?
            .to_path_buf();
        let toml_presets = read_presets_override_from_toml(&config_path).await;
        let toml_presets = toml_presets
            .as_deref()
            .map(|path| resolve_config_presets_path(path, &config_dir));

        let (shine_dir, presets_dir, is_external_presets) = resolve_runtime_config_dirs(
            &default_shine_dir,
            &default_presets_dir,
            toml_presets.as_deref(),
            false,
        );
        let bin_dir = shine_dir.join("bin");

        if config_path.exists() {
            let contents = fs::read_to_string(&config_path)
                .await
                .context("Failed to read global config file")?;
            let mut config: Config =
                toml::from_str(&contents).context("Failed to parse global config file")?;
            config.env_descriptions = parse_env_descriptions(&contents);
            config.config_path = config_path.clone();
            config.is_project_config = false;
            config.shine_dir = shine_dir;
            config.presets_dir = presets_dir;
            config.bin_dir = bin_dir;
            config.home_dir = home_dir;
            config.is_external_presets = is_external_presets;
            config.resolve_presets_overlay_dir(&config_dir);
            if let Some(path) = config.app_default_dest_root_override.as_deref() {
                config.app_default_dest_root_override =
                    Some(resolve_config_presets_path(path, &config_dir));
            }
            if let Some(path) = config.self_install_dest.as_deref() {
                config.self_install_dest = Some(resolve_config_presets_path(path, &config_dir));
            }
            crate::presets::set_overlay_dir(config.active_presets_overlay_dir());
            Ok((config, true))
        } else {
            let config = Config {
                config_path: config_path.clone(),
                is_project_config: false,
                project_save_state: None,
                shine_dir,
                presets_dir,
                bin_dir,
                home_dir,
                is_external_presets,
                ..Config::default()
            };
            crate::presets::set_overlay_dir(config.active_presets_overlay_dir());
            Ok((config, false))
        }
    }

    pub async fn read_global_runtime_schema_version() -> Result<u32> {
        let (default_shine_dir, _) = default_config_and_presets_dir()?;
        let config_path =
            preliminary_shine_dir_from_env(&default_shine_dir).join(GLOBAL_CONFIG_FILE);
        let content = match fs::read_to_string(&config_path).await {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(CURRENT_RUNTIME_SCHEMA_VERSION);
            }
            Err(e) => {
                return Err(e).with_context(|| format!("Failed to read {}", config_path.display()));
            }
        };

        #[derive(Deserialize)]
        struct MinimalConfig {
            #[serde(default)]
            schema_version: u32,
        }

        toml::from_str::<MinimalConfig>(&content)
            .map(|config| config.schema_version)
            .with_context(|| format!("Failed to parse {}", config_path.display()))
    }

    pub fn presets_dir(&self) -> &Path {
        &self.presets_dir
    }

    pub fn bin_dir(&self) -> &Path {
        &self.bin_dir
    }

    pub fn shine_dir(&self) -> &Path {
        &self.shine_dir
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    /// Directory where template-rendered shell scripts are written.
    /// Always inside shine_dir so it is never confused with user-owned presets.
    pub fn rendered_dir(&self) -> PathBuf {
        self.shine_dir().join("rendered")
    }

    pub fn app_default_dest_root(&self) -> PathBuf {
        match &self.app_default_dest_root_override {
            Some(p) => {
                let s = p.to_str().unwrap_or("~/.config");
                PathBuf::from(tilde_expand(s))
            }
            None => self.home_dir.join(".config"),
        }
    }

    pub fn new_for_test(dir: &Path) -> Self {
        Self {
            config_path: dir.join("config.toml"),
            is_project_config: false,
            project_save_state: None,
            shine_dir: dir.to_path_buf(),
            presets_dir: dir.join("presets"),
            bin_dir: dir.join("bin"),
            home_dir: dir.to_path_buf(),
            file_name: "config.toml".to_string(),
            schema_version: CURRENT_RUNTIME_SCHEMA_VERSION,
            last_cleared_schema_version: None,
            shell_type: ShellType::default(),
            presets_dir_override: None,
            presets_overlay_dir_override: None,
            app_default_dest_root_override: None,
            is_external_presets: false,
            self_install_dest: None,
            gpg_key_id: None,
            env: default_env_map(),
            env_descriptions: BTreeMap::new(),
        }
    }

    /// Return a clone of this config with `presets_dir_override` replaced.
    pub fn with_presets_dir_override(self, value: Option<PathBuf>) -> Self {
        Self {
            presets_dir_override: value,
            ..self
        }
    }

    /// Return a clone of this config with `presets_overlay_dir_override` replaced.
    pub fn with_presets_overlay_dir_override(self, value: Option<PathBuf>) -> Self {
        Self {
            presets_overlay_dir_override: value,
            ..self
        }
    }

    pub fn active_presets_overlay_dir(&self) -> Option<&Path> {
        if self.is_external_presets {
            None
        } else {
            self.presets_overlay_dir_override.as_deref()
        }
    }

    fn resolve_presets_overlay_dir(&mut self, config_dir: &Path) {
        if let Some(path) = self.presets_overlay_dir_override.as_deref() {
            self.presets_overlay_dir_override = Some(resolve_config_presets_path(path, config_dir));
        }
    }

    pub async fn save(&self) -> Result<()> {
        let config_path = self.resolve_config_path_for_save().await?;

        let shine_dir = config_path
            .parent()
            .context("Config path must have a parent directory")?;

        let new_table = self.serialize_table_for_save()?;
        let new_toml = toml::to_string_pretty(&new_table).context("Failed to serialize config")?;

        let toml_str = if config_path.exists() {
            let existing = fs::read_to_string(&config_path).await.unwrap_or_default();
            if existing.is_empty() {
                new_toml
            } else {
                let mut doc: toml_edit::DocumentMut = existing
                    .parse()
                    .context("Fail to parse existing config for comment preservation")?;

                utils::migration::sync_table(doc.as_table_mut(), &new_table);
                doc.to_string()
            }
        } else {
            new_toml
        };

        let file_name = config_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let temp_path = shine_dir.join(format!(".{file_name}.tmp-{}", uuid::Uuid::new_v4()));

        let mut temp_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .await
            .with_context(|| format!("Failed to create temp file: {temp_path:?}"))?;

        temp_file
            .write_all(toml_str.as_bytes())
            .await
            .context("Failed to write temporary config contents")?;
        temp_file
            .sync_all()
            .await
            .context("Failed to sync temporary config contents")?;
        drop(temp_file);

        fs::rename(&temp_path, &config_path)
            .await
            .with_context(|| format!("Failed to rename temp file to {:?}", config_path))?;

        Ok(())
    }

    fn serialize_table_for_save(&self) -> Result<toml::Table> {
        let table = self.serialize_effective_table()?;

        if self.is_project_config {
            let mut sparse = if let Some(state) = &self.project_save_state {
                let mut sparse = state.original.clone();
                apply_table_changes(&mut sparse, &state.loaded, &table);
                sparse
            } else {
                table
            };
            sparse.remove("schema_version");
            sparse.remove("last_cleared_schema_version");
            return Ok(sparse);
        }

        Ok(table)
    }

    fn serialize_effective_table(&self) -> Result<toml::Table> {
        let serialized = toml::to_string_pretty(self).context("Failed to serialize config")?;
        toml::from_str(&serialized).context("Failed to round-trip serialize config")
    }

    async fn resolve_config_path_for_save(&self) -> Result<PathBuf> {
        if self
            .config_path
            .parent()
            .is_some_and(|parent| !parent.as_os_str().is_empty())
        {
            return Ok(self.config_path.clone());
        }
        bail!("config path must not be empty");
    }

    async fn migrate_env(&mut self, config_has_env: bool) -> Result<()> {
        let mut needs_save = !config_has_env;

        let legacy_path = self.shine_dir().join(LEGACY_ENV_FILE);
        let legacy_env = read_env_file(&legacy_path).await?;
        let has_legacy = legacy_env.is_some();

        if let Some(vars) = legacy_env {
            for (key, value) in vars {
                if config_has_env {
                    if let std::collections::btree_map::Entry::Vacant(entry) = self.env.entry(key) {
                        entry.insert(value);
                        needs_save = true;
                    }
                } else {
                    self.env.insert(key, value);
                    needs_save = true;
                }
            }
        }

        for (key, value) in DEFAULT_ENV_VARS {
            if let std::collections::btree_map::Entry::Vacant(entry) =
                self.env.entry(key.to_string())
            {
                entry.insert(value.to_string());
                needs_save = true;
            }
        }

        if needs_save {
            self.save().await?;
        }

        if has_legacy {
            match fs::remove_file(&legacy_path).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(e)
                        .with_context(|| format!("failed to remove {}", legacy_path.display()));
                }
            }
        }

        Ok(())
    }

    async fn apply_global_env_override(&mut self) -> Result<()> {
        let env_path = self.shine_dir().join(PROJECT_ENV_FILE);
        let Some(vars) = read_env_file(&env_path).await? else {
            return Ok(());
        };

        for (key, value) in vars {
            self.env.insert(key, value);
        }

        Ok(())
    }

    async fn apply_project_env_override(&mut self, project_config: &ProjectConfig) -> Result<()> {
        let env_path = project_config.root.join(PROJECT_ENV_FILE);
        let (env_path, is_legacy) = if env_path.exists() {
            (env_path, false)
        } else {
            let legacy_env_path = project_config.root.join(LEGACY_PROJECT_ENV_FILE);
            (legacy_env_path, true)
        };
        let Some(vars) = read_env_file(&env_path).await? else {
            return Ok(());
        };

        if is_legacy {
            eprintln!(
                "Warning: project env {} is deprecated and will no longer be supported in {}; rename it to {}.",
                env_path.display(),
                LEGACY_PROJECT_FILES_REMOVAL_VERSION,
                project_config.root.join(PROJECT_ENV_FILE).display()
            );
        }

        for (key, value) in vars {
            self.env.insert(key, value);
        }

        Ok(())
    }
}

/// Print a note showing the active external presets directory.
/// No-op when the embedded presets are in use.
pub fn print_presets_note(config: &Config) {
    if config.is_external_presets {
        println!(
            "{}",
            crate::colors::external_presets_note(config.presets_dir())
        );
        if let Some(dir) = &config.presets_overlay_dir_override {
            println!(
                "{}",
                crate::colors::yellow(&format!(
                    "Presets overlay configured but inactive: {}",
                    crate::path_display::format(dir)
                ))
            );
        }
        println!();
    } else if let Some(dir) = config.active_presets_overlay_dir() {
        println!("{}", crate::colors::presets_overlay_note(dir));
        println!();
    }
}

impl Default for Config {
    fn default() -> Self {
        let home_dir = effective_home_dir();
        let shine_dir = home_dir.join(".shine");

        Self {
            presets_dir: shine_dir.join("presets"),
            bin_dir: shine_dir.join("bin"),
            config_path: shine_dir.join("config.toml"),
            is_project_config: false,
            project_save_state: None,
            shine_dir,
            home_dir,
            file_name: "config.toml".to_string(),
            schema_version: CURRENT_RUNTIME_SCHEMA_VERSION,
            last_cleared_schema_version: None,
            shell_type: ShellType::default(),
            presets_dir_override: None,
            presets_overlay_dir_override: None,
            app_default_dest_root_override: None,
            is_external_presets: false,
            self_install_dest: None,
            gpg_key_id: None,
            env: default_env_map(),
            env_descriptions: BTreeMap::new(),
        }
    }
}

fn config_toml_has_env_table(contents: &str) -> bool {
    toml::from_str::<toml::Table>(contents)
        .map(|table| table.contains_key("env"))
        .unwrap_or(false)
}

fn apply_table_changes(target: &mut toml::Table, loaded: &toml::Table, current: &toml::Table) {
    let keys: std::collections::BTreeSet<_> = loaded.keys().chain(current.keys()).collect();
    for key in keys {
        match (loaded.get(key), current.get(key)) {
            (Some(toml::Value::Table(before)), Some(toml::Value::Table(after))) => {
                let entry = target
                    .entry(key.clone())
                    .or_insert_with(|| toml::Value::Table(toml::Table::new()));
                if !entry.is_table() {
                    *entry = toml::Value::Table(toml::Table::new());
                }
                apply_table_changes(entry.as_table_mut().unwrap(), before, after);
                if entry.as_table().is_some_and(toml::Table::is_empty) {
                    target.remove(key);
                }
            }
            (before, after) if before == after => {}
            (_, Some(value)) => {
                target.insert(key.clone(), value.clone());
            }
            (_, None) => {
                target.remove(key);
            }
        }
    }
}

async fn read_env_file(path: &Path) -> Result<Option<BTreeMap<String, String>>> {
    let content = match fs::read_to_string(path).await {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };

    let table: toml::Table =
        toml::from_str(&content).with_context(|| format!("parsing {}", path.display()))?;
    let vars = table
        .into_iter()
        .filter_map(|(key, value)| value.as_str().map(|value| (key, value.to_string())))
        .collect();
    Ok(Some(vars))
}

/// Return the home directory of the original (pre-sudo) user when the process
/// is running under `sudo`, or `None` if not applicable.
///
/// `sudo` sets `SUDO_USER` to the invoking user's login name and resets `HOME`
/// to root's home, causing the config to be read from the wrong directory.
/// We resolve the correct home by looking up the user in the passwd database.
#[cfg(unix)]
fn sudo_user_home() -> Option<PathBuf> {
    let sudo_user = std::env::var("SUDO_USER").ok()?;
    let sudo_user = sudo_user.trim();
    if sudo_user.is_empty() || sudo_user == "root" {
        return None;
    }
    // /etc/passwd is authoritative for local accounts on both Linux and macOS.
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    for line in passwd.lines() {
        let mut fields = line.splitn(7, ':');
        let username = fields.next()?;
        if username != sudo_user {
            continue;
        }
        // passwd field order: name:password:uid:gid:gecos:home:shell
        let home = fields.nth(4)?; // skip password, uid, gid, gecos (index 1-4)
        if !home.is_empty() {
            return Some(PathBuf::from(home));
        }
    }
    None
}

#[cfg(not(unix))]
fn sudo_user_home() -> Option<PathBuf> {
    None
}

fn effective_home_dir() -> PathBuf {
    if let Some(home) = sudo_user_home() {
        return home;
    }
    if let Ok(home) = std::env::var("HOME") {
        let home = home.trim().to_string();
        if !home.is_empty() {
            return PathBuf::from(home);
        }
    }
    UserDirs::new().map_or_else(|| PathBuf::from("."), |u| u.home_dir().to_path_buf())
}

/// Expand a leading `~` using the effective home directory instead of `HOME`.
/// Needed because `sudo` resets `HOME` to `/root`.
pub fn tilde_expand(s: &str) -> String {
    let home = effective_home_dir().to_string_lossy().into_owned();
    shellexpand::tilde_with_context(s, || Some(home)).into_owned()
}

/// Like `shellexpand::full` but uses the effective home for both `~` and `$HOME`.
pub fn full_expand(s: &str) -> Result<String, shellexpand::LookupError<std::env::VarError>> {
    full_expand_with_home(s, &effective_home_dir())
}

/// Like `full_expand` but takes an explicit home directory instead of reading the environment.
/// Use this when a `Config` is available — pass `&config.home_dir` to avoid a data race in tests.
pub fn full_expand_with_home(
    s: &str,
    home: &std::path::Path,
) -> Result<String, shellexpand::LookupError<std::env::VarError>> {
    let home = home.to_string_lossy().into_owned();
    let home2 = home.clone();
    shellexpand::full_with_context(
        s,
        move || Some(home),
        move |var| {
            if var == "HOME" {
                return Ok(Some(home2.clone()));
            }
            match std::env::var(var) {
                Ok(v) => Ok(Some(v)),
                Err(std::env::VarError::NotPresent) => Ok(None),
                Err(e) => Err(e),
            }
        },
    )
    .map(|c| c.into_owned())
}

fn default_config_dir() -> Result<PathBuf> {
    Ok(effective_home_dir().join(".shine"))
}

fn default_config_and_presets_dir() -> Result<(PathBuf, PathBuf)> {
    let config_dir = default_config_dir()?;
    Ok((config_dir.clone(), config_dir.join("presets")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::env_lock;

    fn config_in(dir: &Path) -> Config {
        Config {
            config_path: dir.join("config.toml"),
            is_project_config: false,
            project_save_state: None,
            shine_dir: dir.to_path_buf(),
            presets_dir: dir.join("presets"),
            bin_dir: dir.join("bin"),
            home_dir: dir.join("home"),
            file_name: "config.toml".to_string(),
            schema_version: CURRENT_RUNTIME_SCHEMA_VERSION,
            last_cleared_schema_version: None,
            shell_type: ShellType::default(),
            presets_dir_override: None,
            presets_overlay_dir_override: None,
            app_default_dest_root_override: None,
            is_external_presets: false,
            self_install_dest: None,
            gpg_key_id: None,
            env: default_env_map(),
            env_descriptions: BTreeMap::new(),
        }
    }

    async fn make_temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shine-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();
        dir
    }

    fn restore_current_dir(dir: &Path) {
        std::env::set_current_dir(dir).expect("restore current dir");
    }

    #[tokio::test]
    async fn save_writes_config_file_for_new_config() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert_eq!(
            parsed["schema_version"].as_integer(),
            Some(CURRENT_RUNTIME_SCHEMA_VERSION.into())
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_writes_new_toml_when_existing_file_is_empty() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);
        fs::write(&config.config_path, b"").await.unwrap();

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        assert!(!content.is_empty());
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert!(parsed.contains_key("schema_version"));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_merges_updates_changed_value() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);
        fs::write(&config.config_path, "schema_version = 0\n")
            .await
            .unwrap();

        let updated = Config {
            schema_version: 2,
            ..config
        };
        updated.save().await.unwrap();

        let content = fs::read_to_string(&updated.config_path).await.unwrap();
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert_eq!(parsed["schema_version"].as_integer(), Some(2));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_writes_last_cleared_schema_version_when_set() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        config.last_cleared_schema_version = Some(1);

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert_eq!(parsed["last_cleared_schema_version"].as_integer(), Some(1));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_merges_preserves_comments() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        config.schema_version = 0;
        fs::write(&config.config_path, "# keep this\nschema_version = 0\n")
            .await
            .unwrap();

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        assert!(
            content.contains("# keep this"),
            "comment should be preserved"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn detailed_env_values_are_normalized_with_descriptions() {
        let contents = r#"
            [env]
            PLAIN = "value"
            TOKEN = { value = "secret", description = "Internal token" }
        "#;
        let config: Config = toml::from_str(contents).unwrap();

        assert_eq!(config.env.get("PLAIN").map(String::as_str), Some("value"));
        assert_eq!(config.env.get("TOKEN").map(String::as_str), Some("secret"));
        assert_eq!(
            parse_env_descriptions(contents)
                .get("TOKEN")
                .map(String::as_str),
            Some("Internal token")
        );
    }

    #[tokio::test]
    async fn save_updates_detailed_env_value_without_losing_description() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        fs::write(
            &config.config_path,
            "[env]\nMY_TOKEN = { value = \"old\", description = \"Internal token\" }\n",
        )
        .await
        .unwrap();
        config.env.insert("MY_TOKEN".into(), "new".into());

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        assert!(
            content.contains("MY_TOKEN = { value = \"new\", description = \"Internal token\" }")
        );
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_merges_removes_stale_keys() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);
        fs::write(
            &config.config_path,
            "schema_version = 0\nstale_key = \"old\"\n",
        )
        .await
        .unwrap();

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        assert!(
            !content.contains("stale_key"),
            "stale key should be removed"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_returns_error_for_path_without_parent() {
        let config = Config {
            config_path: PathBuf::from("config.toml"),
            is_project_config: false,
            project_save_state: None,
            shine_dir: PathBuf::from("shine"),
            presets_dir: PathBuf::from("presets"),
            bin_dir: PathBuf::from("bin"),
            home_dir: PathBuf::from("home"),
            file_name: "config.toml".to_string(),
            schema_version: CURRENT_RUNTIME_SCHEMA_VERSION,
            last_cleared_schema_version: None,
            shell_type: ShellType::default(),
            presets_dir_override: None,
            presets_overlay_dir_override: None,
            app_default_dest_root_override: None,
            is_external_presets: false,
            self_install_dest: None,
            gpg_key_id: None,
            env: default_env_map(),
            env_descriptions: BTreeMap::new(),
        };
        assert!(config.save().await.is_err());
    }

    #[test]
    fn new_for_test_bin_dir_is_under_root() {
        let dir = std::env::temp_dir().join("shine-test-bin-dir");
        let config = Config::new_for_test(&dir);
        assert_eq!(config.bin_dir(), dir.join("bin"));
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn load_or_init_creates_bin_dir() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", dir.to_str().unwrap()) };

        let config = Config::load_or_init().await.unwrap();
        assert!(config.bin_dir().exists(), "bin dir should be created");
        assert_eq!(config.bin_dir(), dir.join("bin"));

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn load_or_init_creates_env_table_in_config() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", dir.to_str().unwrap()) };

        let config = Config::load_or_init().await.unwrap();

        assert_eq!(
            config.env.get("HTTP_PROXY_PORT").map(String::as_str),
            Some("6152")
        );
        let content = fs::read_to_string(dir.join("config.toml")).await.unwrap();
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert!(
            parsed.get("env").is_some(),
            "config.toml should contain [env]"
        );

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn load_or_init_migrates_legacy_env_file_and_deletes_it() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        fs::write(
            dir.join("env.toml"),
            "HTTP_PROXY_PORT = \"7890\"\nCUSTOM_TOKEN = \"abc\"\n",
        )
        .await
        .unwrap();
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", dir.to_str().unwrap()) };

        let config = Config::load_or_init().await.unwrap();

        assert_eq!(
            config.env.get("HTTP_PROXY_PORT").map(String::as_str),
            Some("7890")
        );
        assert_eq!(
            config.env.get("CUSTOM_TOKEN").map(String::as_str),
            Some("abc")
        );
        assert!(
            !dir.join("env.toml").exists(),
            "legacy env.toml should be removed"
        );

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn load_or_init_reads_global_shine_env_without_presets_dir() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        fs::write(
            dir.join("config.toml"),
            "[env]\nHTTP_PROXY_PORT = \"1111\"\nCONFIG_ONLY = \"config\"\n",
        )
        .await
        .unwrap();
        fs::write(
            dir.join("shine.env.toml"),
            "HTTP_PROXY_PORT = \"2222\"\nSIDECAR_ONLY = \"sidecar\"\n",
        )
        .await
        .unwrap();

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", dir.to_str().unwrap()) };
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_PRESETS") };

        let config = Config::load_or_init().await.unwrap();

        assert_eq!(
            config.env.get("HTTP_PROXY_PORT").map(String::as_str),
            Some("2222")
        );
        assert_eq!(
            config.env.get("CONFIG_ONLY").map(String::as_str),
            Some("config")
        );
        assert_eq!(
            config.env.get("SIDECAR_ONLY").map(String::as_str),
            Some("sidecar")
        );
        let content = fs::read_to_string(dir.join("config.toml")).await.unwrap();
        assert!(
            !content.contains("SIDECAR_ONLY"),
            "global shine.env.toml overrides should not be written into config.toml"
        );

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn load_or_init_discovers_project_config_from_child_dir() {
        let _guard = env_lock();
        let original_dir = std::env::current_dir().unwrap();
        let project_dir = make_temp_dir().await;
        let child_dir = project_dir.join("presets/shell/proxy");
        fs::create_dir_all(&child_dir).await.unwrap();
        let state_dir = make_temp_dir().await;
        fs::write(
            project_dir.join("shine.config.toml"),
            "presets_dir = \".\"\n[env]\nHTTP_PROXY_PORT = \"1111\"\nCONFIG_ONLY = \"config\"\n",
        )
        .await
        .unwrap();
        fs::write(
            project_dir.join("shine.env.toml"),
            "HTTP_PROXY_PORT = \"2222\"\nDOTENV_ONLY = \"dotenv\"\n",
        )
        .await
        .unwrap();
        fs::write(
            project_dir.join(".env.toml"),
            "HTTP_PROXY_PORT = \"3333\"\n",
        )
        .await
        .unwrap();

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", state_dir.to_str().unwrap()) };
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_PRESETS") };
        std::env::set_current_dir(&child_dir).unwrap();

        let config = Config::load_or_init().await.unwrap();

        assert_eq!(
            fs::canonicalize(&config.config_path).await.unwrap(),
            fs::canonicalize(project_dir.join("shine.config.toml"))
                .await
                .unwrap()
        );
        assert_eq!(config.shine_dir(), state_dir);
        assert_eq!(config.bin_dir(), state_dir.join("bin"));
        assert_eq!(
            fs::canonicalize(config.presets_dir()).await.unwrap(),
            fs::canonicalize(&project_dir).await.unwrap()
        );
        assert_eq!(
            config.env.get("HTTP_PROXY_PORT").map(String::as_str),
            Some("2222")
        );
        assert_eq!(
            config.env.get("CONFIG_ONLY").map(String::as_str),
            Some("config")
        );
        assert_eq!(
            config.env.get("DOTENV_ONLY").map(String::as_str),
            Some("dotenv")
        );

        restore_current_dir(&original_dir);
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        fs::remove_dir_all(&project_dir).await.unwrap();
        fs::remove_dir_all(&state_dir).await.unwrap();
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn load_global_runtime_ignores_project_config() {
        let _guard = env_lock();
        let original_dir = std::env::current_dir().unwrap();
        let project_dir = make_temp_dir().await;
        let child_dir = project_dir.join("subdir");
        fs::create_dir_all(&child_dir).await.unwrap();
        let state_dir = make_temp_dir().await;
        fs::write(
            project_dir.join("shine.config.toml"),
            "schema_version = 7\npresets_dir = \".\"\n",
        )
        .await
        .unwrap();
        fs::write(
            state_dir.join("config.toml"),
            "schema_version = 0\nlast_cleared_schema_version = 0\n",
        )
        .await
        .unwrap();

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", state_dir.to_str().unwrap()) };
        std::env::set_current_dir(&child_dir).unwrap();

        let config = Config::load_global_runtime_or_init().await.unwrap();

        assert_eq!(
            fs::canonicalize(config.config_path()).await.unwrap(),
            fs::canonicalize(state_dir.join("config.toml"))
                .await
                .unwrap()
        );
        assert_eq!(config.schema_version, 0);
        assert_eq!(config.last_cleared_schema_version, Some(0));
        assert_eq!(config.shine_dir(), state_dir);

        restore_current_dir(&original_dir);
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        fs::remove_dir_all(&project_dir).await.unwrap();
        fs::remove_dir_all(&state_dir).await.unwrap();
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn load_global_runtime_for_dry_run_does_not_create_state() {
        let _guard = env_lock();
        let dir = std::env::temp_dir().join(format!("shine-dry-run-{}", uuid::Uuid::new_v4()));
        assert!(!dir.exists());

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", dir.to_str().unwrap()) };

        let config = Config::load_global_runtime_for_dry_run().await.unwrap();

        assert_eq!(config.config_path(), dir.join("config.toml"));
        assert_eq!(config.schema_version, CURRENT_RUNTIME_SCHEMA_VERSION);
        assert!(!dir.exists(), "dry-run loader must not create state dir");

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn project_shine_env_overrides_global_shine_env() {
        let _guard = env_lock();
        let original_dir = std::env::current_dir().unwrap();
        let project_dir = make_temp_dir().await;
        let child_dir = project_dir.join("subdir");
        fs::create_dir_all(&child_dir).await.unwrap();
        let state_dir = make_temp_dir().await;
        fs::write(
            project_dir.join("shine.config.toml"),
            "presets_dir = \".\"\n[env]\nHTTP_PROXY_PORT = \"1111\"\nCONFIG_ONLY = \"config\"\n",
        )
        .await
        .unwrap();
        fs::write(
            state_dir.join("shine.env.toml"),
            "HTTP_PROXY_PORT = \"2222\"\nGLOBAL_ONLY = \"global\"\n",
        )
        .await
        .unwrap();
        fs::write(
            project_dir.join("shine.env.toml"),
            "HTTP_PROXY_PORT = \"3333\"\nPROJECT_ONLY = \"project\"\n",
        )
        .await
        .unwrap();

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", state_dir.to_str().unwrap()) };
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_PRESETS") };
        std::env::set_current_dir(&child_dir).unwrap();

        let config = Config::load_or_init().await.unwrap();

        assert_eq!(
            config.env.get("HTTP_PROXY_PORT").map(String::as_str),
            Some("3333")
        );
        assert_eq!(
            config.env.get("CONFIG_ONLY").map(String::as_str),
            Some("config")
        );
        assert_eq!(
            config.env.get("GLOBAL_ONLY").map(String::as_str),
            Some("global")
        );
        assert_eq!(
            config.env.get("PROJECT_ONLY").map(String::as_str),
            Some("project")
        );

        restore_current_dir(&original_dir);
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        fs::remove_dir_all(&project_dir).await.unwrap();
        fs::remove_dir_all(&state_dir).await.unwrap();
    }

    #[tokio::test]
    async fn project_config_discovery_prefers_new_name_over_legacy_config() {
        let project_dir = make_temp_dir().await;
        let child_dir = project_dir.join("subdir");
        let global_config = make_temp_dir().await.join("config.toml");
        fs::create_dir_all(&child_dir).await.unwrap();
        fs::write(
            project_dir.join("shine.config.toml"),
            "presets_dir = \".\"\n",
        )
        .await
        .unwrap();
        fs::write(
            project_dir.join("config.toml"),
            "presets_dir = \"legacy\"\n",
        )
        .await
        .unwrap();

        let config = find_project_config(&child_dir, &global_config)
            .expect("project config should be found");

        assert_eq!(config.path, project_dir.join("shine.config.toml"));
        assert!(!config.is_legacy);

        fs::remove_dir_all(&project_dir).await.unwrap();
        fs::remove_dir_all(global_config.parent().unwrap())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn legacy_config_without_presets_dir_is_not_project_config() {
        let project_dir = make_temp_dir().await;
        let global_config = make_temp_dir().await.join("config.toml");
        fs::write(project_dir.join("config.toml"), "name = \"other-app\"\n")
            .await
            .unwrap();

        assert!(find_project_config(&project_dir, &global_config).is_none());

        fs::remove_dir_all(&project_dir).await.unwrap();
        fs::remove_dir_all(global_config.parent().unwrap())
            .await
            .unwrap();
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn global_config_with_presets_dir_is_not_legacy_project_config() {
        let _guard = env_lock();
        let original_dir = std::env::current_dir().unwrap();
        let original_home = std::env::var("HOME").ok();
        let home_dir = make_temp_dir().await;
        let shine_dir = home_dir.join(".shine");
        let child_dir = shine_dir.join("presets/shell/proxy");
        let external_presets = make_temp_dir().await.join("presets");
        fs::create_dir_all(&child_dir).await.unwrap();
        fs::create_dir_all(&external_presets).await.unwrap();
        fs::write(
            shine_dir.join("config.toml"),
            format!(
                "schema_version = 1\npresets_dir = \"{}\"\n",
                external_presets.display()
            ),
        )
        .await
        .unwrap();

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("HOME", home_dir.to_str().unwrap()) };
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_PRESETS") };
        std::env::set_current_dir(&child_dir).unwrap();

        let config = Config::load_or_init().await.unwrap();

        assert_eq!(
            fs::canonicalize(config.config_path()).await.unwrap(),
            fs::canonicalize(shine_dir.join("config.toml"))
                .await
                .unwrap()
        );
        assert!(
            !config.is_project_config,
            "global config.toml must not be treated as a legacy project config"
        );
        assert_eq!(
            fs::canonicalize(config.presets_dir()).await.unwrap(),
            fs::canonicalize(&external_presets).await.unwrap()
        );

        restore_current_dir(&original_dir);
        match original_home {
            Some(home) => {
                // SAFETY: env_lock() is held for the duration of this block, preventing
                //          concurrent env mutation from other threads in this test binary.
                unsafe { std::env::set_var("HOME", home) };
            }
            None => {
                // SAFETY: env_lock() is held for the duration of this block, preventing
                //          concurrent env mutation from other threads in this test binary.
                unsafe { std::env::remove_var("HOME") };
            }
        }
        fs::remove_dir_all(&home_dir).await.unwrap();
        fs::remove_dir_all(external_presets.parent().unwrap())
            .await
            .unwrap();
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn load_or_init_supports_legacy_project_config_and_env_when_new_files_absent() {
        let _guard = env_lock();
        let original_dir = std::env::current_dir().unwrap();
        let project_dir = make_temp_dir().await;
        let child_dir = project_dir.join("subdir");
        fs::create_dir_all(&child_dir).await.unwrap();
        let state_dir = make_temp_dir().await;
        fs::write(
            project_dir.join("config.toml"),
            "presets_dir = \".\"\n[env]\nHTTP_PROXY_PORT = \"1111\"\n",
        )
        .await
        .unwrap();
        fs::write(
            project_dir.join(".env.toml"),
            "HTTP_PROXY_PORT = \"3333\"\n",
        )
        .await
        .unwrap();

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", state_dir.to_str().unwrap()) };
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_PRESETS") };
        std::env::set_current_dir(&child_dir).unwrap();

        let config = Config::load_or_init().await.unwrap();

        assert_eq!(
            fs::canonicalize(&config.config_path).await.unwrap(),
            fs::canonicalize(project_dir.join("config.toml"))
                .await
                .unwrap()
        );
        assert_eq!(
            fs::canonicalize(config.presets_dir()).await.unwrap(),
            fs::canonicalize(&project_dir).await.unwrap()
        );
        assert_eq!(
            config.env.get("HTTP_PROXY_PORT").map(String::as_str),
            Some("3333")
        );

        restore_current_dir(&original_dir);
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        fs::remove_dir_all(&project_dir).await.unwrap();
        fs::remove_dir_all(&state_dir).await.unwrap();
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn init_current_dir_config_writes_presets_dir_and_refuses_existing_file() {
        let _guard = env_lock();
        let original_dir = std::env::current_dir().unwrap();
        let project_dir = make_temp_dir().await;
        std::env::set_current_dir(&project_dir).unwrap();

        let path = Config::init_current_dir_config().await.unwrap();
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("shine.config.toml")
        );
        assert_eq!(
            fs::canonicalize(path.parent().unwrap()).await.unwrap(),
            fs::canonicalize(&project_dir).await.unwrap()
        );

        let content = fs::read_to_string(&path).await.unwrap();
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert_eq!(
            parsed.get("presets_dir").and_then(|value| value.as_str()),
            Some(".")
        );
        assert!(
            !parsed.contains_key("schema_version"),
            "project config must not persist runtime schema_version"
        );
        assert!(
            !parsed.contains_key("last_cleared_schema_version"),
            "project config must not persist runtime clear state"
        );

        let err = Config::init_current_dir_config().await.unwrap_err();
        assert!(
            err.to_string().contains("already exists"),
            "error should refuse overwrite: {err:#}"
        );

        restore_current_dir(&original_dir);
        fs::remove_dir_all(&project_dir).await.unwrap();
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn project_config_save_removes_runtime_schema_fields() {
        let _guard = env_lock();
        let original_dir = std::env::current_dir().unwrap();
        let project_dir = make_temp_dir().await;
        let state_dir = make_temp_dir().await;
        fs::write(
            project_dir.join("shine.config.toml"),
            "schema_version = 0\nlast_cleared_schema_version = 0\npresets_dir = \".\"\n",
        )
        .await
        .unwrap();

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", state_dir.to_str().unwrap()) };
        std::env::set_current_dir(&project_dir).unwrap();

        let config = Config::load_or_init().await.unwrap();
        assert_eq!(config.schema_version, CURRENT_RUNTIME_SCHEMA_VERSION);
        assert_eq!(config.last_cleared_schema_version, None);

        config.save().await.unwrap();

        let content = fs::read_to_string(project_dir.join("shine.config.toml"))
            .await
            .unwrap();
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert_eq!(
            parsed.get("presets_dir").and_then(|value| value.as_str()),
            Some(".")
        );
        assert!(!parsed.contains_key("schema_version"));
        assert!(!parsed.contains_key("last_cleared_schema_version"));

        restore_current_dir(&original_dir);
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        fs::remove_dir_all(&project_dir).await.unwrap();
        fs::remove_dir_all(&state_dir).await.unwrap();
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn project_config_without_presets_dir_inherits_global_config() {
        let _guard = env_lock();
        let original_dir = std::env::current_dir().unwrap();
        let original_home = std::env::var("HOME").ok();
        let project_dir = make_temp_dir().await;
        let home_dir = make_temp_dir().await;
        let state_dir = home_dir.join(".shine");
        fs::create_dir_all(&state_dir).await.unwrap();
        let global_presets = state_dir.join("shared-presets");
        fs::create_dir_all(&global_presets).await.unwrap();
        fs::write(
            state_dir.join("config.toml"),
            "presets_dir = \"shared-presets\"\ngpg_key_id = \"global-key\"\n",
        )
        .await
        .unwrap();
        fs::write(
            project_dir.join("shine.config.toml"),
            "[env]\nLOCAL = \"yes\"\n",
        )
        .await
        .unwrap();

        unsafe { std::env::set_var("HOME", home_dir.to_str().unwrap()) };
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        unsafe { std::env::remove_var("SHINE_PRESETS") };
        std::env::set_current_dir(&project_dir).unwrap();

        let config = Config::load_or_init().await.unwrap();
        assert_eq!(
            fs::canonicalize(config.presets_dir()).await.unwrap(),
            fs::canonicalize(&global_presets).await.unwrap()
        );
        assert_eq!(config.gpg_key_id.as_deref(), Some("global-key"));
        assert!(config.is_external_presets);

        restore_current_dir(&original_dir);
        match original_home {
            Some(home) => unsafe { std::env::set_var("HOME", home) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        fs::remove_dir_all(&project_dir).await.unwrap();
        fs::remove_dir_all(&home_dir).await.unwrap();
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn project_config_merges_layers_and_saves_only_local_changes() {
        let _guard = env_lock();
        let original_dir = std::env::current_dir().unwrap();
        let project_dir = make_temp_dir().await;
        let state_dir = make_temp_dir().await;
        fs::create_dir_all(project_dir.join("project-presets"))
            .await
            .unwrap();
        fs::write(
            state_dir.join("config.toml"),
            "presets_dir = \"global-presets\"\ngpg_key_id = \"global-key\"\n[env]\nGLOBAL = \"config\"\nSHARED = \"global-config\"\n",
        )
        .await
        .unwrap();
        fs::write(
            state_dir.join("shine.env.toml"),
            "SHARED = \"global-env\"\nGLOBAL_FILE = \"yes\"\n",
        )
        .await
        .unwrap();
        fs::write(
            project_dir.join("shine.config.toml"),
            "presets_dir = \"project-presets\"\n[env]\nPROJECT = \"config\"\nSHARED = \"project-config\"\n",
        )
        .await
        .unwrap();
        fs::write(
            project_dir.join("shine.env.toml"),
            "SHARED = \"project-env\"\nPROJECT_FILE = \"yes\"\n",
        )
        .await
        .unwrap();

        unsafe { std::env::set_var("SHINE_CONFIG_DIR", state_dir.to_str().unwrap()) };
        unsafe { std::env::remove_var("SHINE_PRESETS") };
        std::env::set_current_dir(&project_dir).unwrap();

        let mut config = Config::load_or_init().await.unwrap();
        assert_eq!(
            fs::canonicalize(config.presets_dir()).await.unwrap(),
            fs::canonicalize(project_dir.join("project-presets"))
                .await
                .unwrap()
        );
        assert_eq!(config.env.get("GLOBAL").map(String::as_str), Some("config"));
        assert_eq!(
            config.env.get("PROJECT").map(String::as_str),
            Some("config")
        );
        assert_eq!(
            config.env.get("GLOBAL_FILE").map(String::as_str),
            Some("yes")
        );
        assert_eq!(
            config.env.get("PROJECT_FILE").map(String::as_str),
            Some("yes")
        );
        assert_eq!(
            config.env.get("SHARED").map(String::as_str),
            Some("project-env")
        );

        config.env.insert("ADDED".into(), "new".into());
        config.save().await.unwrap();
        let saved = fs::read_to_string(project_dir.join("shine.config.toml"))
            .await
            .unwrap();
        let table: toml::Table = toml::from_str(&saved).unwrap();
        assert_eq!(
            table.get("presets_dir").and_then(toml::Value::as_str),
            Some("project-presets")
        );
        assert!(!table.contains_key("gpg_key_id"));
        let env = table.get("env").and_then(toml::Value::as_table).unwrap();
        assert_eq!(env.get("ADDED").and_then(toml::Value::as_str), Some("new"));
        assert!(!env.contains_key("GLOBAL"));
        assert!(!env.contains_key("GLOBAL_FILE"));
        assert!(!env.contains_key("PROJECT_FILE"));

        restore_current_dir(&original_dir);
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        fs::remove_dir_all(&project_dir).await.unwrap();
        fs::remove_dir_all(&state_dir).await.unwrap();
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn config_env_wins_over_legacy_env_file() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        fs::write(
            dir.join("config.toml"),
            "[env]\nHTTP_PROXY_PORT = \"1111\"\n",
        )
        .await
        .unwrap();
        fs::write(
            dir.join("env.toml"),
            "HTTP_PROXY_PORT = \"2222\"\nCUSTOM_TOKEN = \"abc\"\n",
        )
        .await
        .unwrap();
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", dir.to_str().unwrap()) };

        let config = Config::load_or_init().await.unwrap();

        assert_eq!(
            config.env.get("HTTP_PROXY_PORT").map(String::as_str),
            Some("1111")
        );
        assert_eq!(
            config.env.get("CUSTOM_TOKEN").map(String::as_str),
            Some("abc")
        );
        assert!(
            !dir.join("env.toml").exists(),
            "legacy env.toml should be removed"
        );

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    // --- resolve_runtime_config_dirs unit tests ---

    #[test]
    fn shine_config_dir_overrides_everything() {
        let _guard = env_lock();
        let default_shine = PathBuf::from("/home/user/.shine");
        let default_presets = PathBuf::from("/home/user/.shine/presets");
        let custom = std::env::temp_dir().join("shine-override-test");

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", custom.to_str().unwrap()) };
        let (shine, presets, _) =
            resolve_runtime_config_dirs(&default_shine, &default_presets, None, false);
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };

        assert_eq!(shine, custom);
        assert_eq!(presets, custom.join("presets"));
    }

    #[test]
    fn shine_presets_overrides_presets_only() {
        let _guard = env_lock();
        let default_shine = PathBuf::from("/home/user/.shine");
        let default_presets = PathBuf::from("/home/user/.shine/presets");
        let custom_presets = std::env::temp_dir().join("my-presets");

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_PRESETS", custom_presets.to_str().unwrap()) };
        let (shine, presets, _) =
            resolve_runtime_config_dirs(&default_shine, &default_presets, None, false);
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_PRESETS") };

        assert_eq!(shine, default_shine);
        assert_eq!(presets, custom_presets);
    }

    #[test]
    fn shine_config_dir_takes_precedence_over_shine_presets() {
        let _guard = env_lock();
        let default_shine = PathBuf::from("/home/user/.shine");
        let default_presets = PathBuf::from("/home/user/.shine/presets");
        let custom_dir = std::env::temp_dir().join("shine-cfg-dir");
        let custom_presets = std::env::temp_dir().join("shine-presets-ignored");

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", custom_dir.to_str().unwrap()) };
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_PRESETS", custom_presets.to_str().unwrap()) };
        let (shine, presets, _) =
            resolve_runtime_config_dirs(&default_shine, &default_presets, None, false);
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_PRESETS") };

        assert_eq!(shine, custom_dir);
        assert_eq!(presets, custom_dir.join("presets"));
    }

    #[test]
    fn local_config_keeps_shine_config_dir_as_state_only() {
        let _guard = env_lock();
        let default_shine = PathBuf::from("/home/user/.shine");
        let default_presets = PathBuf::from("/home/user/.shine/presets");
        let custom_dir = std::env::temp_dir().join("shine-local-state");
        let toml_presets = PathBuf::from("/project/presets");

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", custom_dir.to_str().unwrap()) };
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_PRESETS") };
        let (shine, presets, _) = resolve_runtime_config_dirs(
            &default_shine,
            &default_presets,
            Some(toml_presets.as_path()),
            true,
        );
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };

        assert_eq!(shine, custom_dir);
        assert_eq!(presets, toml_presets);
    }

    #[test]
    fn config_toml_presets_dir_is_used_when_no_env() {
        let _guard = env_lock();
        let default_shine = PathBuf::from("/home/user/.shine");
        let default_presets = PathBuf::from("/home/user/.shine/presets");
        let toml_presets = PathBuf::from("/custom/presets");

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_PRESETS") };
        let (shine, presets, _) = resolve_runtime_config_dirs(
            &default_shine,
            &default_presets,
            Some(toml_presets.as_path()),
            false,
        );

        assert_eq!(shine, default_shine);
        assert_eq!(presets, toml_presets);
    }

    #[test]
    fn shine_presets_takes_precedence_over_config_toml() {
        let _guard = env_lock();
        let default_shine = PathBuf::from("/home/user/.shine");
        let default_presets = PathBuf::from("/home/user/.shine/presets");
        let env_presets = std::env::temp_dir().join("env-presets");
        let toml_presets = PathBuf::from("/toml/presets");

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_PRESETS", env_presets.to_str().unwrap()) };
        let (_, presets, _) = resolve_runtime_config_dirs(
            &default_shine,
            &default_presets,
            Some(toml_presets.as_path()),
            false,
        );
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_PRESETS") };

        assert_eq!(presets, env_presets);
    }

    #[tokio::test]
    async fn read_presets_override_returns_none_when_file_missing() {
        let missing =
            std::env::temp_dir().join(format!("shine-no-config-{}.toml", uuid::Uuid::new_v4()));
        let result = read_presets_override_from_toml(&missing).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn read_presets_override_returns_none_when_key_absent() {
        let dir = make_temp_dir().await;
        let path = dir.join("config.toml");
        fs::write(&path, "schema_version = 0\n").await.unwrap();

        let result = read_presets_override_from_toml(&path).await;
        assert!(result.is_none());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn read_presets_override_returns_path_when_key_present() {
        let dir = make_temp_dir().await;
        let path = dir.join("config.toml");
        fs::write(&path, "schema_version = 0\npresets_dir = \"/my/presets\"\n")
            .await
            .unwrap();

        let result = read_presets_override_from_toml(&path).await;
        assert_eq!(result, Some(PathBuf::from("/my/presets")));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn presets_dir_override_round_trips_through_save() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        config.presets_dir_override = Some(PathBuf::from("/external/presets"));

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        assert!(
            content.contains("/external/presets"),
            "presets_dir should be written to config.toml"
        );

        let loaded: Config = toml::from_str(&content).unwrap();
        assert_eq!(
            loaded.presets_dir_override,
            Some(PathBuf::from("/external/presets"))
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn presets_overlay_dir_override_round_trips_through_save() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        config.presets_overlay_dir_override = Some(PathBuf::from("/external/overlay"));

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        assert!(content.contains("presets_overlay_dir"));

        let loaded: Config = toml::from_str(&content).unwrap();
        assert_eq!(
            loaded.presets_overlay_dir_override,
            Some(PathBuf::from("/external/overlay"))
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn gpg_key_id_round_trips_through_save() {
        let dir = make_temp_dir().await;
        let mut config = config_in(&dir);
        config.gpg_key_id = Some("alice@example.com".to_string());

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        let loaded: Config = toml::from_str(&content).unwrap();
        assert_eq!(loaded.gpg_key_id.as_deref(), Some("alice@example.com"));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn presets_dir_absent_from_toml_when_override_is_none() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir); // presets_dir_override: None

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert!(
            !parsed.contains_key("presets_dir"),
            "presets_dir key must be absent when override is None"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    // --- is_external_presets flag tests ---

    #[test]
    fn is_external_presets_true_when_shine_config_dir_set() {
        let _guard = env_lock();
        let default = PathBuf::from("/home/user/.shine");
        let presets = PathBuf::from("/home/user/.shine/presets");
        let custom = std::env::temp_dir().join("shine-ext-test");

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", custom.to_str().unwrap()) };
        let (_, _, is_external) = resolve_runtime_config_dirs(&default, &presets, None, false);
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };

        assert!(
            is_external,
            "SHINE_CONFIG_DIR should set is_external_presets"
        );
    }

    #[test]
    fn is_external_presets_true_when_shine_presets_set() {
        let _guard = env_lock();
        let default = PathBuf::from("/home/user/.shine");
        let presets = PathBuf::from("/home/user/.shine/presets");
        let custom = std::env::temp_dir().join("shine-ext-presets");

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_PRESETS", custom.to_str().unwrap()) };
        let (_, _, is_external) = resolve_runtime_config_dirs(&default, &presets, None, false);
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_PRESETS") };

        assert!(is_external, "SHINE_PRESETS should set is_external_presets");
    }

    #[test]
    fn is_external_presets_true_when_toml_presets_dir_set() {
        let _guard = env_lock();
        let default = PathBuf::from("/home/user/.shine");
        let presets = PathBuf::from("/home/user/.shine/presets");
        let toml_override = PathBuf::from("/toml/presets");

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_PRESETS") };
        let (_, _, is_external) =
            resolve_runtime_config_dirs(&default, &presets, Some(toml_override.as_path()), false);

        assert!(
            is_external,
            "config.toml presets_dir should set is_external_presets"
        );
    }

    #[test]
    fn is_external_presets_false_when_no_override() {
        let _guard = env_lock();
        let default = PathBuf::from("/home/user/.shine");
        let presets = PathBuf::from("/home/user/.shine/presets");

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_PRESETS") };
        let (_, _, is_external) = resolve_runtime_config_dirs(&default, &presets, None, false);

        assert!(
            !is_external,
            "no override should leave is_external_presets false"
        );
    }
}
