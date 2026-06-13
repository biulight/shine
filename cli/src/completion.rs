use crate::CompletionShell;
use crate::config;
use clap::CommandFactory;
use clap_complete::engine::{ArgValueCandidates, CompletionCandidate};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub(crate) fn complete_from_env() {
    clap_complete::CompleteEnv::with_factory(command)
        .bin("shine")
        .complete();
}

pub(crate) fn generate_registration(shell: CompletionShell) {
    // SAFETY: this is called during single-threaded startup before Tokio is
    // initialized, and `CompleteEnv` removes the variable before returning.
    unsafe { std::env::set_var("COMPLETE", shell.as_str()) };
    let _ = clap_complete::CompleteEnv::with_factory(command)
        .bin("shine")
        .completer("shine")
        .try_complete(["shine"], std::env::current_dir().ok().as_deref())
        .unwrap_or_else(|e| e.exit());
}

pub(crate) fn command() -> clap::Command {
    let all_categories = ArgValueCandidates::new(all_category_candidates);
    let shell_categories = ArgValueCandidates::new(shell_category_candidates);
    let app_categories = ArgValueCandidates::new(app_category_candidates);

    crate::Cli::command()
        .mut_subcommand("install", |cmd| {
            cmd.mut_arg("category", |arg| arg.add(all_categories.clone()))
        })
        .mut_subcommand("reinstall", |cmd| {
            cmd.mut_arg("category", |arg| arg.add(all_categories.clone()))
        })
        .mut_subcommand("uninstall", |cmd| {
            cmd.mut_arg("category", |arg| arg.add(all_categories.clone()))
        })
        .mut_subcommand("shell", |cmd| {
            cmd.mut_subcommand("install", |cmd| {
                cmd.mut_arg("category", |arg| arg.add(shell_categories.clone()))
            })
            .mut_subcommand("reinstall", |cmd| {
                cmd.mut_arg("category", |arg| arg.add(shell_categories.clone()))
            })
            .mut_subcommand("uninstall", |cmd| {
                cmd.mut_arg("category", |arg| arg.add(shell_categories.clone()))
            })
        })
        .mut_subcommand("app", |cmd| {
            cmd.mut_subcommand("info", |cmd| {
                cmd.mut_arg("category", |arg| arg.add(app_categories.clone()))
            })
            .mut_subcommand("install", |cmd| {
                cmd.mut_arg("category", |arg| arg.add(app_categories.clone()))
            })
            .mut_subcommand("reinstall", |cmd| {
                cmd.mut_arg("category", |arg| arg.add(app_categories.clone()))
            })
            .mut_subcommand("uninstall", |cmd| {
                cmd.mut_arg("category", |arg| arg.add(app_categories.clone()))
            })
        })
}

fn all_category_candidates() -> Vec<CompletionCandidate> {
    let mut categories = BTreeSet::new();
    categories.extend(category_names("shell"));
    categories.extend(category_names("app"));
    completion_candidates(categories)
}

fn shell_category_candidates() -> Vec<CompletionCandidate> {
    completion_candidates(category_names("shell"))
}

fn app_category_candidates() -> Vec<CompletionCandidate> {
    completion_candidates(category_names("app"))
}

fn completion_candidates(names: BTreeSet<String>) -> Vec<CompletionCandidate> {
    names.into_iter().map(CompletionCandidate::new).collect()
}

fn category_names(kind: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    if let Some((presets_dir, true)) = runtime_presets_dir()
        && let Some(fs_names) = fs_category_names(&presets_dir.join(kind))
    {
        names.extend(fs_names);
        return names;
    }

    names.extend(embedded_category_names(kind));
    names
}

fn runtime_presets_dir() -> Option<(PathBuf, bool)> {
    let default_shine_dir = default_shine_dir();
    let default_presets_dir = default_shine_dir.join("presets");
    let current_dir = std::env::current_dir().ok();
    let global_config = shine_dir_from_env_or_default(&default_shine_dir).join("config.toml");
    let project_config = current_dir
        .as_deref()
        .and_then(|dir| find_project_config(dir, &global_config));
    let has_project_config = project_config.is_some();
    let config_path = project_config.unwrap_or(global_config);
    let config_dir = config_path.parent().unwrap_or_else(|| Path::new("."));
    let toml_presets = read_presets_override_from_toml(&config_path)
        .map(|path| resolve_config_presets_path(&path, config_dir));

    if let Ok(val) = std::env::var("SHINE_CONFIG_DIR") {
        let val = val.trim();
        if !val.is_empty() {
            let shine_dir = PathBuf::from(config::tilde_expand(val));
            if !has_project_config {
                return Some((shine_dir.join("presets"), true));
            }
            if let Some(presets) = presets_from_env() {
                return Some((presets, true));
            }
            return toml_presets
                .map(|presets| (presets, true))
                .or_else(|| Some((shine_dir.join("presets"), false)));
        }
    }

    presets_from_env()
        .map(|presets| (presets, true))
        .or_else(|| toml_presets.map(|presets| (presets, true)))
        .or(Some((default_presets_dir, false)))
}

fn presets_from_env() -> Option<PathBuf> {
    let val = std::env::var("SHINE_PRESETS").ok()?;
    let val = val.trim();
    (!val.is_empty()).then(|| PathBuf::from(config::tilde_expand(val)))
}

fn default_shine_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .ok()
        .filter(|home| !home.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".shine")
}

fn shine_dir_from_env_or_default(default: &Path) -> PathBuf {
    std::env::var("SHINE_CONFIG_DIR")
        .ok()
        .map(|val| val.trim().to_string())
        .filter(|val| !val.is_empty())
        .map(|val| PathBuf::from(config::tilde_expand(&val)))
        .unwrap_or_else(|| default.to_path_buf())
}

fn read_presets_override_from_toml(path: &Path) -> Option<PathBuf> {
    let content = std::fs::read_to_string(path).ok()?;
    #[derive(serde::Deserialize)]
    struct MinimalConfig {
        #[serde(default)]
        presets_dir: Option<PathBuf>,
    }
    toml::from_str::<MinimalConfig>(&content).ok()?.presets_dir
}

fn resolve_config_presets_path(path: &Path, config_dir: &Path) -> PathBuf {
    if let Some(s) = path.to_str()
        && s.starts_with('~')
    {
        return PathBuf::from(config::tilde_expand(s));
    }

    if path.is_absolute() {
        path.to_path_buf()
    } else {
        config_dir.join(path)
    }
}

fn find_project_config(start: &Path, global_config: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let path = dir.join("shine.config.toml");
        if path.exists() {
            return Some(path);
        }

        let legacy_path = dir.join("config.toml");
        if legacy_path.exists()
            && legacy_path != global_config
            && config_has_presets_dir(&legacy_path)
        {
            return Some(legacy_path);
        }
    }
    None
}

fn config_has_presets_dir(path: &Path) -> bool {
    read_presets_override_from_toml(path).is_some()
}

fn fs_category_names(root: &Path) -> Option<BTreeSet<String>> {
    if !root.is_dir() {
        return None;
    }

    let mut names = BTreeSet::new();
    for entry in std::fs::read_dir(root).ok()? {
        let entry = entry.ok()?;
        if entry.file_type().ok()?.is_dir()
            && let Some(name) = entry.file_name().to_str()
        {
            names.insert(name.to_string());
        }
    }
    Some(names)
}

fn embedded_category_names(kind: &str) -> BTreeSet<String> {
    let prefix = format!("{kind}/");
    crate::presets::asset_paths(kind)
        .into_iter()
        .filter_map(|path| {
            let rest = path.strip_prefix(&prefix)?;
            let (category, _) = rest.split_once('/')?;
            Some(category.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("shine-completion-{name}-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn embedded_candidates_include_known_categories() {
        let _guard = env_lock().lock().unwrap();
        unsafe {
            std::env::remove_var("SHINE_CONFIG_DIR");
            std::env::remove_var("SHINE_PRESETS");
        }

        let shell = category_names("shell");
        let app = category_names("app");

        assert!(shell.contains("proxy"), "shell candidates: {shell:?}");
        assert!(app.contains("starship"), "app candidates: {app:?}");
    }

    #[test]
    fn config_dir_candidates_read_external_presets_without_creating_config() {
        let _guard = env_lock().lock().unwrap();
        let dir = temp_dir("config-dir");
        std::fs::create_dir_all(dir.join("presets/shell/custom-shell")).unwrap();
        std::fs::create_dir_all(dir.join("presets/app/custom-app")).unwrap();

        unsafe {
            std::env::set_var("SHINE_CONFIG_DIR", &dir);
            std::env::remove_var("SHINE_PRESETS");
        }

        assert!(category_names("shell").contains("custom-shell"));
        assert!(category_names("app").contains("custom-app"));
        assert!(
            !dir.join("config.toml").exists(),
            "completion must not initialize config files"
        );

        unsafe {
            std::env::remove_var("SHINE_CONFIG_DIR");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn completion_command_accepts_dynamic_registration_shells() {
        let _guard = env_lock().lock().unwrap();
        for shell in ["bash", "powershell", "zsh"] {
            unsafe { std::env::set_var("COMPLETE", shell) };
            let completed = clap_complete::CompleteEnv::with_factory(command)
                .bin("shine")
                .completer("shine")
                .try_complete(["shine"], None);
            assert!(
                completed.is_ok_and(|completed| completed),
                "completion setup should be callable for {shell}"
            );
        }
    }
}
