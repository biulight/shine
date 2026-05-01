use crate::shells::ShellType;
use anyhow::{Context, Result, bail};
use directories::UserDirs;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(crate) struct Config {
    /// Presets directory - computed at runtime, not serialized
    #[serde(skip)]
    presets_dir: PathBuf,
    /// Bin directory for symlinks - computed from home
    #[serde(skip)]
    bin_dir: PathBuf,
    /// Path to config.toml - computed from home
    #[serde(skip)]
    config_path: PathBuf,
    #[serde(skip)]
    pub home_dir: PathBuf,
    /// Filename
    #[serde(skip)]
    #[allow(dead_code)]
    file_name: String,
    #[serde(default)]
    pub schema_version: u32,
    #[serde(skip)]
    pub shell_type: ShellType,
    /// Optional persistent presets_dir override stored in config.toml.
    /// Takes effect when neither SHINE_CONFIG_DIR nor SHINE_PRESETS is set.
    #[serde(
        rename = "presets_dir",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub presets_dir_override: Option<PathBuf>,
    /// Optional override for the default destination root used by `shine app install`
    /// when a preset file carries no `shine-dest:` annotation.
    /// Defaults to `~/.config` when not set.
    #[serde(
        rename = "app_default_dest_root",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub app_default_dest_root_override: Option<PathBuf>,
    /// `true` when the presets directory is provided by the user (via env var or config.toml
    /// `presets_dir` key) rather than the default `~/.shine/presets/`.
    /// When `true`, install commands read presets from disk directly without extracting
    /// embedded assets, and list commands enumerate the on-disk folder.
    #[serde(skip)]
    pub is_external_presets: bool,
    /// Path where `shine self install` last copied the binary.
    /// When set, `shine upgrade` will try to sync the new binary there automatically.
    #[serde(
        rename = "self_install_dest",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub self_install_dest: Option<PathBuf>,
}

impl Config {
    pub(crate) async fn load_or_init() -> Result<Self> {
        let home_dir = effective_home_dir();
        let (default_shine_dir, default_presets_dir) = default_config_and_presets_dir()?;

        // Pre-read config.toml (from the expected shine_dir) to extract an optional
        // presets_dir override before the full resolution pass.
        let preliminary_shine_dir = preliminary_shine_dir_from_env(&default_shine_dir);
        let toml_presets =
            read_presets_override_from_toml(&preliminary_shine_dir.join("config.toml")).await;

        let (shine_dir, presets_dir, is_external_presets) = resolve_runtime_config_dirs(
            &default_shine_dir,
            &default_presets_dir,
            toml_presets.as_deref(),
        );

        let bin_dir = shine_dir.join("bin");
        let config_path = shine_dir.join("config.toml");

        fs::create_dir_all(&shine_dir)
            .await
            .with_context(|| "creating shine config dir")?;
        fs::create_dir_all(&presets_dir)
            .await
            .with_context(|| "creating presets dir")?;
        fs::create_dir_all(&bin_dir)
            .await
            .with_context(|| "creating bin dir")?;

        if config_path.exists() {
            let contents = fs::read_to_string(&config_path)
                .await
                .context("Failed to read config file")?;

            let mut config: Config =
                toml::from_str(&contents).context("Failed to parse config file")?;
            config.config_path = config_path.clone();
            config.presets_dir = presets_dir;
            config.bin_dir = bin_dir;
            config.home_dir = home_dir;
            config.is_external_presets = is_external_presets;
            Ok(config)
        } else {
            let config = Config {
                config_path: config_path.clone(),
                presets_dir,
                bin_dir,
                home_dir,
                is_external_presets,
                ..Config::default()
            };
            config.save().await?;
            Ok(config)
        }
    }

    pub(crate) fn presets_dir(&self) -> &Path {
        &self.presets_dir
    }

    pub(crate) fn bin_dir(&self) -> &Path {
        &self.bin_dir
    }

    pub(crate) fn shine_dir(&self) -> &Path {
        self.config_path
            .parent()
            .expect("config_path is always under the shine config directory")
    }

    pub(crate) fn app_default_dest_root(&self) -> PathBuf {
        match &self.app_default_dest_root_override {
            Some(p) => {
                let s = p.to_str().unwrap_or("~/.config");
                PathBuf::from(tilde_expand(s))
            }
            None => self.home_dir.join(".config"),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_test(dir: &Path) -> Self {
        Self {
            config_path: dir.join("config.toml"),
            presets_dir: dir.join("presets"),
            bin_dir: dir.join("bin"),
            home_dir: dir.to_path_buf(),
            file_name: "config.toml".to_string(),
            schema_version: 0,
            shell_type: ShellType::default(),
            presets_dir_override: None,
            app_default_dest_root_override: None,
            is_external_presets: false,
            self_install_dest: None,
        }
    }

    /// Return a clone of this config with `presets_dir_override` replaced.
    pub(crate) fn with_presets_dir_override(self, value: Option<PathBuf>) -> Self {
        Self {
            presets_dir_override: value,
            ..self
        }
    }

    #[allow(dead_code)]
    pub(crate) fn validate(&self) -> Result<()> {
        todo!("Validate config")
    }

    pub(crate) async fn save(&self) -> Result<()> {
        let config_to_save = self.clone();
        let config_path = self.resolve_config_path_for_save().await?;

        let shine_dir = config_path
            .parent()
            .context("Config path must have a parent directory")?;

        let new_toml =
            toml::to_string_pretty(&config_to_save).context("Failed to serialize config")?;

        let toml_str = if config_path.exists() {
            let existing = fs::read_to_string(&config_path).await.unwrap_or_default();
            if existing.is_empty() {
                new_toml
            } else {
                let new_table: toml::Table =
                    toml::from_str(&new_toml).context("Failed to round-trip serialize config")?;
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
        let _backup_path = shine_dir.join(format!("{file_name}.bak"));

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
}

/// Print a note showing the active external presets directory.
/// No-op when the embedded presets are in use.
pub(crate) fn print_presets_note(config: &Config) {
    if config.is_external_presets {
        println!(
            "{}",
            crate::colors::external_presets_note(config.presets_dir())
        );
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
            home_dir,
            file_name: "config.toml".to_string(),
            schema_version: 0,
            shell_type: ShellType::default(),
            presets_dir_override: None,
            app_default_dest_root_override: None,
            is_external_presets: false,
            self_install_dest: None,
        }
    }
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
pub(crate) fn tilde_expand(s: &str) -> String {
    let home = effective_home_dir().to_string_lossy().into_owned();
    shellexpand::tilde_with_context(s, || Some(home)).into_owned()
}

/// Like `shellexpand::full` but uses the effective home for both `~` and `$HOME`.
pub(crate) fn full_expand(s: &str) -> Result<String, shellexpand::LookupError<std::env::VarError>> {
    let home = effective_home_dir().to_string_lossy().into_owned();
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

/// Return the shine root dir implied by `SHINE_CONFIG_DIR`, or `default` if unset.
/// Used for a preliminary read of config.toml before full resolution.
fn preliminary_shine_dir_from_env(default: &Path) -> PathBuf {
    if let Ok(val) = std::env::var("SHINE_CONFIG_DIR") {
        let val = val.trim().to_string();
        if !val.is_empty() {
            return PathBuf::from(tilde_expand(&val));
        }
    }
    default.to_owned()
}

/// Attempt to read the `presets_dir` key from an existing config.toml without
/// doing a full parse. Returns `None` if the file is absent, unreadable, or the
/// key is not set.
async fn read_presets_override_from_toml(config_path: &Path) -> Option<PathBuf> {
    let content = tokio::fs::read_to_string(config_path).await.ok()?;
    #[derive(Deserialize)]
    struct MinimalConfig {
        #[serde(default)]
        presets_dir: Option<PathBuf>,
    }
    let partial: MinimalConfig = toml::from_str(&content).ok()?;
    partial.presets_dir
}

/// Resolve the runtime (shine_dir, presets_dir) pair.
///
/// Priority (highest first):
///   1. `SHINE_CONFIG_DIR` — overrides both shine_dir and presets_dir
///   2. `SHINE_PRESETS`    — overrides presets_dir only
///   3. `config_toml_presets` — presets_dir from config.toml `presets_dir` key
///   4. defaults
///
/// Returns `(shine_dir, presets_dir, is_external_presets)`.
fn resolve_runtime_config_dirs(
    default_shine_dir: &Path,
    default_presets_dir: &Path,
    config_toml_presets: Option<&Path>,
) -> (PathBuf, PathBuf, bool) {
    if let Ok(val) = std::env::var("SHINE_CONFIG_DIR") {
        let val = val.trim().to_string();
        if !val.is_empty() {
            let dir = PathBuf::from(tilde_expand(&val));
            return (dir.clone(), dir.join("presets"), true);
        }
    }

    if let Ok(val) = std::env::var("SHINE_PRESETS") {
        let val = val.trim().to_string();
        if !val.is_empty() {
            let presets = PathBuf::from(tilde_expand(&val));
            return (default_shine_dir.to_owned(), presets, true);
        }
    }

    if let Some(p) = config_toml_presets
        && let Some(s) = p.to_str()
    {
        let presets = PathBuf::from(tilde_expand(s));
        return (default_shine_dir.to_owned(), presets, true);
    }

    (
        default_shine_dir.to_owned(),
        default_presets_dir.to_owned(),
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("environment lock must not be poisoned")
    }

    fn config_in(dir: &Path) -> Config {
        Config {
            config_path: dir.join("config.toml"),
            presets_dir: dir.join("presets"),
            bin_dir: dir.join("bin"),
            home_dir: dir.join("home"),
            file_name: "config.toml".to_string(),
            schema_version: 0,
            shell_type: ShellType::default(),
            presets_dir_override: None,
            app_default_dest_root_override: None,
            is_external_presets: false,
            self_install_dest: None,
        }
    }

    async fn make_temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shine-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();
        dir
    }

    #[tokio::test]
    async fn save_writes_config_file_for_new_config() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);

        config.save().await.unwrap();

        let content = fs::read_to_string(&config.config_path).await.unwrap();
        let parsed: toml::Table = toml::from_str(&content).unwrap();
        assert_eq!(parsed["schema_version"].as_integer(), Some(0));

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
    async fn save_merges_preserves_comments() {
        let dir = make_temp_dir().await;
        let config = config_in(&dir);
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
            presets_dir: PathBuf::from("presets"),
            bin_dir: PathBuf::from("bin"),
            home_dir: PathBuf::from("home"),
            file_name: "config.toml".to_string(),
            schema_version: 0,
            shell_type: ShellType::default(),
            presets_dir_override: None,
            app_default_dest_root_override: None,
            is_external_presets: false,
            self_install_dest: None,
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
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", dir.to_str().unwrap()) };

        let config = Config::load_or_init().await.unwrap();
        assert!(config.bin_dir().exists(), "bin dir should be created");
        assert_eq!(config.bin_dir(), dir.join("bin"));

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

        unsafe { std::env::set_var("SHINE_CONFIG_DIR", custom.to_str().unwrap()) };
        let (shine, presets, _) =
            resolve_runtime_config_dirs(&default_shine, &default_presets, None);
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

        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        unsafe { std::env::set_var("SHINE_PRESETS", custom_presets.to_str().unwrap()) };
        let (shine, presets, _) =
            resolve_runtime_config_dirs(&default_shine, &default_presets, None);
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

        unsafe { std::env::set_var("SHINE_CONFIG_DIR", custom_dir.to_str().unwrap()) };
        unsafe { std::env::set_var("SHINE_PRESETS", custom_presets.to_str().unwrap()) };
        let (shine, presets, _) =
            resolve_runtime_config_dirs(&default_shine, &default_presets, None);
        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        unsafe { std::env::remove_var("SHINE_PRESETS") };

        assert_eq!(shine, custom_dir);
        assert_eq!(presets, custom_dir.join("presets"));
    }

    #[test]
    fn config_toml_presets_dir_is_used_when_no_env() {
        let _guard = env_lock();
        let default_shine = PathBuf::from("/home/user/.shine");
        let default_presets = PathBuf::from("/home/user/.shine/presets");
        let toml_presets = PathBuf::from("/custom/presets");

        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        unsafe { std::env::remove_var("SHINE_PRESETS") };
        let (shine, presets, _) = resolve_runtime_config_dirs(
            &default_shine,
            &default_presets,
            Some(toml_presets.as_path()),
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

        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        unsafe { std::env::set_var("SHINE_PRESETS", env_presets.to_str().unwrap()) };
        let (_, presets, _) = resolve_runtime_config_dirs(
            &default_shine,
            &default_presets,
            Some(toml_presets.as_path()),
        );
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

        unsafe { std::env::set_var("SHINE_CONFIG_DIR", custom.to_str().unwrap()) };
        let (_, _, is_external) = resolve_runtime_config_dirs(&default, &presets, None);
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

        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        unsafe { std::env::set_var("SHINE_PRESETS", custom.to_str().unwrap()) };
        let (_, _, is_external) = resolve_runtime_config_dirs(&default, &presets, None);
        unsafe { std::env::remove_var("SHINE_PRESETS") };

        assert!(is_external, "SHINE_PRESETS should set is_external_presets");
    }

    #[test]
    fn is_external_presets_true_when_toml_presets_dir_set() {
        let _guard = env_lock();
        let default = PathBuf::from("/home/user/.shine");
        let presets = PathBuf::from("/home/user/.shine/presets");
        let toml_override = PathBuf::from("/toml/presets");

        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        unsafe { std::env::remove_var("SHINE_PRESETS") };
        let (_, _, is_external) =
            resolve_runtime_config_dirs(&default, &presets, Some(toml_override.as_path()));

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

        unsafe { std::env::remove_var("SHINE_CONFIG_DIR") };
        unsafe { std::env::remove_var("SHINE_PRESETS") };
        let (_, _, is_external) = resolve_runtime_config_dirs(&default, &presets, None);

        assert!(
            !is_external,
            "no override should leave is_external_presets false"
        );
    }
}
