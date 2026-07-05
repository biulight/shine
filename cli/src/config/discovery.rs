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

fn config_file_has_presets_dir(path: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(path) else {
        return false;
    };
    #[derive(Deserialize)]
    struct MinimalConfig {
        #[serde(default)]
        presets_dir: Option<PathBuf>,
    }
    toml::from_str::<MinimalConfig>(&content)
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
    #[derive(Deserialize)]
    struct MinimalConfig {
        #[serde(default)]
        presets_dir: Option<PathBuf>,
    }
    let partial: MinimalConfig = toml::from_str(&content).ok()?;
    partial.presets_dir
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
