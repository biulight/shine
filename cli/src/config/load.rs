//! Config loading: global runtime config, project-layer discovery and merge.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use tokio::fs;

use super::discovery::{
    find_project_config, preliminary_shine_dir_from_env, read_minimal_config,
    read_presets_override_from_toml, resolve_config_presets_path, resolve_runtime_config_dirs,
};
use super::env_layer::{deserialize_env_values, parse_env_descriptions};
use super::{Config, ExternalShellMode, GLOBAL_CONFIG_FILE, PROJECT_CONFIG_FILE, ProjectSaveState};
use crate::home::{default_config_and_presets_dir, effective_home_dir};

#[derive(Default, Deserialize)]
struct ProjectOverrides {
    #[serde(default)]
    presets_dir: Option<PathBuf>,
    #[serde(default)]
    external_shell_mode: Option<ExternalShellMode>,
    #[serde(default)]
    presets_overlay_dir: Option<PathBuf>,
    #[serde(default)]
    app_default_dest_root: Option<PathBuf>,
    #[serde(default)]
    allow_app_hooks: Option<bool>,
    #[serde(default)]
    self_install_dest: Option<PathBuf>,
    #[serde(default)]
    gpg_key_id: Option<String>,
    #[serde(default)]
    secret_backend: Option<String>,
    #[serde(default)]
    age_recipients: Vec<String>,
    #[serde(default)]
    age_identity: Option<String>,
    #[serde(default, deserialize_with = "deserialize_env_values")]
    env: BTreeMap<String, String>,
}

impl Config {
    pub async fn init_current_dir_config() -> Result<PathBuf> {
        let current_dir = std::env::current_dir().context("resolving current directory")?;
        let (default_shine_dir, _) = default_config_and_presets_dir()?;
        let preliminary_shine_dir = preliminary_shine_dir_from_env(&default_shine_dir);
        if let Some(project_config) = find_project_config(&current_dir) {
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
        let project_config = find_project_config(&current_dir);
        let Some(project_config) = project_config else {
            return Self::load_global_runtime_or_init().await;
        };
        // Initialize the global layer before applying the sparse project layer.
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
        config.ensure_env_defaults(global_has_env).await?;

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
        if let Some(mode) = overrides.external_shell_mode {
            config.external_shell_mode = mode;
        }
        if let Some(path) = overrides.app_default_dest_root {
            config.app_default_dest_root_override =
                Some(resolve_config_presets_path(&path, &project_config.root));
        }
        if let Some(value) = overrides.allow_app_hooks {
            config.allow_app_hooks = value;
        }
        if let Some(path) = overrides.self_install_dest {
            config.self_install_dest =
                Some(resolve_config_presets_path(&path, &project_config.root));
        }
        if overrides.gpg_key_id.is_some() {
            config.gpg_key_id = overrides.gpg_key_id;
        }
        if overrides.secret_backend.is_some() {
            config.secret_backend = overrides.secret_backend;
        }
        if !overrides.age_recipients.is_empty() {
            config.age_recipients = overrides.age_recipients;
        }
        if overrides.age_identity.is_some() {
            config.age_identity = overrides.age_identity;
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
        config.apply_overlay_env_override().await?;
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
        config.ensure_env_defaults(config_has_env).await?;
        config.apply_global_env_override().await?;
        config.apply_overlay_env_override().await?;
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
            config.resolve_managed_overlay_dir();
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
                return Ok(super::CURRENT_RUNTIME_SCHEMA_VERSION);
            }
            Err(e) => {
                return Err(e).with_context(|| format!("Failed to read {}", config_path.display()));
            }
        };

        read_minimal_config(&content)
            .map(|config| config.schema_version)
            .with_context(|| format!("Failed to parse {}", config_path.display()))
    }
}

fn config_toml_has_env_table(contents: &str) -> bool {
    toml::from_str::<toml::Table>(contents)
        .map(|table| table.contains_key("env"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{make_temp_dir, restore_current_dir};
    use super::*;
    use crate::config::CURRENT_RUNTIME_SCHEMA_VERSION;
    use crate::test_support::env_lock;

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
    async fn load_or_init_backfills_missing_env_defaults() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        fs::write(dir.join("config.toml"), "[env]\nCUSTOM = \"kept\"\n")
            .await
            .unwrap();

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", dir.to_str().unwrap()) };

        let config = Config::load_or_init().await.unwrap();

        assert_eq!(config.env.get("CUSTOM").map(String::as_str), Some("kept"));
        assert_eq!(
            config.env.get("HTTP_PROXY_PORT").map(String::as_str),
            Some("6152")
        );
        assert_eq!(
            config.env.get("SOCKS5_PROXY_PORT").map(String::as_str),
            Some("6153")
        );
        let content = fs::read_to_string(dir.join("config.toml")).await.unwrap();
        assert!(content.contains("CUSTOM = \"kept\""));
        assert!(content.contains("HTTP_PROXY_PORT = \"6152\""));
        assert!(content.contains("SOCKS5_PROXY_PORT = \"6153\""));

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
    async fn load_global_runtime_for_dry_run_ignores_removed_global_env_file() {
        let _guard = env_lock();
        let dir = make_temp_dir().await;
        let removed_path = dir.join("env.toml");
        fs::write(&removed_path, "CUSTOM_TOKEN = \"abc\"\n")
            .await
            .unwrap();

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", dir.to_str().unwrap()) };

        let config = Config::load_global_runtime_for_dry_run().await.unwrap();

        assert_eq!(config.config_path(), dir.join("config.toml"));
        assert!(removed_path.exists());
        assert!(!dir.join("config.toml").exists());

        // SAFETY: env_lock() is held for the duration of this block, preventing
        //          concurrent env mutation from other threads in this test binary.
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test(flavor = "current_thread")]
    async fn global_config_with_presets_dir_remains_global_config() {
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
            "global config.toml must not be treated as a project config"
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
    async fn load_or_init_ignores_generic_project_config_and_dotenv() {
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
            fs::canonicalize(state_dir.join("config.toml"))
                .await
                .unwrap()
        );
        assert!(!config.is_project_config);
        assert_eq!(
            fs::canonicalize(config.presets_dir()).await.unwrap(),
            fs::canonicalize(state_dir.join("presets")).await.unwrap()
        );
        assert_eq!(
            config.env.get("HTTP_PROXY_PORT").map(String::as_str),
            Some("6152")
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
    async fn init_current_dir_config_ignores_generic_config_and_refuses_existing_shine_config() {
        let _guard = env_lock();
        let original_dir = std::env::current_dir().unwrap();
        let project_dir = make_temp_dir().await;
        fs::write(
            project_dir.join("config.toml"),
            "presets_dir = \"other-tool\"\n",
        )
        .await
        .unwrap();
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
    async fn project_config_overrides_age_backend_settings() {
        let _guard = env_lock();
        let original_dir = std::env::current_dir().unwrap();
        let project_dir = make_temp_dir().await;
        let state_dir = make_temp_dir().await;
        fs::write(
            state_dir.join("config.toml"),
            "secret_backend = \"gpg\"\nage_recipients = [\"age1global\"]\nage_identity = \"~/.shine/age/global.txt\"\n",
        )
        .await
        .unwrap();
        fs::write(
            project_dir.join("shine.config.toml"),
            "presets_dir = \".\"\nsecret_backend = \"age\"\nage_recipients = [\"age1project-a\", \"age1project-b\"]\n",
        )
        .await
        .unwrap();

        unsafe { std::env::set_var("SHINE_CONFIG_DIR", state_dir.to_str().unwrap()) };
        unsafe { std::env::remove_var("SHINE_PRESETS") };
        std::env::set_current_dir(&project_dir).unwrap();

        let config = Config::load_or_init().await.unwrap();

        assert_eq!(config.secret_backend.as_deref(), Some("age"));
        assert_eq!(
            config.age_recipients,
            vec!["age1project-a".to_string(), "age1project-b".to_string()]
        );
        // age_identity is absent from the project override, so the global value persists.
        assert_eq!(
            config.age_identity.as_deref(),
            Some("~/.shine/age/global.txt")
        );

        restore_current_dir(&original_dir);
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        fs::remove_dir_all(&project_dir).await.unwrap();
        fs::remove_dir_all(&state_dir).await.unwrap();
    }
}
