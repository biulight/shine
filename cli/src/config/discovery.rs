use anyhow::{Context, Result};
use serde::{Deserialize, de::DeserializeOwned};
use std::path::{Path, PathBuf};

use super::{GLOBAL_CONFIG_FILE, PROJECT_CONFIG_FILE, tilde_expand};
use crate::home::default_config_and_presets_dir;

#[derive(Clone, Debug)]
pub(crate) struct ProjectConfig {
    pub(crate) path: PathBuf,
    pub(crate) root: PathBuf,
}

pub(crate) fn find_project_config(start: &Path) -> Option<ProjectConfig> {
    find_ancestor_file(start, PROJECT_CONFIG_FILE).and_then(|path| {
        let root = path.parent()?.to_path_buf();
        Some(ProjectConfig { root, path })
    })
}

fn find_ancestor_file(start: &Path, file_name: &str) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let path = dir.join(file_name);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Minimal view of a config file used before full `Config` deserialization.
#[derive(Deserialize)]
pub(super) struct MinimalConfig {
    #[serde(default)]
    pub(super) presets_dir: Option<PathBuf>,
    #[serde(default)]
    pub(super) presets_overlay_dir: Option<PathBuf>,
    #[serde(default)]
    pub(super) presets_overlay_git: Option<String>,
    #[serde(default)]
    pub(super) schema_version: u32,
}

// Project configuration can override directories. Git and runtime schema settings are global.
#[derive(Default, Deserialize)]
struct ProjectPresetPaths {
    presets_dir: Option<PathBuf>,
    presets_overlay_dir: Option<PathBuf>,
}

pub(super) fn read_minimal_config(content: &str) -> anyhow::Result<MinimalConfig> {
    toml::from_str(content).map_err(super::config_parse_error)
}

/// Return the shine root dir implied by `SHINE_CONFIG_DIR`, or `default` if unset.
/// Used for a preliminary read of global config before full resolution.
pub(super) fn preliminary_shine_dir_from_env(default: &Path) -> PathBuf {
    if let Ok(val) = std::env::var("SHINE_CONFIG_DIR") {
        let val = val.trim().to_string();
        if !val.is_empty() {
            return PathBuf::from(tilde_expand(&val));
        }
    }
    default.to_owned()
}

pub(super) fn resolve_config_presets_path(path: &Path, config_dir: &Path) -> PathBuf {
    if let Some(s) = path.to_str()
        && s.starts_with('~')
    {
        return PathBuf::from(tilde_expand(s));
    }

    if path.is_absolute() {
        path.to_path_buf()
    } else {
        config_dir.join(path)
    }
}

/// Paths needed by latency-sensitive, read-only commands such as dynamic shell
/// completion. Discovery never creates config or preset directories.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReadOnlyRuntimePaths {
    pub(crate) shine_dir: PathBuf,
    pub(crate) presets_dir: PathBuf,
    pub(crate) presets_overlay_dir: Option<PathBuf>,
    pub(crate) is_external_presets: bool,
    pub(crate) managed_overlay_dir: Option<PathBuf>,
}

/// Resolve the active runtime paths without initializing `Config` or writing
/// any state. This mirrors the global/project preset layering in
/// `Config::load_or_init` while deliberately ignoring unrelated config keys.
pub(crate) fn discover_runtime_paths_read_only() -> Option<ReadOnlyRuntimePaths> {
    let (default_shine_dir, default_presets_dir) = default_config_and_presets_dir().ok()?;
    let preliminary_shine_dir = preliminary_shine_dir_from_env(&default_shine_dir);
    let global_path = preliminary_shine_dir.join(GLOBAL_CONFIG_FILE);
    let global = read_minimal_config_file(&global_path);
    let project = find_project_config(&std::env::current_dir().ok()?).map(|project| {
        let config = read_minimal_config_file(&project.path).unwrap_or_default();
        (project.root, config)
    });
    Some(resolve_read_only_paths(
        &default_shine_dir,
        &default_presets_dir,
        &preliminary_shine_dir,
        global,
        project,
    ))
}

/// Migration may write to the selected overlay. Only an absent layer can be skipped.
pub(crate) async fn discover_migration_paths() -> Result<ReadOnlyRuntimePaths> {
    let (default_shine_dir, default_presets_dir) = default_config_and_presets_dir()?;
    let global_dir = preliminary_shine_dir_from_env(&default_shine_dir);
    let global = capture_minimal_config(&global_dir.join(GLOBAL_CONFIG_FILE)).await?;
    let current = std::env::current_dir().context("resolving current directory")?;
    let mut project = None;
    for dir in current.ancestors() {
        if let Some(config) = capture_minimal_config(&dir.join(PROJECT_CONFIG_FILE)).await? {
            project = Some((dir.to_owned(), config));
            break;
        }
    }
    Ok(resolve_read_only_paths(
        &default_shine_dir,
        &default_presets_dir,
        &global_dir,
        global,
        project,
    ))
}

async fn capture_minimal_config<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    super::ConfigInput::capture(path)
        .await?
        .contents
        .as_deref()
        .map(|contents| toml::from_str(contents).map_err(super::config_parse_error))
        .transpose()
        .with_context(|| {
            format!(
                "parsing {}",
                crate::output::escape_controls(&path.display().to_string())
            )
        })
}

fn resolve_read_only_paths(
    default_shine_dir: &Path,
    default_presets_dir: &Path,
    global_dir: &Path,
    global: Option<MinimalConfig>,
    project: Option<(PathBuf, ProjectPresetPaths)>,
) -> ReadOnlyRuntimePaths {
    let global_presets = global
        .as_ref()
        .and_then(|config| config.presets_dir.as_deref())
        .map(|path| resolve_config_presets_path(path, global_dir));
    let (global_shine_dir, global_presets_dir, global_is_external) = resolve_runtime_config_dirs(
        default_shine_dir,
        default_presets_dir,
        global_presets.as_deref(),
        false,
    );
    let managed_overlay_dir = global
        .as_ref()
        .filter(|config| {
            config.presets_overlay_dir.is_none() && config.presets_overlay_git.is_some()
        })
        .map(|_| global_shine_dir.join("overlay"));
    let global_overlay = global
        .as_ref()
        .and_then(|config| config.presets_overlay_dir.as_deref())
        .map(|path| resolve_config_presets_path(path, global_dir))
        .or_else(|| managed_overlay_dir.clone().filter(|path| path.exists()));

    let Some((project_root, project_config)) = project else {
        return ReadOnlyRuntimePaths {
            shine_dir: global_shine_dir,
            presets_dir: global_presets_dir,
            presets_overlay_dir: global_overlay,
            is_external_presets: global_is_external,
            managed_overlay_dir,
        };
    };

    let project_presets = project_config
        .presets_dir
        .as_deref()
        .map(|path| resolve_config_presets_path(path, &project_root));
    let effective_presets = project_presets
        .clone()
        .or_else(|| global_is_external.then_some(global_presets_dir.clone()));
    let runtime_presets = if project_presets.is_none()
        && std::env::var("SHINE_CONFIG_DIR").is_ok_and(|value| !value.trim().is_empty())
    {
        None
    } else {
        effective_presets
    };
    let (shine_dir, presets_dir, is_external_presets) = resolve_runtime_config_dirs(
        default_shine_dir,
        default_presets_dir,
        runtime_presets.as_deref(),
        true,
    );
    let presets_overlay_dir = project_config
        .presets_overlay_dir
        .as_deref()
        .map(|path| resolve_config_presets_path(path, &project_root))
        .or(global_overlay);

    ReadOnlyRuntimePaths {
        shine_dir,
        presets_dir,
        presets_overlay_dir,
        is_external_presets,
        managed_overlay_dir,
    }
}

fn read_minimal_config_file<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let content = std::fs::read_to_string(path).ok()?;
    toml::from_str(&content).ok()
}

/// Resolve the runtime (shine_dir, presets_dir) pair.
///
/// Priority (highest first):
///   1. `SHINE_CONFIG_DIR` — overrides both shine_dir and presets_dir unless project config is active
///   2. `SHINE_PRESETS`    — overrides presets_dir only
///   3. `config_toml_presets` — presets_dir from active config `presets_dir` key
///   4. defaults
///
/// Returns `(shine_dir, presets_dir, is_external_presets)`.
pub(super) fn resolve_runtime_config_dirs(
    default_shine_dir: &Path,
    default_presets_dir: &Path,
    config_toml_presets: Option<&Path>,
    has_local_config: bool,
) -> (PathBuf, PathBuf, bool) {
    if let Ok(val) = std::env::var("SHINE_CONFIG_DIR") {
        let val = val.trim().to_string();
        if !val.is_empty() {
            let dir = PathBuf::from(tilde_expand(&val));
            if !has_local_config {
                return (dir.clone(), dir.join("presets"), true);
            }
            if let Ok(val) = std::env::var("SHINE_PRESETS") {
                let val = val.trim().to_string();
                if !val.is_empty() {
                    let presets = PathBuf::from(tilde_expand(&val));
                    return (dir, presets, true);
                }
            }
            if let Some(p) = config_toml_presets
                && let Some(s) = p.to_str()
            {
                let presets = PathBuf::from(tilde_expand(s));
                return (dir, presets, true);
            }
            return (dir.clone(), dir.join("presets"), false);
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
    use super::super::test_util::make_temp_dir;
    use super::*;
    use crate::test_support::env_lock;
    use tokio::fs;

    #[tokio::test]
    async fn project_config_discovery_finds_shine_config() {
        let project_dir = make_temp_dir().await;
        let child_dir = project_dir.join("subdir");
        fs::create_dir_all(&child_dir).await.unwrap();
        fs::write(
            project_dir.join("shine.config.toml"),
            "presets_dir = \".\"\n",
        )
        .await
        .unwrap();
        let config = find_project_config(&child_dir).expect("project config should be found");

        assert_eq!(config.path, project_dir.join("shine.config.toml"));

        fs::remove_dir_all(&project_dir).await.unwrap();
    }

    #[tokio::test]
    async fn generic_config_toml_is_not_project_config() {
        let project_dir = make_temp_dir().await;
        fs::write(
            project_dir.join("config.toml"),
            "presets_dir = \"legacy\"\n",
        )
        .await
        .unwrap();

        assert!(find_project_config(&project_dir).is_none());

        fs::remove_dir_all(&project_dir).await.unwrap();
    }

    #[test]
    fn read_only_discovery_marks_existing_git_overlay_as_managed() {
        let _guard = env_lock();
        let root = std::env::temp_dir().join(format!(
            "shine-discovery-managed-overlay-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join("overlay")).unwrap();
        std::fs::write(
            root.join(GLOBAL_CONFIG_FILE),
            "presets_overlay_git = 'https://example.invalid/presets.git'\n",
        )
        .unwrap();
        // SAFETY: env_lock serializes process-global environment and cwd mutation.
        unsafe {
            std::env::set_var("SHINE_CONFIG_DIR", &root);
            std::env::remove_var("SHINE_PRESETS");
        }
        std::env::set_current_dir(&root).unwrap();

        let runtime = discover_runtime_paths_read_only().unwrap();

        drop(_guard);

        assert_eq!(runtime.managed_overlay_dir, Some(root.join("overlay")));
        assert_eq!(runtime.presets_overlay_dir, Some(root.join("overlay")));
        std::fs::remove_dir_all(root).unwrap();
    }

    fn resolve_with_overrides(
        config_dir: Option<&str>,
        presets_env: Option<&str>,
        toml_presets: Option<&str>,
        local: bool,
    ) -> (PathBuf, PathBuf, bool) {
        let _guard = env_lock();
        // SAFETY: the guard serializes these changes and restores the caller's environment.
        for (name, value) in [
            ("SHINE_CONFIG_DIR", config_dir),
            ("SHINE_PRESETS", presets_env),
        ] {
            unsafe {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
        resolve_runtime_config_dirs(
            Path::new("/default"),
            Path::new("/default/presets"),
            toml_presets.map(Path::new),
            local,
        )
    }

    #[test]
    fn global_config_resolves_state_and_presets_in_priority_order() {
        let state = Some("/state");
        let env = Some("/env");
        let configured = Some("/configured");
        let from_state = ("/state", "/state/presets", true);
        let from_env = ("/default", "/env", true);
        for (config_dir, presets_env, toml_presets, expected) in [
            (None, None, None, ("/default", "/default/presets", false)),
            (None, None, configured, ("/default", "/configured", true)),
            (None, env, None, from_env),
            (None, env, configured, from_env),
            (state, None, None, from_state),
            (state, env, None, from_state),
        ] {
            let (state_dir, presets_dir, external) = expected;
            assert_eq!(
                resolve_with_overrides(config_dir, presets_env, toml_presets, false),
                (state_dir.into(), presets_dir.into(), external),
                "SHINE_CONFIG_DIR={config_dir:?}, SHINE_PRESETS={presets_env:?}, presets_dir={toml_presets:?}"
            );
        }
    }

    #[test]
    fn local_config_keeps_shine_config_dir_as_state_only() {
        assert_eq!(
            resolve_with_overrides(Some("/state"), None, Some("/project"), true),
            ("/state".into(), "/project".into(), true)
        );
    }

    #[tokio::test]
    async fn minimal_config_distinguishes_missing_file_absent_key_and_configured_path() {
        let dir = make_temp_dir().await;
        let path = dir.join("config.toml");
        let read = || read_minimal_config_file::<MinimalConfig>(&path);
        assert!(read().is_none());

        for (contents, expected) in [
            ("schema_version = 0\n", None),
            (
                "schema_version = 0\npresets_dir = \"/my/presets\"\n",
                Some(PathBuf::from("/my/presets")),
            ),
        ] {
            fs::write(&path, contents).await.unwrap();
            assert_eq!(read().unwrap().presets_dir, expected, "{contents}");
        }
        fs::remove_dir_all(dir).await.unwrap();
    }
}
