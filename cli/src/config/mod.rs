use crate::shells::ShellType;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod discovery;
mod env_layer;
mod load;
mod save;

use discovery::resolve_config_presets_path;
pub(crate) use discovery::{ReadOnlyRuntimePaths, discover_runtime_paths_read_only};
use env_layer::deserialize_env_values;

pub use crate::home::{full_expand, full_expand_with_home, tilde_expand};
pub use env_layer::{validate_env_override_file, write_env_override_entry};

const GLOBAL_CONFIG_FILE: &str = "config.toml";
const PROJECT_CONFIG_FILE: &str = "shine.config.toml";
const PROJECT_ENV_FILE: &str = "shine.env.toml";

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

fn default_sync_terminal_theme() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalShellMode {
    #[default]
    Snapshot,
    Live,
}

fn is_snapshot_mode(value: &ExternalShellMode) -> bool {
    *value == ExternalShellMode::Snapshot
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
    /// Deployment policy for shell commands sourced from an external presets directory.
    /// Snapshot mode is the safe default; live mode is an explicit preset-development opt-in.
    #[serde(default, skip_serializing_if = "is_snapshot_mode")]
    pub external_shell_mode: ExternalShellMode,
    /// Optional overlay directory merged over the active presets source.
    #[serde(
        rename = "presets_overlay_dir",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub presets_overlay_dir_override: Option<PathBuf>,
    /// Optional Git URL for a shine-managed overlay. When set (and no explicit
    /// `presets_overlay_dir` is configured), shine owns the overlay checkout at
    /// `<shine_dir>/overlay`, cloning it `--depth 1` on `shine preset pull` and keeping
    /// it as an always-latest mirror of the remote tip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presets_overlay_git: Option<String>,
    /// Optional branch to track for `presets_overlay_git`. When unset, the
    /// remote's default branch is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presets_overlay_git_branch: Option<String>,
    /// Resolved `<shine_dir>/overlay` path when `presets_overlay_git` is set.
    /// Computed at load time, never serialized. `None` when no Git overlay is
    /// configured.
    #[serde(skip)]
    managed_overlay_dir: Option<PathBuf>,
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
    /// When `true`, commands resolve desired presets from disk without extracting embedded
    /// assets. Shell deployment then follows `external_shell_mode`.
    #[serde(skip)]
    pub is_external_presets: bool,
    /// Allows app presets loaded from external preset directories to run post-upgrade hooks.
    /// Embedded presets may run hooks without this opt-in.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_app_hooks: bool,
    /// Whether the managed sys `pre` profile auto-syncs the terminal theme
    /// (`shine theme sync --auto`) on interactive shell startup. Defaults to
    /// `true`. The `SHINE_SYNC_TERMINAL_THEME` env var overrides this at
    /// runtime regardless of value (docs/terminal-theme-sync-prd.md §5).
    /// Deliberately not project-overridable: this is a terminal/session-level
    /// toggle, not something that varies per project.
    #[serde(
        default = "default_sync_terminal_theme",
        skip_serializing_if = "is_true"
    )]
    pub sync_terminal_theme: bool,
    /// Path where `shine self install` last copied the binary.
    /// When set, `shine self upgrade` will try to sync the new binary there automatically.
    #[serde(
        rename = "self_install_dest",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub self_install_dest: Option<PathBuf>,
    /// Default GPG recipient key used by `shine env secret encrypt` when the command
    /// does not provide `-r/--recipient`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpg_key_id: Option<String>,
    /// Selects the default [`crate::secret::BackendKind`] used by `shine env
    /// encrypt`/`seal` when neither `-r/--recipient` nor a workspace backend
    /// override is given. Absent means GPG. Decryption never consults this
    /// field — it is resolved purely from the ciphertext's backend tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_backend: Option<String>,
    /// Default age recipients (`age1...` / `age1se1...`) used by `shine env
    /// encrypt`/`seal` when the age backend is active and no `-r/--recipient`
    /// is given. Encrypting to every team member's recipient lets any of them
    /// decrypt the resulting ciphertext with their own identity.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub age_recipients: Vec<String>,
    /// Path to the age identity file used to decrypt `age:`-tagged secrets.
    /// May contain multiple identities (e.g. a Secure Enclave identity plus a
    /// plain fallback), one per line. Defaults to
    /// `<shine_dir>/age/identity.txt` when unset and that file exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_identity: Option<String>,
    /// Environment variables substituted into template-enabled presets.
    #[serde(
        default = "default_env_map",
        deserialize_with = "deserialize_env_values"
    )]
    pub env: BTreeMap<String, String>,
    /// Per-variable descriptions read from detailed `[env]` entries.
    #[serde(skip)]
    pub env_descriptions: BTreeMap<String, String>,
    /// Which env override file (if any) currently supplies each key's effective
    /// value. Only override files populate this — `config.toml [env]` layers
    /// never do, since a plain write there is always effective unless shadowed
    /// by one of these. Used to detect when `env set`/`encrypt`/`delete` would
    /// otherwise silently write a value that an override file keeps shadowing.
    #[serde(skip)]
    pub env_override_sources: BTreeMap<String, EnvOverrideSource>,
}

/// Identifies the override file (global/overlay/project `shine.env.toml`) that
/// currently supplies a given env key's effective value, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvOverrideSource {
    pub path: PathBuf,
    /// Which override layer `path` belongs to. Drives the source labels in
    /// `shine env list`; `is_managed_overlay` further distinguishes the two
    /// `Overlay` variants.
    pub kind: EnvOverrideKind,
    /// `true` when `path` is inside the shine-managed Git overlay checkout
    /// (force-mirrored, read-only per ADR 0010) rather than the global/project
    /// override file or a manual overlay directory.
    pub is_managed_overlay: bool,
}

/// Which override-file layer supplied an env key's effective value. Ordered
/// low-to-high by precedence (a later layer shadows an earlier one), matching
/// the apply order in `Config::load_or_init`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvOverrideKind {
    /// Global `<shine_dir>/shine.env.toml`.
    Global,
    /// Overlay `<overlay_dir>/shine.env.toml` (managed-git or manual overlay).
    Overlay,
    /// Project `<project_root>/shine.env.toml`.
    Project,
}

#[derive(Clone, Debug)]
struct ProjectSaveState {
    original: toml::Table,
    loaded: toml::Table,
}

impl Config {
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

    /// Shine-owned snapshots of external shell categories.
    pub fn installed_shell_dir(&self) -> PathBuf {
        self.shine_dir().join("installed").join("shell")
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
            schema_version: CURRENT_RUNTIME_SCHEMA_VERSION,
            last_cleared_schema_version: None,
            shell_type: ShellType::default(),
            presets_dir_override: None,
            external_shell_mode: ExternalShellMode::Snapshot,
            presets_overlay_dir_override: None,
            presets_overlay_git: None,
            presets_overlay_git_branch: None,
            managed_overlay_dir: None,
            app_default_dest_root_override: None,
            is_external_presets: false,
            allow_app_hooks: false,
            sync_terminal_theme: default_sync_terminal_theme(),
            self_install_dest: None,
            gpg_key_id: None,
            secret_backend: None,
            age_recipients: Vec::new(),
            age_identity: None,
            env: default_env_map(),
            env_descriptions: BTreeMap::new(),
            env_override_sources: BTreeMap::new(),
        }
    }

    /// Age identity file(s) used to decrypt `age:`-tagged secrets, resolved
    /// from `age_identity` (tilde-expanded) or, when unset, the default path
    /// under `shine_dir` if it exists.
    pub fn age_identities(&self) -> Vec<PathBuf> {
        if let Some(identity) = self
            .age_identity
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return vec![PathBuf::from(tilde_expand(identity))];
        }
        let default_path = self.shine_dir.join("age").join("identity.txt");
        if default_path.is_file() {
            vec![default_path]
        } else {
            Vec::new()
        }
    }

    /// Return a clone of this config with `presets_dir_override` replaced.
    pub fn with_presets_dir_override(self, value: Option<PathBuf>) -> Self {
        Self {
            presets_dir_override: value,
            ..self
        }
    }

    pub fn with_external_shell_mode(self, value: ExternalShellMode) -> Self {
        Self {
            external_shell_mode: value,
            ..self
        }
    }

    /// Return a clone of this config with the Git-managed overlay source
    /// replaced. Setting a URL clears any manual `presets_overlay_dir` override
    /// so the two overlay modes never coexist; clearing the URL leaves the
    /// managed checkout on disk untouched.
    pub fn with_presets_overlay_git(self, url: Option<String>, branch: Option<String>) -> Self {
        let managed_overlay_dir = url.as_ref().map(|_| self.shine_dir.join("overlay"));
        Self {
            presets_overlay_dir_override: if url.is_some() {
                None
            } else {
                self.presets_overlay_dir_override
            },
            presets_overlay_git_branch: branch,
            presets_overlay_git: url,
            managed_overlay_dir,
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

    /// Which override file (if any) currently supplies `key`'s effective value.
    /// `None` means the key resolves purely from `config.toml [env]` (global or
    /// project), so writing there via `env set`/`encrypt`/`delete` is effective.
    pub fn env_override_source(&self, key: &str) -> Option<&EnvOverrideSource> {
        self.env_override_sources.get(key)
    }

    pub fn active_presets_overlay_dir(&self) -> Option<&Path> {
        if let Some(dir) = self.presets_overlay_dir_override.as_deref() {
            return Some(dir);
        }
        // A Git-managed overlay only counts as active once its checkout exists
        // on disk. Until the first `shine preset pull` clones it, resolution falls back
        // to the base presets source.
        self.managed_overlay_dir
            .as_deref()
            .filter(|dir| dir.exists())
    }

    /// Git source for a shine-managed overlay, if configured: `(url, branch,
    /// managed_dir)`. Returned regardless of whether the checkout exists yet,
    /// so `shine preset pull` can clone it on first use.
    pub fn overlay_git_source(&self) -> Option<(&str, Option<&str>, &Path)> {
        let url = self.presets_overlay_git.as_deref()?;
        let dir = self.managed_overlay_dir.as_deref()?;
        Some((url, self.presets_overlay_git_branch.as_deref(), dir))
    }

    /// Resolve a preset file with the overlay taking precedence over the base source.
    pub fn preset_path(&self, relative: impl AsRef<Path>) -> PathBuf {
        let relative = relative.as_ref();
        if let Some(overlay) = self.active_presets_overlay_dir() {
            let candidate = overlay.join(relative);
            if candidate.exists() {
                return candidate;
            }
        }
        self.presets_dir().join(relative)
    }

    fn resolve_presets_overlay_dir(&mut self, config_dir: &Path) {
        if let Some(path) = self.presets_overlay_dir_override.as_deref() {
            self.presets_overlay_dir_override = Some(resolve_config_presets_path(path, config_dir));
        }
    }

    /// Populate `managed_overlay_dir` from `presets_overlay_git`. Must be called
    /// after `shine_dir` is resolved. The managed overlay always lives at
    /// `<shine_dir>/overlay` so it follows `SHINE_CONFIG_DIR` automatically.
    fn resolve_managed_overlay_dir(&mut self) {
        self.managed_overlay_dir = self
            .presets_overlay_git
            .as_ref()
            .map(|_| self.shine_dir.join("overlay"));
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
        if let Some(dir) = config.active_presets_overlay_dir() {
            println!("{}", crate::colors::presets_overlay_note(dir));
        }
        let deployment = match config.external_shell_mode {
            ExternalShellMode::Snapshot => "snapshot · changes require `shine upgrade`",
            ExternalShellMode::Live => "live · content applies on next invocation",
        };
        println!(
            "{}",
            crate::colors::dim(&crate::colors::shell_deployment_note(deployment))
        );
        println!();
    } else if let Some(dir) = config.active_presets_overlay_dir() {
        println!("{}", crate::colors::presets_overlay_note(dir));
        println!();
    }
}

impl Default for Config {
    fn default() -> Self {
        let home_dir = crate::home::effective_home_dir();
        let shine_dir = home_dir.join(".shine");

        Self {
            presets_dir: shine_dir.join("presets"),
            bin_dir: shine_dir.join("bin"),
            config_path: shine_dir.join("config.toml"),
            is_project_config: false,
            project_save_state: None,
            shine_dir,
            home_dir,
            schema_version: CURRENT_RUNTIME_SCHEMA_VERSION,
            last_cleared_schema_version: None,
            shell_type: ShellType::default(),
            presets_dir_override: None,
            external_shell_mode: ExternalShellMode::Snapshot,
            presets_overlay_dir_override: None,
            presets_overlay_git: None,
            presets_overlay_git_branch: None,
            managed_overlay_dir: None,
            app_default_dest_root_override: None,
            is_external_presets: false,
            allow_app_hooks: false,
            sync_terminal_theme: default_sync_terminal_theme(),
            self_install_dest: None,
            gpg_key_id: None,
            secret_backend: None,
            age_recipients: Vec::new(),
            age_identity: None,
            env: default_env_map(),
            env_descriptions: BTreeMap::new(),
            env_override_sources: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
pub(super) mod test_util {
    use super::Config;
    use std::path::{Path, PathBuf};

    /// Config rooted in `dir` with a separate `home` subdirectory, mirroring
    /// the layout most config tests expect. Distinct from
    /// `crate::test_support::test_config`, which roots `home_dir` at `dir`
    /// itself — do not merge the two.
    pub(crate) fn config_in(dir: &Path) -> Config {
        Config {
            home_dir: dir.join("home"),
            ..Config::new_for_test(dir)
        }
    }

    pub(crate) async fn make_temp_dir() -> PathBuf {
        crate::test_support::make_temp_dir("shine-test").await
    }

    pub(crate) fn restore_current_dir(dir: &Path) {
        crate::test_support::restore_current_dir(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_for_test_bin_dir_is_under_root() {
        let dir = std::env::temp_dir().join("shine-test-bin-dir");
        let config = Config::new_for_test(&dir);
        assert_eq!(config.bin_dir(), dir.join("bin"));
    }

    #[test]
    fn git_overlay_is_inactive_until_checkout_exists() {
        let dir = std::env::temp_dir().join(format!("shine-overlay-git-{}", uuid::Uuid::new_v4()));
        let config = Config::new_for_test(&dir)
            .with_presets_overlay_git(Some("https://example.com/o.git".to_string()), None);

        // The source is recorded regardless of on-disk state.
        let (url, branch, managed) = config.overlay_git_source().unwrap();
        assert_eq!(url, "https://example.com/o.git");
        assert_eq!(branch, None);
        assert_eq!(managed, dir.join("overlay"));

        // But the overlay only becomes active once its checkout exists on disk.
        assert!(config.active_presets_overlay_dir().is_none());
        std::fs::create_dir_all(dir.join("overlay")).unwrap();
        assert_eq!(
            config.active_presets_overlay_dir(),
            Some(dir.join("overlay").as_path())
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn manual_overlay_path_takes_precedence_over_git() {
        let dir = std::env::temp_dir().join(format!("shine-overlay-prec-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("overlay")).unwrap();
        let manual = dir.join("manual");
        let config = Config::new_for_test(&dir)
            .with_presets_overlay_git(Some("https://example.com/o.git".to_string()), None)
            .with_presets_overlay_dir_override(Some(manual.clone()));

        assert_eq!(config.active_presets_overlay_dir(), Some(manual.as_path()));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn setting_git_overlay_clears_manual_path() {
        let dir = std::env::temp_dir().join("shine-overlay-clear");
        let config = Config::new_for_test(&dir)
            .with_presets_overlay_dir_override(Some(dir.join("manual")))
            .with_presets_overlay_git(
                Some("https://example.com/o.git".to_string()),
                Some("dev".to_string()),
            );

        assert!(config.presets_overlay_dir_override.is_none());
        assert_eq!(
            config.presets_overlay_git.as_deref(),
            Some("https://example.com/o.git")
        );
        assert_eq!(config.overlay_git_source().unwrap().1, Some("dev"));
    }

    #[test]
    fn age_identities_is_empty_without_configured_or_default_identity() {
        let dir =
            std::env::temp_dir().join(format!("shine-age-identities-{}", uuid::Uuid::new_v4()));
        let config = Config::new_for_test(&dir);

        assert!(config.age_identities().is_empty());
    }

    #[test]
    fn age_identities_uses_configured_path_when_set() {
        let dir =
            std::env::temp_dir().join(format!("shine-age-identities-{}", uuid::Uuid::new_v4()));
        let mut config = Config::new_for_test(&dir);
        config.age_identity = Some("/tmp/my-identity.txt".to_string());

        assert_eq!(
            config.age_identities(),
            vec![PathBuf::from("/tmp/my-identity.txt")]
        );
    }

    #[test]
    fn age_identities_treats_blank_configured_path_as_unset() {
        let dir =
            std::env::temp_dir().join(format!("shine-age-identities-{}", uuid::Uuid::new_v4()));
        let mut config = Config::new_for_test(&dir);
        config.age_identity = Some("   ".to_string());

        assert!(config.age_identities().is_empty());
    }

    #[tokio::test]
    async fn age_identities_falls_back_to_default_path_when_it_exists() {
        let dir =
            std::env::temp_dir().join(format!("shine-age-identities-{}", uuid::Uuid::new_v4()));
        let config = Config::new_for_test(&dir);
        let default_path = dir.join("age").join("identity.txt");
        tokio::fs::create_dir_all(default_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&default_path, "AGE-SECRET-KEY-1EXAMPLE\n")
            .await
            .unwrap();

        assert_eq!(config.age_identities(), vec![default_path]);

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
