use crate::commands::CompletionShell;
use clap::CommandFactory;
use clap_complete::engine::{ArgValueCandidates, CompletionCandidate};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub fn complete_from_env() {
    clap_complete::CompleteEnv::with_factory(command)
        .bin("shine")
        .complete();
}

pub fn generate_registration(shell: CompletionShell) {
    // SAFETY: this is called during single-threaded startup before Tokio is
    // initialized, and `CompleteEnv` removes the variable before returning.
    unsafe { std::env::set_var("COMPLETE", shell.as_str()) };
    let _ = clap_complete::CompleteEnv::with_factory(command)
        .bin("shine")
        .completer("shine")
        .try_complete(["shine"], std::env::current_dir().ok().as_deref())
        .unwrap_or_else(|e| e.exit());
}

pub fn command() -> clap::Command {
    let preset_targets = ArgValueCandidates::new(preset_target_candidates);
    let shell_lifecycle_targets = ArgValueCandidates::new(shell_lifecycle_candidates);
    let shell_targets = ArgValueCandidates::new(shell_info_candidates);
    let app_categories = ArgValueCandidates::new(app_category_candidates);
    let app_build_categories = ArgValueCandidates::new(app_build_candidates);
    let app_unbuild_categories = ArgValueCandidates::new(app_unbuild_candidates);
    let app_refresh_categories = ArgValueCandidates::new(app_refresh_candidates);
    let sys_items = ArgValueCandidates::new(sys_item_candidates);
    let sys_updates = ArgValueCandidates::new(sys_update_candidates);
    let resource_targets = ArgValueCandidates::new(resource_target_candidates);
    let upgrade_targets = ArgValueCandidates::new(upgrade_target_candidates);
    let installed_targets = ArgValueCandidates::new(installed_target_candidates);
    let task_names = ArgValueCandidates::new(task_name_candidates);
    let preset_copy_targets = ArgValueCandidates::new(preset_copy_candidates);

    crate::commands::Cli::command()
        .mut_subcommand("install", |cmd| {
            cmd.mut_arg("target", |arg| arg.add(preset_targets.clone()))
        })
        .mut_subcommand("uninstall", |cmd| {
            cmd.mut_arg("target", |arg| arg.add(preset_targets.clone()))
        })
        .mut_subcommand("info", |cmd| {
            cmd.mut_arg("target", |arg| arg.add(resource_targets.clone()))
        })
        .mut_subcommand("update", |cmd| {
            cmd.mut_arg("target", |arg| arg.add(installed_targets.clone()))
        })
        .mut_subcommand("upgrade", |cmd| {
            cmd.mut_arg("target", |arg| arg.add(upgrade_targets))
        })
        .mut_subcommand("shell", |cmd| {
            cmd.mut_subcommand("info", |cmd| {
                cmd.mut_arg("target", |arg| arg.add(shell_targets.clone()))
            })
            .mut_subcommand("install", |cmd| {
                cmd.mut_arg("target", |arg| arg.add(shell_lifecycle_targets.clone()))
            })
            .mut_subcommand("uninstall", |cmd| {
                cmd.mut_arg("target", |arg| arg.add(shell_lifecycle_targets.clone()))
            })
        })
        .mut_subcommand("app", |cmd| {
            cmd.mut_subcommand("info", |cmd| {
                cmd.mut_arg("category", |arg| arg.add(app_categories.clone()))
            })
            .mut_subcommand("install", |cmd| {
                cmd.mut_arg("category", |arg| arg.add(app_categories.clone()))
            })
            .mut_subcommand("uninstall", |cmd| {
                cmd.mut_arg("category", |arg| arg.add(app_categories.clone()))
            })
            .mut_subcommand("refresh", |cmd| {
                cmd.mut_arg("category", |arg| {
                    arg.index(1).add(app_refresh_categories.clone())
                })
                .mut_arg("file", |arg| arg.index(2))
            })
            .mut_subcommand("artifact", |cmd| {
                cmd.mut_subcommand("apply", |cmd| {
                    cmd.mut_arg("app_id", |arg| arg.add(app_build_categories.clone()))
                })
                .mut_subcommand("remove", |cmd| {
                    cmd.mut_arg("app_id", |arg| arg.add(app_unbuild_categories.clone()))
                })
            })
        })
        .mut_subcommand("sys", |cmd| {
            cmd.mut_subcommand("info", |cmd| {
                cmd.mut_arg("item", |arg| arg.add(sys_items.clone()))
            })
            .mut_subcommand("apply", |cmd| {
                cmd.mut_arg("item", |arg| arg.add(sys_items.clone()))
            })
            .mut_subcommand("update", |cmd| {
                cmd.mut_arg("item", |arg| arg.add(sys_updates.clone()))
            })
            .mut_subcommand("uninstall", |cmd| {
                cmd.mut_arg("item", |arg| arg.add(sys_items.clone()))
            })
        })
        .mut_subcommand("task", |cmd| {
            cmd.mut_subcommand("run", |cmd| {
                cmd.mut_arg("name", |arg| arg.index(1).add(task_names.clone()))
                    .mut_arg("extra", |arg| arg.index(2))
            })
            .mut_subcommand("info", |cmd| {
                cmd.mut_arg("name", |arg| arg.add(task_names.clone()))
            })
            .mut_subcommand("delete", |cmd| {
                cmd.mut_arg("name", |arg| arg.add(task_names.clone()))
            })
        })
        .mut_subcommand("preset", |cmd| {
            cmd.mut_subcommand("copy", |cmd| {
                cmd.mut_arg("target", |arg| arg.add(preset_copy_targets))
            })
        })
        .mut_subcommand("run", |cmd| {
            cmd.mut_arg("name", |arg| arg.index(1).add(task_names))
                .mut_arg("extra", |arg| arg.index(2))
        })
}

fn preset_copy_candidates() -> Vec<CompletionCandidate> {
    let targets = crate::presets::embedded_asset_paths("")
        .into_iter()
        .filter_map(|path| {
            let mut parts = path.split('/');
            let kind = parts.next()?;
            let name = parts.next()?;
            matches!(kind, "app" | "shell" | "sys").then(|| format!("{kind}/{name}"))
        })
        .collect();
    completion_candidates(targets)
}

fn preset_target_candidates() -> Vec<CompletionCandidate> {
    let mut targets = BTreeSet::new();
    for category in category_names("app") {
        targets.insert(category.clone());
        targets.insert(format!("app/{category}"));
    }
    for category in category_names("shell") {
        targets.insert(category.clone());
        targets.insert(format!("shell/{category}"));
    }
    for (category, commands) in shell_command_names() {
        for command in commands {
            targets.insert(format!("shell/{category}/{command}"));
        }
    }
    completion_candidates(targets)
}

fn shell_lifecycle_candidates() -> Vec<CompletionCandidate> {
    let mut names = shell_category_names();
    for (category, commands) in shell_command_names() {
        names.extend(
            commands
                .into_iter()
                .map(|command| format!("{category}/{command}")),
        );
    }
    completion_candidates(names)
}

fn shell_info_candidates() -> Vec<CompletionCandidate> {
    let mut names = shell_category_names();
    for (category, commands) in shell_command_names() {
        for command in commands {
            names.insert(command.clone());
            names.insert(format!("{category}/{command}"));
        }
    }
    completion_candidates(names)
}

fn shell_category_names() -> BTreeSet<String> {
    category_names("shell")
}

fn shell_command_names() -> Vec<(String, BTreeSet<String>)> {
    let Some(paths) = runtime_paths() else {
        return Vec::new();
    };
    if paths.is_external_presets {
        return collect_fs_shell_command_names(&paths.presets_dir.join("shell"))
            .into_iter()
            .collect();
    }

    let mut commands = crate::shells::metadata::load_embedded_categories(None)
        .unwrap_or_default()
        .into_iter()
        .map(|category| {
            (
                category.name,
                category
                    .files
                    .into_iter()
                    .map(|file| file.command_name)
                    .collect::<BTreeSet<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    if let Some(overlay_dir) = paths.presets_overlay_dir {
        for (category, overlay_commands) in
            collect_fs_shell_command_names(&overlay_dir.join("shell"))
        {
            commands
                .entry(category)
                .or_default()
                .extend(overlay_commands);
        }
    }
    commands.into_iter().collect()
}

fn collect_fs_shell_command_names(root: &Path) -> BTreeMap<String, BTreeSet<String>> {
    #[derive(serde::Deserialize)]
    struct ShellManifest {
        files: Option<Vec<ShellFile>>,
    }
    #[derive(serde::Deserialize)]
    struct ShellFile {
        source: PathBuf,
        target: Option<String>,
    }

    let mut result = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return result;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let Some(category) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let dir = entry.path();
        let declared_commands = if let Ok(content) = std::fs::read_to_string(dir.join("shine.toml"))
        {
            toml::from_str::<ShellManifest>(&content)
                .map(|manifest| {
                    manifest.files.map(|files| {
                        files
                            .into_iter()
                            .filter_map(|file| {
                                file.target.or_else(|| {
                                    crate::bin_links::link_stem(&file.source).into_string().ok()
                                })
                            })
                            .collect()
                    })
                })
                .unwrap_or(None)
        } else {
            None
        };
        let commands = declared_commands.unwrap_or_else(|| {
            std::fs::read_dir(&dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|file| {
                    let path = file.path();
                    (path.is_file()
                        && matches!(
                            path.extension().and_then(|extension| extension.to_str()),
                            Some("sh" | "ps1")
                        ))
                    .then(|| crate::bin_links::link_stem(&path).into_string().ok())
                    .flatten()
                })
                .collect()
        });
        result.insert(category, commands);
    }
    result
}

fn app_category_candidates() -> Vec<CompletionCandidate> {
    completion_candidates(category_names("app"))
}

fn app_build_candidates() -> Vec<CompletionCandidate> {
    completion_candidates(app_capability_names(AppCapability::Build))
}

fn app_unbuild_candidates() -> Vec<CompletionCandidate> {
    completion_candidates(app_capability_names(AppCapability::Unbuild))
}

fn app_refresh_candidates() -> Vec<CompletionCandidate> {
    completion_candidates(app_capability_names(AppCapability::Refresh))
}

#[derive(Clone, Copy)]
enum AppCapability {
    Build,
    Unbuild,
    Refresh,
}

fn app_capability_names(capability: AppCapability) -> BTreeSet<String> {
    let Some(paths) = runtime_paths() else {
        return BTreeSet::new();
    };
    if paths.is_external_presets {
        return collect_fs_app_capabilities(&paths.presets_dir.join("app"), capability);
    }

    let mut names: BTreeSet<String> = crate::apps::load_embedded_categories(None)
        .unwrap_or_default()
        .into_iter()
        .filter(|category| match capability {
            AppCapability::Build => category.artifact.is_some(),
            AppCapability::Unbuild => category
                .artifact
                .as_ref()
                .is_some_and(|artifact| artifact.teardown.is_some()),
            AppCapability::Refresh => category.files.iter().any(|file| file.generator.is_some()),
        })
        .map(|category| category.name)
        .collect();
    if let Some(overlay_dir) = paths.presets_overlay_dir {
        names.extend(collect_fs_app_capabilities(
            &overlay_dir.join("app"),
            capability,
        ));
    }
    names
}

fn collect_fs_app_capabilities(root: &Path, capability: AppCapability) -> BTreeSet<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return BTreeSet::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            let content = std::fs::read_to_string(entry.path().join("shine.toml")).ok()?;
            let value = toml::from_str::<toml::Value>(&content).ok()?;
            let matches = match capability {
                AppCapability::Build => value.get("artifact").is_some_and(|artifact| {
                    artifact
                        .get("script")
                        .and_then(toml::Value::as_str)
                        .is_some()
                }),
                AppCapability::Unbuild => value.get("artifact").is_some_and(|artifact| {
                    artifact
                        .get("teardown")
                        .and_then(toml::Value::as_str)
                        .is_some()
                }),
                AppCapability::Refresh => value
                    .get("files")
                    .and_then(toml::Value::as_array)
                    .is_some_and(|files| files.iter().any(|file| file.get("generator").is_some())),
            };
            matches.then_some(name)
        })
        .collect()
}

fn installed_target_candidates() -> Vec<CompletionCandidate> {
    completion_candidates(installed_target_names())
}

fn resource_target_candidates() -> Vec<CompletionCandidate> {
    let mut names = installed_target_names();
    for category in category_names("app") {
        names.insert(category.clone());
        names.insert(format!("app/{category}"));
    }
    for (category, commands) in shell_command_names() {
        names.insert(category.clone());
        names.insert(format!("shell/{category}"));
        for command in commands {
            names.insert(command.clone());
            names.insert(format!("{category}/{command}"));
            names.insert(format!("shell/{category}/{command}"));
        }
    }
    names.extend(
        sys_item_names()
            .into_iter()
            .map(|item| format!("sys/{item}")),
    );
    completion_candidates(names)
}

fn upgrade_target_candidates() -> Vec<CompletionCandidate> {
    let mut names = installed_target_names();
    names.extend(
        sys_item_names()
            .into_iter()
            .map(|item| format!("sys/{item}")),
    );
    completion_candidates(names)
}

fn installed_target_names() -> BTreeSet<String> {
    #[derive(serde::Deserialize)]
    struct AppManifest {
        #[serde(default)]
        entries: Vec<AppEntry>,
    }
    #[derive(serde::Deserialize)]
    struct AppEntry {
        source: String,
    }

    let Some(paths) = runtime_paths() else {
        return BTreeSet::new();
    };
    let mut names = BTreeSet::new();
    if let Ok(content) = std::fs::read_to_string(paths.shine_dir.join("app-manifest.toml"))
        && let Ok(manifest) = toml::from_str::<AppManifest>(&content)
    {
        for entry in manifest.entries {
            let mut parts = entry.source.splitn(3, '/');
            if let (Some("app"), Some(category), Some(file)) =
                (parts.next(), parts.next(), parts.next())
            {
                names.insert(category.to_string());
                names.insert(format!("app/{category}"));
                names.insert(format!("{category}/{file}"));
                names.insert(format!("app/{category}/{file}"));
            }
        }
    }

    for (category, commands) in collect_fs_shell_command_names(&paths.presets_dir.join("shell")) {
        names.insert(category.clone());
        names.insert(format!("shell/{category}"));
        for command in commands {
            names.insert(command.clone());
            names.insert(format!("{category}/{command}"));
            names.insert(format!("shell/{category}/{command}"));
        }
    }
    names
}

fn sys_item_candidates() -> Vec<CompletionCandidate> {
    completion_candidates(sys_item_names())
}

fn sys_item_names() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let Some(paths) = runtime_paths() else {
        return names;
    };
    if paths.is_external_presets {
        collect_fs_sys_item_names(&paths.presets_dir.join("sys"), &mut names);
        return names;
    }

    for path in crate::presets::asset_paths("sys") {
        if !path.ends_with("/shine.toml") {
            continue;
        }
        if let Some(bytes) = crate::presets::read_asset_bytes(&path) {
            collect_toml_sys_item_names(&bytes, &mut names);
        }
    }
    if let Some(overlay_dir) = paths.presets_overlay_dir {
        collect_fs_sys_item_names(&overlay_dir.join("sys"), &mut names);
    }
    names
}

fn sys_update_candidates() -> Vec<CompletionCandidate> {
    #[derive(serde::Deserialize)]
    struct Manifest {
        #[serde(default)]
        entries: Vec<Entry>,
    }
    #[derive(serde::Deserialize)]
    struct Entry {
        item_id: String,
        #[serde(default)]
        managed: bool,
    }

    let names = runtime_paths()
        .and_then(|paths| std::fs::read_to_string(paths.shine_dir.join("sys-manifest.toml")).ok())
        .and_then(|content| toml::from_str::<Manifest>(&content).ok())
        .map(|manifest| {
            manifest
                .entries
                .into_iter()
                .filter(|entry| !entry.managed)
                .map(|entry| entry.item_id)
                .collect()
        })
        .unwrap_or_default();
    completion_candidates(names)
}

fn task_name_candidates() -> Vec<CompletionCandidate> {
    #[derive(serde::Deserialize)]
    struct Manifest {
        #[serde(default)]
        tasks: BTreeMap<String, toml::Value>,
    }

    let names = runtime_paths()
        .and_then(|paths| std::fs::read_to_string(paths.shine_dir.join("tasks.toml")).ok())
        .and_then(|content| toml::from_str::<Manifest>(&content).ok())
        .map(|manifest| manifest.tasks.into_keys().collect())
        .unwrap_or_default();
    completion_candidates(names)
}

fn collect_fs_sys_item_names(root: &Path, names: &mut BTreeSet<String>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let manifest = entry.path().join("shine.toml");
        if let Ok(bytes) = std::fs::read(manifest) {
            collect_toml_sys_item_names(&bytes, names);
        }
    }
}

fn collect_toml_sys_item_names(bytes: &[u8], names: &mut BTreeSet<String>) {
    let Ok(content) = std::str::from_utf8(bytes) else {
        return;
    };
    let Ok(value) = toml::from_str::<toml::Value>(content) else {
        return;
    };
    let Some(items) = value.get("items").and_then(toml::Value::as_array) else {
        return;
    };
    names.extend(items.iter().filter_map(|item| {
        item.get("id")
            .and_then(toml::Value::as_str)
            .map(str::to_string)
    }));
}

fn completion_candidates(names: BTreeSet<String>) -> Vec<CompletionCandidate> {
    names.into_iter().map(CompletionCandidate::new).collect()
}

fn category_names(kind: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let Some(paths) = runtime_paths() else {
        return names;
    };
    if paths.is_external_presets
        && let Some(fs_names) = fs_category_names(&paths.presets_dir.join(kind))
    {
        names.extend(fs_names);
        return names;
    }

    names.extend(embedded_category_names(kind));
    if let Some(overlay_dir) = paths.presets_overlay_dir
        && let Some(fs_names) = fs_category_names(&overlay_dir.join(kind))
    {
        names.extend(fs_names);
    }
    names
}

fn runtime_paths() -> Option<crate::config::ReadOnlyRuntimePaths> {
    crate::config::discover_runtime_paths_read_only()
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
    use crate::test_support::env_lock;
    use std::ffi::OsString;

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("shine-completion-{name}-{}", uuid::Uuid::new_v4()))
    }

    fn complete_values(args: &[&str], arg_index: usize) -> BTreeSet<String> {
        clap_complete::engine::complete(
            &mut command(),
            args.iter().map(OsString::from).collect(),
            arg_index,
            None,
        )
        .unwrap()
        .into_iter()
        .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
        .collect()
    }

    #[test]
    fn embedded_candidates_include_known_categories() {
        let _guard = env_lock();
        unsafe {
            std::env::remove_var("SHINE_CONFIG_DIR");
            std::env::remove_var("SHINE_PRESETS");
        }

        let shell = category_names("shell");
        let app = category_names("app");
        let sys = sys_item_names();

        assert!(shell.contains("proxy"), "shell candidates: {shell:?}");
        assert!(app.contains("starship"), "app candidates: {app:?}");
        assert!(sys.contains("split-dns"), "sys candidates: {sys:?}");
        let shell_info = shell_info_candidates()
            .into_iter()
            .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
            .collect::<BTreeSet<_>>();
        assert!(shell_info.contains("setproxy"));
        assert!(shell_info.contains("proxy/setproxy"));
        let build = complete_values(&["shine", "app", "artifact", "apply", ""], 4);
        assert!(build.contains("surge"));
        assert!(!build.contains("starship"));
        assert!(complete_values(&["shine", "app", "refresh", ""], 3).contains("surge"));
        assert!(complete_values(&["shine", "info", ""], 2).contains("sys/split-dns"));
        assert!(complete_values(&["shine", "info", ""], 2).contains("app/starship"));
        assert!(complete_values(&["shine", "install", ""], 2).contains("shell/proxy"));
        assert!(
            complete_values(&["shine", "install", ""], 2).contains("shell/utils/shine-env-export")
        );
        assert!(
            complete_values(&["shine", "shell", "install", ""], 3)
                .contains("utils/shine-env-export")
        );
        let preset_copy = complete_values(&["shine", "preset", "copy", ""], 3);
        assert!(preset_copy.contains("app/surge"));
        assert!(preset_copy.contains("shell/proxy"));
        assert!(preset_copy.contains("sys/macos"));
        assert!(!preset_copy.contains("surge"));
    }

    #[test]
    fn config_dir_candidates_read_external_presets_without_creating_config() {
        let _guard = env_lock();
        let dir = temp_dir("config-dir");
        std::fs::create_dir_all(dir.join("presets/shell/custom-shell")).unwrap();
        std::fs::create_dir_all(dir.join("presets/app/custom-app")).unwrap();
        std::fs::create_dir_all(dir.join("presets/sys/custom-os")).unwrap();
        std::fs::write(
            dir.join("presets/sys/custom-os/shine.toml"),
            "[[items]]\nid = \"custom-sys\"\nlabel = \"Custom sys\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("presets/shell/custom-shell/shine.toml"),
            "[[files]]\nsource = \"tool.sh\"\ntarget = \"custom-tool\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("presets/shell/custom-shell/tool.sh"),
            "#!/bin/sh\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("tasks.toml"),
            "[tasks.ship]\ncommand = [\"cargo\", \"publish\"]\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("sys-manifest.toml"),
            "[[entries]]\nitem_id = \"brew\"\n\n[[entries]]\nitem_id = \"split-dns\"\nmanaged = true\n",
        )
        .unwrap();

        unsafe {
            std::env::set_var("SHINE_CONFIG_DIR", &dir);
            std::env::remove_var("SHINE_PRESETS");
        }

        assert!(category_names("shell").contains("custom-shell"));
        assert!(category_names("app").contains("custom-app"));
        assert!(sys_item_names().contains("custom-sys"));
        let shell_info = shell_info_candidates()
            .into_iter()
            .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
            .collect::<BTreeSet<_>>();
        assert!(shell_info.contains("custom-tool"));
        assert!(shell_info.contains("custom-shell/custom-tool"));
        assert!(complete_values(&["shine", "run", ""], 2).contains("ship"));
        let sys_updates = complete_values(&["shine", "sys", "update", ""], 3);
        assert!(sys_updates.contains("brew"));
        assert!(!sys_updates.contains("split-dns"));
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
    fn overlay_candidates_extend_embedded_presets() {
        let _guard = env_lock();
        let home = temp_dir("overlay-home");
        let overlay = temp_dir("overlay");
        let old_home = std::env::var_os("HOME");
        std::fs::create_dir_all(home.join(".shine")).unwrap();
        std::fs::create_dir_all(overlay.join("shell/personal")).unwrap();
        std::fs::write(
            home.join(".shine/config.toml"),
            format!(
                "presets_overlay_dir = \"{}\"\n",
                overlay.to_string_lossy().replace('\\', "\\\\")
            ),
        )
        .unwrap();

        unsafe {
            std::env::set_var("HOME", &home);
            std::env::remove_var("SHINE_CONFIG_DIR");
            std::env::remove_var("SHINE_PRESETS");
        }

        let shell = category_names("shell");

        unsafe {
            match old_home {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
        std::fs::remove_dir_all(home).unwrap();
        std::fs::remove_dir_all(overlay).unwrap();

        assert!(shell.contains("proxy"), "shell candidates: {shell:?}");
        assert!(shell.contains("personal"), "shell candidates: {shell:?}");
    }

    #[test]
    fn completion_command_accepts_dynamic_registration_shells() {
        let _guard = env_lock();
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
