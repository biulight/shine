use super::collect::{AppInfoFile, ShellInfoFile};
use anyhow::{Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum InfoRef {
    AppCategory(String),
    AppFile { category: String, source: PathBuf },
    ShellCategory(String),
    ShellFile { category: String, command: String },
}

#[derive(Clone, Debug)]
pub(super) struct TargetCandidate {
    canonical: String,
    aliases: BTreeSet<String>,
    item: InfoRef,
}

pub(super) fn build_candidates(
    app_files: &[AppInfoFile],
    shell_files: &[ShellInfoFile],
) -> Vec<TargetCandidate> {
    let mut candidates = Vec::new();

    for category in app_files
        .iter()
        .map(|f| f.category.name.clone())
        .collect::<BTreeSet<_>>()
    {
        let mut aliases = BTreeSet::from([category.clone()]);
        aliases.insert(format!("app/{category}"));
        candidates.push(TargetCandidate {
            canonical: format!("app/{category}"),
            aliases,
            item: InfoRef::AppCategory(category),
        });
    }

    for file in app_files {
        let source = file.file.source_rel.display().to_string();
        let mut aliases =
            BTreeSet::from([format!("{}/{}", file.category.name, source), source.clone()]);
        if let Some(display_name) = &file.file.display_name {
            aliases.insert(display_name.clone());
        }
        if let Some(name) = file.destination.file_name().and_then(|n| n.to_str()) {
            aliases.insert(name.to_string());
        }
        candidates.push(TargetCandidate {
            canonical: format!("app/{}/{}", file.category.name, source),
            aliases,
            item: InfoRef::AppFile {
                category: file.category.name.clone(),
                source: file.file.source_rel.clone(),
            },
        });
    }

    for category in shell_files
        .iter()
        .map(|f| f.category.name.clone())
        .collect::<BTreeSet<_>>()
    {
        let mut aliases = BTreeSet::from([category.clone()]);
        aliases.insert(format!("shell/{category}"));
        candidates.push(TargetCandidate {
            canonical: format!("shell/{category}"),
            aliases,
            item: InfoRef::ShellCategory(category),
        });
    }

    for file in shell_files {
        let source = file.file.source_rel.display().to_string();
        let mut aliases = BTreeSet::from([
            file.file.command_name.clone(),
            source.clone(),
            format!("{}/{}", file.category.name, file.file.command_name),
        ]);
        if let Some(name) = file.file.source_rel.file_stem().and_then(|n| n.to_str()) {
            aliases.insert(name.to_string());
        }
        candidates.push(TargetCandidate {
            canonical: format!("shell/{}/{}", file.category.name, file.file.command_name),
            aliases,
            item: InfoRef::ShellFile {
                category: file.category.name.clone(),
                command: file.file.command_name.clone(),
            },
        });
    }

    candidates
}

pub(super) fn resolve_target(target: &str, candidates: &[TargetCandidate]) -> Result<Vec<InfoRef>> {
    if target.trim().is_empty() {
        bail!("info target must not be empty");
    }

    let exact: Vec<_> = candidates
        .iter()
        .filter(|c| c.canonical == target)
        .collect();
    if exact.len() == 1 {
        return Ok(vec![exact[0].item.clone()]);
    }
    if exact.len() > 1 {
        return ambiguity(target, exact);
    }

    let alias: Vec<_> = candidates
        .iter()
        .filter(|c| c.aliases.contains(target))
        .collect();
    if alias.len() == 1 {
        return Ok(vec![alias[0].item.clone()]);
    }
    if alias.len() > 1 {
        return ambiguity(target, alias);
    }

    bail!("{}", missing_target_message(target, candidates));
}

fn ambiguity(target: &str, matches: Vec<&TargetCandidate>) -> Result<Vec<InfoRef>> {
    let choices = matches
        .iter()
        .map(|c| c.canonical.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(", ");
    bail!("ambiguous info target '{target}'. Use one of: {choices}");
}

fn missing_target_message(target: &str, candidates: &[TargetCandidate]) -> String {
    let suggestions = suggested_targets(target, candidates);
    let mut message = format!("installed item not found: {target}");

    if suggestions.is_empty() {
        let available = grouped_available_targets(candidates);

        if !available.is_empty() {
            message.push_str("\n\nAvailable installed targets:");
            for (heading, targets) in available {
                message.push_str(&format!("\n  {heading}"));
                for target in targets {
                    message.push_str(&format!("\n    {target}"));
                }
            }
        }
    } else {
        message.push_str("\n\nDid you mean:");
        for target in suggestions {
            message.push_str(&format!("\n  {target}"));
        }
    }

    message.push_str("\n\nRun `shine list` to see installed configs.");
    message.push_str(
        "\nUse full targets like `app/docker-desktop/settings-store.jsonc` for exact file info.",
    );
    message
}

fn suggested_targets(target: &str, candidates: &[TargetCandidate]) -> Vec<String> {
    let needle = target.to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }

    let matches = candidates
        .iter()
        .filter(|candidate| is_suggested_target(&needle, candidate))
        .collect::<Vec<_>>();
    let matched_parents = matches
        .iter()
        .filter_map(|candidate| match &candidate.item {
            InfoRef::AppCategory(category) => Some(("app", category.as_str())),
            InfoRef::ShellCategory(category) => Some(("shell", category.as_str())),
            InfoRef::AppFile { .. } | InfoRef::ShellFile { .. } => None,
        })
        .collect::<BTreeSet<_>>();

    matches
        .into_iter()
        .filter(|candidate| !has_matched_parent(candidate, &matched_parents))
        .map(display_target_name)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn grouped_available_targets(
    candidates: &[TargetCandidate],
) -> BTreeMap<&'static str, Vec<String>> {
    let mut groups: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
    for candidate in candidates {
        match &candidate.item {
            InfoRef::AppCategory(category) => {
                groups
                    .entry("App Configs")
                    .or_default()
                    .insert(category.clone());
            }
            InfoRef::ShellCategory(category) => {
                groups
                    .entry("Shell Presets")
                    .or_default()
                    .insert(category.clone());
            }
            InfoRef::AppFile { .. } | InfoRef::ShellFile { .. } => {}
        }
    }

    groups
        .into_iter()
        .map(|(heading, targets)| (heading, targets.into_iter().collect()))
        .collect()
}

fn has_matched_parent(
    candidate: &TargetCandidate,
    matched_parents: &BTreeSet<(&str, &str)>,
) -> bool {
    match &candidate.item {
        InfoRef::AppFile { category, .. } => matched_parents.contains(&("app", category.as_str())),
        InfoRef::ShellFile { category, .. } => {
            matched_parents.contains(&("shell", category.as_str()))
        }
        InfoRef::AppCategory(_) | InfoRef::ShellCategory(_) => false,
    }
}

fn display_target_name(candidate: &TargetCandidate) -> String {
    match &candidate.item {
        InfoRef::AppCategory(category) | InfoRef::ShellCategory(category) => category.clone(),
        InfoRef::AppFile { category, source } => format!("{category}/{}", source.display()),
        InfoRef::ShellFile { category, command } => format!("{category}/{command}"),
    }
}

fn is_suggested_target(needle: &str, candidate: &TargetCandidate) -> bool {
    std::iter::once(candidate.canonical.as_str())
        .chain(candidate.aliases.iter().map(String::as_str))
        .any(|value| {
            let value = value.to_ascii_lowercase();
            value.contains(needle)
                || value
                    .split(['/', '-', '_', '.'])
                    .filter(|part| !part.is_empty())
                    .any(|part| part == needle || part.starts_with(needle))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::install_core::AppInstallStrategy;
    use crate::status::FileStatus;

    fn app_file(category: &str, source: &str, dest: &str) -> AppInfoFile {
        AppInfoFile {
            source_path: PathBuf::from(source),
            category: crate::apps::AppCategory {
                secret_files: Vec::new(),
                name: category.to_string(),
                description: None,
                destination_root: Some("~/.config".to_string()),
                files: vec![],
                list_mode: crate::apps::AppListMode::Files,
                post_upgrade: Vec::new(),
                post_install: Vec::new(),
                uses_metadata: true,
                has_explicit_files: true,
                artifact: None,
                permissions: None,
                metadata_schema_version: 2,
                metadata_is_overlay: false,
            },
            file: crate::apps::AppFile {
                source_rel: PathBuf::from(source),
                target_rel: PathBuf::from(source),
                destination_root: None,
                description: None,
                display_name: None,
                legacy_dest_annotation: None,
                transforms: vec![],
                install_strategy: AppInstallStrategy::Copy,
                requires_admin: false,
                restart_hint: None,
                generator: None,
            },
            destination: PathBuf::from(dest),
            status: FileStatus::UpToDate,
            manifest_entry: None,
            desired_content: None,
            current_content: None,
            changes: Vec::new(),
            code_analysis: None,
            assessment_error: None,
            assessment_diagnostic: None,
        }
    }

    fn shell_file(category: &str, command: &str, source: &str) -> ShellInfoFile {
        ShellInfoFile {
            category: crate::shells::metadata::ShellCategory {
                name: category.to_string(),
                description: None,
                files: vec![],
                uses_metadata: true,
            },
            file: crate::shells::metadata::ShellFile {
                source_rel: PathBuf::from(source),
                command_name: command.to_string(),
                description: vec![],
                needs_source: false,
                runtime: crate::bin_links::LinkRuntime::Native,
                transforms: vec![],
                env: vec![],
                permissions: None,
            },
            source_path: PathBuf::from(format!("/tmp/{source}")),
            installed_source_path: PathBuf::from(format!("/tmp/{source}")),
            rendered_path: PathBuf::from(format!("/tmp/rendered/{source}")),
            link_path: PathBuf::from(format!("/tmp/bin/{command}")),
            link_target: None,
            desired_content: None,
            current_content: None,
            status: "up-to-date",
            attention_required: false,
            changes: Vec::new(),
        }
    }

    #[test]
    fn resolves_shell_command_by_alias_and_full_target() {
        let candidates = build_candidates(&[], &[shell_file("proxy", "setproxy", "set_proxy.sh")]);
        for target in ["setproxy", "shell/proxy/setproxy"] {
            assert_eq!(
                resolve_target(target, &candidates).unwrap(),
                [InfoRef::ShellFile {
                    category: "proxy".into(),
                    command: "setproxy".into()
                }],
                "{target}"
            );
        }
    }

    #[test]
    fn resolves_category_alias() {
        let files = vec![app_file("git", "gitconfig", "/tmp/.gitconfig")];
        let candidates = build_candidates(&files, &[]);
        assert_eq!(
            resolve_target("git", &candidates).unwrap(),
            vec![InfoRef::AppCategory("git".to_string())]
        );
    }

    #[test]
    fn resolves_full_app_file_target() {
        let files = vec![app_file(
            "docker-desktop",
            "settings-store.jsonc",
            "/tmp/settings-store.json",
        )];
        let candidates = build_candidates(&files, &[]);
        assert_eq!(
            resolve_target("app/docker-desktop/settings-store.jsonc", &candidates).unwrap(),
            vec![InfoRef::AppFile {
                category: "docker-desktop".to_string(),
                source: PathBuf::from("settings-store.jsonc")
            }]
        );
    }

    #[test]
    fn reports_ambiguous_alias() {
        let app_files = vec![app_file("proxy", "config", "/tmp/proxy")];
        let shell_files = vec![shell_file("proxy", "setproxy", "set_proxy.sh")];
        let candidates = build_candidates(&app_files, &shell_files);
        let err = resolve_target("proxy", &candidates).unwrap_err();
        assert!(err.to_string().contains("ambiguous info target"));
        assert!(err.to_string().contains("app/proxy"));
        assert!(err.to_string().contains("shell/proxy"));
    }

    #[test]
    fn missing_target_lists_available_categories_or_matching_suggestions() {
        let app_files = vec![
            app_file(
                "docker-desktop",
                "settings-store.jsonc",
                "/tmp/settings-store.json",
            ),
            app_file("docker-engine", "daemon.jsonc", "/tmp/daemon.json"),
        ];
        let shell_files = vec![
            shell_file("agent", "ccenv", "cc.ts"),
            shell_file("proxy", "setproxy", "set_proxy.sh"),
            shell_file("utils", "copyfile", "copyfile.sh"),
        ];
        let candidates = build_candidates(&app_files, &shell_files);
        for (target, listing) in [
            (
                "missing",
                "Available installed targets:
  App Configs
    docker-desktop
    docker-engine
  Shell Presets
    agent
    proxy
    utils",
            ),
            (
                "docker",
                "Did you mean:
  docker-desktop
  docker-engine",
            ),
        ] {
            let message = resolve_target(target, &candidates).unwrap_err().to_string();
            let expected = format!(
                "installed item not found: {target}\n\n{listing}\n\nRun `shine list` to see installed configs.\nUse full targets like `app/docker-desktop/settings-store.jsonc` for exact file info."
            );
            assert_eq!(message, expected);
        }
    }
}
