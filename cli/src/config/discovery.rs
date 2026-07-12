use serde::Deserialize;
use std::path::{Path, PathBuf};

use super::{LEGACY_PROJECT_CONFIG_FILE, PROJECT_CONFIG_FILE, tilde_expand};

#[derive(Clone, Debug)]
pub(super) struct ProjectConfig {
    pub(super) path: PathBuf,
    pub(super) root: PathBuf,
    pub(super) is_legacy: bool,
}

pub(super) fn find_project_config(
    start: &Path,
    global_config_path: &Path,
) -> Option<ProjectConfig> {
    find_ancestor_file(start, PROJECT_CONFIG_FILE)
        .and_then(|path| {
            let root = path.parent()?.to_path_buf();
            Some(ProjectConfig {
                root,
                path,
                is_legacy: false,
            })
        })
        .or_else(|| {
            find_ancestor_file(start, LEGACY_PROJECT_CONFIG_FILE)
                .filter(|path| !paths_match(path, global_config_path))
                .filter(|path| config_file_has_presets_dir(path))
                .and_then(|path| {
                    let root = path.parent()?.to_path_buf();
                    Some(ProjectConfig {
                        root,
                        path,
                        is_legacy: true,
                    })
                })
        })
}

fn paths_match(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    // If canonicalization fails for either side (most commonly because the global
    // config doesn't exist yet), treat the paths as not matching.  A non-NotFound
    // I/O error (e.g. permission denied on an ancestor) would also return false
    // here, which is intentionally conservative: the project config is then used
    // without attempting to merge in the global one.
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
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
    pub(super) schema_version: u32,
}

pub(super) fn read_minimal_config(content: &str) -> Result<MinimalConfig, toml::de::Error> {
    toml::from_str(content)
}

fn config_file_has_presets_dir(path: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    read_minimal_config(&content)
        .map(|config| config.presets_dir.is_some())
        .unwrap_or(false)
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

/// Attempt to read the `presets_dir` key from an existing config without
/// doing a full parse. Returns `None` if the file is absent, unreadable, or the
/// key is not set.
pub(super) async fn read_presets_override_from_toml(config_path: &Path) -> Option<PathBuf> {
    let content = tokio::fs::read_to_string(config_path).await.ok()?;
    read_minimal_config(&content).ok()?.presets_dir
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
