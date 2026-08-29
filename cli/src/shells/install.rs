use super::links::link_conflict_render_lines;
#[cfg(test)]
use super::profile::append_path_to_shell_config;
use super::profile::{
    managed_shell_profile_path, shell_source_command, source_command_activation_hint_lines,
};
use super::report::{
    ShellUpgradeReport, link_report_summary_parts, shell_cache_summary_parts, style_bold,
    style_dim, style_green, style_symbol, style_yellow,
};
use super::{PathUpdateStatus, get_shell_config_path, metadata};
use crate::config::Config;
use crate::output;
use crate::presentation::{LifecycleReporter, PresentationEvent, TerminalRenderer};
use anyhow::{Context, Result};
use shine_core::lifecycle::{
    LifecycleEffect, LifecycleOperation, LifecycleOutcomeV1, LifecycleResultV1, LifecycleStatus,
};
use std::collections::BTreeSet;
#[cfg(test)]
use std::path::Path;

const SHELL_TEMPLATE: &str = r#"# Shell preset metadata for shine.
description = "My shell helper commands."

[[files]]
source = "my_tool.sh"
target = "mytool"
needs_source = false
# Optional: limit a file to specific platforms.
# platforms = ["macos"]    # exact: macos/linux/windows; unix groups macOS + Linux

[files.permissions]
schema_version = 1

# PowerShell scripts are also supported:
# source = "my_tool.ps1"

# Cross-platform Bun helpers (requires `bun` on PATH; shine never installs it):
# [[files]]
# source = "my_tool.ts"     # .ts / .js / .mts / .mjs
# target = "mytool"
# runtime = "bun"
# platforms = ["unix", "windows"]
# description = "What mytool does."  # or a `// ...` header at the top of my_tool.ts
# transforms = ["template"] # opt into @@VAR@@ env substitution (static, needs `shine upgrade`)
# env = ["API_URL", "SERVICE_TOKEN=API_TOKEN"]  # inject shine values at launch; read via Bun.env
# [files.permissions]
# schema_version = 1
# commands = ["bun"]
# environment = [
#   { name = "API_URL", sensitivity = "plain" },
#   { name = "SERVICE_TOKEN", sensitivity = "secret" },
# ]
"#;

pub async fn handle_init_template(force: bool) -> Result<()> {
    let dir = std::env::current_dir().context("reading current directory")?;
    let (path, overwritten) =
        shine_core::init_template::write_shine_toml_template(&dir, force, SHELL_TEMPLATE)?;
    if overwritten {
        println!("Updated shell preset template: {}", path.display());
    } else {
        println!("Created shell preset template: {}", path.display());
    }
    Ok(())
}

pub async fn handle_install(config: &Config, target: Option<&str>, force: bool) -> Result<()> {
    let mut renderer = TerminalRenderer::stdio();
    handle_install_with_reporter(config, target, force, &mut renderer)
        .await
        .map(|_| ())
}

#[cfg(test)]
pub(crate) async fn handle_install_with_result(
    config: &Config,
    target: Option<&str>,
    force: bool,
) -> Result<LifecycleResultV1> {
    let mut renderer = TerminalRenderer::stdio();
    handle_install_with_reporter(config, target, force, &mut renderer).await
}

async fn handle_install_with_reporter(
    config: &Config,
    target: Option<&str>,
    force: bool,
    reporter: &mut dyn LifecycleReporter,
) -> Result<LifecycleResultV1> {
    for line in crate::config::presets_note_lines(config) {
        reporter.emit(PresentationEvent::stdout(line));
    }
    let selection = target.map(metadata::parse_lifecycle_target).transpose()?;
    let category_filter = selection.map(|target| target.category);
    let core_report = crate::core_runtime::from_config(config)
        .await?
        .install_shells(shine_core::runtime::ShellLifecycleRequest {
            target: target.map(str::to_string),
            dry_run: false,
            force,
        })
        .await?;
    if !config.is_external_presets {
        reporter.emit(PresentationEvent::stdout(output::summary_line_text(
            "Shell Presets",
            &shell_cache_summary_parts(&core_report.cache),
        )));
    }
    if config.is_external_presets
        && config.external_shell_mode == crate::config::ExternalShellMode::Snapshot
    {
        let summary = if core_report.snapshots_updated > 0 {
            style_green(&format!("{} updated", core_report.snapshots_updated))
        } else {
            style_dim("up to date")
        };
        reporter.emit(PresentationEvent::stdout(output::summary_line_text(
            "Shell Snapshots",
            &[summary],
        )));
    }
    reporter.emit(PresentationEvent::stdout(output::summary_line_text(
        "Bin Links",
        &link_report_summary_parts(&core_report.links),
    )));
    for line in link_conflict_render_lines(config, &core_report.links.conflicts, category_filter) {
        reporter.emit(PresentationEvent::stdout(line));
    }
    let shell_config_path = get_shell_config_path(&config.shell_type, &config.home_dir)?;
    let shell_update = core_report
        .profile
        .as_ref()
        .expect("Core Shell install profile report");
    let profile_path = managed_shell_profile_path(config);
    if shell_update.profile_updated {
        reporter.emit(PresentationEvent::stdout(output::detail_line_text(
            "Shell Profile",
            &style_green("updated"),
            Some(profile_path.display().to_string()),
        )));
    }
    match &shell_update.config_status {
        PathUpdateStatus::AlreadyConfigured => {
            reporter.emit(PresentationEvent::stdout(output::detail_line_text(
                "Shell Config",
                &style_dim("up to date"),
                Some(shell_config_path.display().to_string()),
            )));
        }
        PathUpdateStatus::Updated(path) => {
            reporter.emit(PresentationEvent::stdout(output::detail_line_text(
                "Shell Config",
                &style_green("updated"),
                Some(path.display().to_string()),
            )));
        }
    }
    for line in source_command_activation_hint_lines(
        config,
        &shell_config_path,
        &core_report.source_commands,
    ) {
        reporter.emit(PresentationEvent::stdout(line));
    }

    Ok(core_report.lifecycle)
}

/// Resolve and validate a shell installation plan without extracting presets,
/// rendering templates, creating links, updating manifests, or editing shell
/// profiles.
pub async fn handle_install_dry_run(config: &Config, target: Option<&str>) -> Result<()> {
    let mut renderer = TerminalRenderer::stdio();
    handle_install_dry_run_with_reporter(config, target, &mut renderer)
        .await
        .map(|_| ())
}

async fn handle_install_dry_run_with_reporter(
    config: &Config,
    target: Option<&str>,
    reporter: &mut dyn LifecycleReporter,
) -> Result<LifecycleResultV1> {
    for line in crate::config::presets_note_lines(config) {
        reporter.emit(PresentationEvent::stdout(line));
    }
    let core_report = crate::core_runtime::from_config(config)
        .await?
        .install_shells(shine_core::runtime::ShellLifecycleRequest {
            target: target.map(str::to_string),
            dry_run: true,
            force: false,
        })
        .await?;
    for (command, target, source) in &core_report.planned_links {
        reporter.emit(PresentationEvent::stdout(format!(
            "Would link shell command {command}: {} -> {}",
            target.display(),
            source.display()
        )));
    }
    reporter.emit(PresentationEvent::stdout(
        "Dry run: no shell files, links, manifests, or profiles were changed.",
    ));
    Ok(core_report.lifecycle)
}

pub async fn handle_upgrade_installed(
    config: &Config,
    verbose: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<ShellUpgradeReport> {
    handle_upgrade_installed_with_result(config, verbose, sep)
        .await
        .map(|(report, _)| report)
}

pub(crate) async fn handle_upgrade_installed_with_result(
    config: &Config,
    verbose: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<(ShellUpgradeReport, LifecycleResultV1)> {
    handle_upgrade_installed_target_with_result(config, None, verbose, sep).await
}

pub async fn handle_upgrade_installed_target(
    config: &Config,
    category_filter: Option<&str>,
    verbose: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<ShellUpgradeReport> {
    handle_upgrade_installed_target_with_result(config, category_filter, verbose, sep)
        .await
        .map(|(report, _)| report)
}

pub(crate) async fn handle_upgrade_installed_target_with_result(
    config: &Config,
    category_filter: Option<&str>,
    verbose: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<(ShellUpgradeReport, LifecycleResultV1)> {
    let mut renderer = TerminalRenderer::stdio_with_separator(sep);
    handle_upgrade_installed_target_with_reporter(config, category_filter, verbose, &mut renderer)
        .await
}

async fn handle_upgrade_installed_target_with_reporter(
    config: &Config,
    category_filter: Option<&str>,
    verbose: bool,
    reporter: &mut dyn LifecycleReporter,
) -> Result<(ShellUpgradeReport, LifecycleResultV1)> {
    let core = crate::core_runtime::from_config(config)
        .await?
        .upgrade_shells(shine_core::runtime::ShellUpgradeRequest {
            category: category_filter.map(str::to_string),
        })
        .await?;
    if core.runs.is_empty() {
        if verbose {
            reporter.emit(PresentationEvent::stdout(style_dim(
                "No installed shell presets found.",
            )));
        }
        return Ok((ShellUpgradeReport::default(), core.lifecycle));
    }

    let snapshots_updated = core.runs.iter().map(|run| run.snapshots_updated).sum();
    let templates_updated = core
        .runs
        .iter()
        .map(|run| run.templates.updated.len())
        .sum();
    let links_created = core.runs.iter().map(|run| run.links.created.len()).sum();
    let links_updated = core
        .runs
        .iter()
        .map(|run| run.links.overwritten.len())
        .sum();
    let link_conflicts = core.runs.iter().map(|run| run.links.conflicts.len()).sum();
    let path_changed = core.runs.iter().any(|run| {
        run.profile.as_ref().is_some_and(|profile| {
            profile.profile_updated || matches!(profile.config_status, PathUpdateStatus::Updated(_))
        })
    });
    let has_visible_result = should_print_upgrade_section(
        verbose,
        !core.updated_categories.is_empty(),
        link_conflicts > 0,
        path_changed,
    );
    if has_visible_result {
        reporter.emit(PresentationEvent::SectionStart);
        if verbose {
            let installed_categories = core
                .runs
                .iter()
                .flat_map(|run| run.categories.iter().map(|category| category.name.as_str()))
                .collect::<BTreeSet<_>>()
                .len();
            reporter.emit(PresentationEvent::stdout(output::summary_line_text(
                "Shell Presets",
                &[style_dim(&format!(
                    "{installed_categories} installed categories"
                ))],
            )));
        } else {
            reporter.emit(PresentationEvent::stdout(style_bold("Shell Presets")));
        }
        for category in &core.updated_categories {
            reporter.emit(PresentationEvent::stdout(format!(
                "  {} {category}",
                style_symbol("✓")
            )));
        }
        if verbose && snapshots_updated > 0 {
            reporter.emit(PresentationEvent::stdout(format!(
                "  {} {}",
                style_symbol("✓"),
                style_green(&format!("{snapshots_updated} snapshot(s) updated"))
            )));
        }
        if verbose && templates_updated > 0 {
            reporter.emit(PresentationEvent::stdout(output::summary_line_text(
                "Templates",
                &[style_green(&format!("{templates_updated} rendered"))],
            )));
        }
        if should_print_link_summary(verbose, link_conflicts) {
            let parts = vec![
                (links_created > 0).then(|| style_green(&format!("{links_created} created"))),
                (links_updated > 0).then(|| style_green(&format!("{links_updated} updated"))),
                (link_conflicts > 0).then(|| style_yellow(&format!("{link_conflicts} conflicts"))),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
            if !parts.is_empty() {
                reporter.emit(PresentationEvent::stdout(output::summary_line_text(
                    "Bin Links",
                    &parts,
                )));
            }
        }
        for run in &core.runs {
            for line in link_conflict_render_lines(config, &run.links.conflicts, category_filter) {
                reporter.emit(PresentationEvent::stdout(line));
            }
        }
        if path_changed
            && let Some(path) = core.runs.iter().find_map(|run| {
                run.profile
                    .as_ref()
                    .and_then(|profile| match &profile.config_status {
                        PathUpdateStatus::Updated(path) => Some(path),
                        PathUpdateStatus::AlreadyConfigured => None,
                    })
            })
        {
            reporter.emit(PresentationEvent::stdout(output::detail_line_text(
                "Shell Config",
                &style_green("updated"),
                Some(path.display().to_string()),
            )));
        }
    }
    Ok((
        ShellUpgradeReport {
            updated_targets: core.updated_targets,
            updated_categories: core.updated_categories,
            snapshots_updated,
            templates_updated,
            links_created,
            links_updated,
            link_conflicts,
            path_changed,
        },
        core.lifecycle,
    ))
}
fn should_print_upgrade_section(
    verbose: bool,
    targets_updated: bool,
    has_link_conflict: bool,
    path_changed: bool,
) -> bool {
    verbose || targets_updated || has_link_conflict || path_changed
}

fn should_print_link_summary(verbose: bool, conflict_count: usize) -> bool {
    verbose || conflict_count > 0
}

pub(crate) async fn collect_update_lifecycle_result(config: &Config) -> Result<LifecycleResultV1> {
    let mut result = LifecycleResultV1::new(LifecycleOperation::Update, false);
    for row in crate::status::build_shell_rows(config)
        .await?
        .into_iter()
        .filter(|row| row.is_installed)
    {
        let mut effects = Vec::new();
        if row.changes.iter().any(|change| {
            matches!(
                change,
                crate::status::UpdateChange::ContentChanged
                    | crate::status::UpdateChange::SourceRelocated { .. }
                    | crate::status::UpdateChange::DeploymentChanged {
                        field: "snapshot",
                        ..
                    }
            )
        }) {
            effects.push(LifecycleEffect::CacheWritePreviewed);
        }
        if row.changes.iter().any(|change| {
            matches!(
                change,
                crate::status::UpdateChange::ManifestEntryMissing { .. }
            )
        }) {
            effects.push(LifecycleEffect::ReceiptWritePreviewed);
        }
        if row.changes.iter().any(|change| {
            !matches!(
                change,
                crate::status::UpdateChange::ManifestEntryMissing { .. }
            )
        }) {
            effects.push(LifecycleEffect::ResourceWritePreviewed);
        }
        let outcome = LifecycleOutcomeV1::new(
            format!(
                "shell/{}/{}",
                row.category,
                row.label.split('/').next_back().unwrap_or(&row.label)
            ),
            None::<String>,
            if row.link_conflict {
                LifecycleStatus::Conflict
            } else if row.status_sym == "↑" {
                LifecycleStatus::Pending
            } else {
                LifecycleStatus::Unchanged
            },
            if row.link_conflict {
                vec![LifecycleEffect::UserResourcePreserved]
            } else {
                effects
            },
        );
        result.push(if row.link_conflict {
            outcome.with_diagnostic_code("shell_command_conflict")
        } else {
            outcome
        });
    }
    Ok(result)
}

pub async fn handle_completion_install(config: &Config) -> Result<()> {
    let completion = crate::core_runtime::from_config(config)
        .await?
        .install_shell_completion(false)
        .await?;
    let shell_config_path = get_shell_config_path(&config.shell_type, &config.home_dir)?;
    let shell_update = completion.profile;
    let profile_path = managed_shell_profile_path(config);

    if shell_update.profile_updated {
        output::detail_line(
            "Shell Profile",
            &style_green("updated"),
            Some(profile_path.display().to_string()),
        );
    } else {
        output::detail_line(
            "Shell Profile",
            &style_dim("up to date"),
            Some(profile_path.display().to_string()),
        );
    }

    match shell_update.config_status {
        PathUpdateStatus::AlreadyConfigured => {
            output::detail_line(
                "Shell Config",
                &style_dim("up to date"),
                Some(shell_config_path.display().to_string()),
            );
        }
        PathUpdateStatus::Updated(path) => {
            output::detail_line(
                "Shell Config",
                &style_green("updated"),
                Some(path.display().to_string()),
            );
        }
    }

    if !super::profile::supports_completion_registration(&config.shell_type) {
        let shell: &'static str = config.shell_type.into();
        output::detail_line(
            "Completion",
            &style_yellow("unsupported"),
            Some(format!("{shell}; PATH setup was installed")),
        );
    }

    output::hint_line(
        "Next Step",
        &format!(
            "run `{}` once, or open a new shell",
            shell_source_command(&config.shell_type, &shell_config_path)
        ),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::ShellType;
    #[cfg(unix)]
    use super::super::uninstall::handle_uninstall;
    use super::*;
    use crate::config::Config;
    use std::path::PathBuf;
    use tokio::fs;

    #[test]
    fn upgrade_section_hides_no_op_by_default_and_shows_verbose_or_changes() {
        assert!(!should_print_upgrade_section(false, false, false, false));
        assert!(should_print_upgrade_section(true, false, false, false));
        assert!(should_print_upgrade_section(false, true, false, false));
        assert!(should_print_upgrade_section(false, false, true, false));
        assert!(should_print_upgrade_section(false, false, false, true));
    }

    #[test]
    fn bin_link_summary_is_verbose_only_unless_there_is_a_conflict() {
        assert!(!should_print_link_summary(false, 0));
        assert!(should_print_link_summary(true, 0));
        assert!(should_print_link_summary(false, 1));
    }

    async fn make_temp_dir() -> PathBuf {
        crate::test_support::make_temp_dir("shine-shell").await
    }

    #[tokio::test]
    async fn install_dry_run_does_not_materialize_shell_state() {
        let dir = make_temp_dir().await;
        let category = dir.join("presets/shell/custom");
        fs::create_dir_all(&category).await.unwrap();
        fs::write(
            category.join("shine.toml"),
            b"[[files]]\nsource = \"tool.sh\"\ntarget = \"tool\"\n",
        )
        .await
        .unwrap();
        fs::write(category.join("tool.sh"), b"#!/bin/sh\necho tool\n")
            .await
            .unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;

        handle_install_dry_run(&config, Some("custom"))
            .await
            .unwrap();

        assert!(!config.bin_dir().exists());
        assert!(!config.shine_dir().join("installed/shell").exists());
        assert!(!config.shine_dir().join("shell-manifest.toml").exists());
        assert!(!config.home_dir.join(".zshrc").exists());
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn command_scoped_install_activates_only_selected_command() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        let lifecycle = handle_install_with_result(&config, Some("utils/shine-env-export"), false)
            .await
            .unwrap();

        assert_eq!(lifecycle.outcomes.len(), 1);
        assert_eq!(lifecycle.outcomes[0].target, "shell/utils/shine-env-export");
        assert_eq!(lifecycle.outcomes[0].status, LifecycleStatus::Changed);

        let selected = crate::bin_links::command_path_for_name(
            config.bin_dir(),
            std::ffi::OsStr::new("shine-env-export"),
        );
        let sibling = crate::bin_links::command_path_for_name(
            config.bin_dir(),
            std::ffi::OsStr::new("shine-theme-sync"),
        );
        assert!(selected.exists());
        assert!(!sibling.exists());

        let manifest =
            crate::shells::deployment::ShellManifest::load(&shine_core::runtime::RealHost, &config)
                .await
                .unwrap();
        assert!(manifest.find("shell/utils/shine-env-export").is_some());
        assert!(manifest.find("shell/utils/shine-theme-sync").is_none());

        let rows = crate::status::build_shell_rows(&config).await.unwrap();
        let selected_row = rows
            .iter()
            .find(|row| row.label == "utils/shine-env-export")
            .unwrap();
        let sibling_row = rows
            .iter()
            .find(|row| row.label == "utils/shine-theme-sync")
            .unwrap();
        assert!(selected_row.is_installed);
        assert!(!sibling_row.is_installed);
        assert_eq!(sibling_row.status_text, "not installed");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn command_scoped_install_preserves_sibling_manifest_entries() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("utils/shine-env-export"), false)
            .await
            .unwrap();
        handle_install(&config, Some("utils/shine-theme-sync"), false)
            .await
            .unwrap();

        let manifest =
            crate::shells::deployment::ShellManifest::load(&shine_core::runtime::RealHost, &config)
                .await
                .unwrap();
        assert!(manifest.find("shell/utils/shine-env-export").is_some());
        assert!(manifest.find("shell/utils/shine-theme-sync").is_some());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn command_scoped_install_rejects_unknown_targets_before_writing() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);

        let error = handle_install(&config, Some("utils/not-a-command"), false)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("shell preset command not found: utils/not-a-command"));
        assert!(!config.bin_dir().exists());
        assert!(!config.presets_dir().join("shell/utils").exists());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn future_manifest_rejects_install_before_shell_mutation() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::write(
            config.shine_dir().join("shell-manifest.toml"),
            "schema_version = 2\nentries = []\n",
        )
        .await
        .unwrap();

        let error = handle_install(&config, Some("utils/shine-env-export"), false)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("newer than this Shine supports"));
        assert!(!config.presets_dir().join("shell/utils").exists());
        assert!(!config.bin_dir().exists());
        assert!(!config.home_dir.join(".zshrc").exists());
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn category_upgrade_repairs_only_installed_commands() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        handle_install(&config, Some("utils/shine-env-export"), false)
            .await
            .unwrap();
        let selected = crate::bin_links::command_path_for_name(
            config.bin_dir(),
            std::ffi::OsStr::new("shine-env-export"),
        );
        let sibling = crate::bin_links::command_path_for_name(
            config.bin_dir(),
            std::ffi::OsStr::new("shine-theme-sync"),
        );
        shine_core::runtime::unlink_managed_command_with_host(
            &shine_core::runtime::RealHost,
            config.bin_dir(),
            std::ffi::OsStr::new("shine-env-export"),
            &[config.presets_dir().join("shell/utils")],
            false,
        )
        .await
        .unwrap();

        let pending = collect_update_lifecycle_result(&config).await.unwrap();
        let selected_pending = pending
            .outcomes
            .iter()
            .find(|outcome| outcome.target == "shell/utils/shine-env-export")
            .unwrap();
        assert_eq!(selected_pending.status, LifecycleStatus::Pending);
        assert!(
            selected_pending
                .effects
                .contains(&LifecycleEffect::ResourceWritePreviewed)
        );

        let mut separator = crate::output::SectionSeparator::new();
        handle_upgrade_installed_target(&config, Some("utils"), false, &mut separator)
            .await
            .unwrap();

        assert!(selected.exists());
        assert!(!sibling.exists());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn external_snapshot_is_shared_but_only_selected_command_is_installed() {
        let dir = make_temp_dir().await;
        let category = dir.join("presets/shell/custom");
        fs::create_dir_all(&category).await.unwrap();
        fs::write(
            category.join("shine.toml"),
            b"[[files]]\nsource = \"one.sh\"\ntarget = \"one\"\n\n[[files]]\nsource = \"two.sh\"\ntarget = \"two\"\n",
        )
        .await
        .unwrap();
        fs::write(category.join("one.sh"), b"#!/bin/sh\necho one\n")
            .await
            .unwrap();
        fs::write(category.join("two.sh"), b"#!/bin/sh\necho two\n")
            .await
            .unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("custom/one"), false)
            .await
            .unwrap();

        assert!(config.installed_shell_dir().join("custom/one.sh").exists());
        assert!(config.installed_shell_dir().join("custom/two.sh").exists());
        assert!(
            crate::bin_links::command_path_for_name(config.bin_dir(), std::ffi::OsStr::new("one"),)
                .exists()
        );
        assert!(
            !crate::bin_links::command_path_for_name(
                config.bin_dir(),
                std::ffi::OsStr::new("two"),
            )
            .exists()
        );
        let rows = crate::status::build_shell_rows(&config).await.unwrap();
        let sibling = rows.iter().find(|row| row.label == "custom/two").unwrap();
        assert!(!sibling.is_installed);
        assert_eq!(sibling.status_text, "not installed");
        assert!(sibling.changes.is_empty());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn structured_snapshot_lifecycle_covers_update_upgrade_and_uninstall() {
        let dir = make_temp_dir().await;
        let category = dir.join("presets/shell/custom");
        fs::create_dir_all(&category).await.unwrap();
        fs::write(
            category.join("shine.toml"),
            b"[[files]]\nsource = \"one.sh\"\ntarget = \"one\"\n\n[[files]]\nsource = \"two.sh\"\ntarget = \"two\"\n",
        )
        .await
        .unwrap();
        fs::write(category.join("one.sh"), b"#!/bin/sh\necho one\n")
            .await
            .unwrap();
        fs::write(category.join("two.sh"), b"#!/bin/sh\necho two\n")
            .await
            .unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        let install = handle_install_with_result(&config, Some("custom/one"), false)
            .await
            .unwrap();
        assert!(install.outcomes.iter().any(|outcome| {
            outcome.target == "shell/custom/one" && outcome.status == LifecycleStatus::Changed
        }));
        let sibling =
            crate::bin_links::command_path_for_name(config.bin_dir(), std::ffi::OsStr::new("two"));
        assert!(!sibling.exists());
        assert!(config.installed_shell_dir().join("custom/two.sh").exists());

        fs::write(category.join("one.sh"), b"#!/bin/sh\necho updated\n")
            .await
            .unwrap();
        let update = collect_update_lifecycle_result(&config).await.unwrap();
        let pending = update
            .outcomes
            .iter()
            .find(|outcome| outcome.target == "shell/custom/one")
            .unwrap();
        assert_eq!(pending.status, LifecycleStatus::Pending);
        assert!(
            pending
                .effects
                .contains(&LifecycleEffect::CacheWritePreviewed)
        );

        let mut separator = crate::output::SectionSeparator::new();
        let (report, upgrade) = handle_upgrade_installed_target_with_result(
            &config,
            Some("custom"),
            false,
            &mut separator,
        )
        .await
        .unwrap();
        assert_eq!(report.updated_targets, ["custom/one"]);
        assert!(upgrade.outcomes.iter().any(|outcome| {
            outcome.target == "shell/custom/one" && outcome.status == LifecycleStatus::Changed
        }));
        assert!(!sibling.exists());
        assert_eq!(
            fs::read(category.join("one.sh")).await.unwrap(),
            b"#!/bin/sh\necho updated\n"
        );

        let current = collect_update_lifecycle_result(&config).await.unwrap();
        assert!(current.outcomes.iter().any(|outcome| {
            outcome.target == "shell/custom/one" && outcome.status == LifecycleStatus::Unchanged
        }));

        let uninstall = super::super::uninstall::handle_uninstall_with_result(
            &config,
            Some("custom/one"),
            false,
            false,
        )
        .await
        .unwrap();
        assert!(uninstall.outcomes.iter().any(|outcome| {
            outcome.target == "shell/custom/one" && outcome.status == LifecycleStatus::Changed
        }));
        assert!(!config.installed_shell_dir().join("custom").exists());
        assert!(category.join("one.sh").exists());
        assert!(category.join("two.sh").exists());
        assert!(!sibling.exists());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    async fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path).await.unwrap().permissions();
        perms.set_mode(perms.mode() | 0o111);
        fs::set_permissions(path, perms).await.unwrap();
    }

    fn wrapper_marker(command: &str, shell: &ShellType) -> String {
        match shell {
            ShellType::PowerShell => format!("\nfunction {command} {{ . (Join-Path $shineBin"),
            ShellType::Fish => format!("\nfunction {command}"),
            _ => format!("\n{command}() {{ source"),
        }
    }

    #[cfg(unix)]
    fn managed_profile_source_marker(shell: &ShellType) -> &'static str {
        match shell {
            ShellType::PowerShell => ". (Join-Path $HOME 'shell/profile.ps1')",
            ShellType::Fish => "source \"$HOME/shell/config.fish\"",
            ShellType::Bash | ShellType::Zsh | ShellType::Elvish => {
                "source \"$HOME/shell/profile.sh\""
            }
        }
    }

    #[cfg(unix)]
    fn managed_profile_path_marker(shell: &ShellType) -> &'static str {
        match shell {
            ShellType::PowerShell => "$shinePathEntries",
            ShellType::Fish => "fish_add_path",
            ShellType::Bash | ShellType::Zsh | ShellType::Elvish => "export PATH",
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn install_then_uninstall_roundtrip() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, None, false).await.unwrap();
        assert!(
            config
                .presets_dir()
                .join("shell/proxy/set_proxy.sh")
                .exists(),
            "preset should exist after install"
        );
        let first_bin_entry = fs::read_dir(config.bin_dir())
            .await
            .unwrap()
            .next_entry()
            .await
            .unwrap();
        assert!(
            first_bin_entry.is_some(),
            "bin dir should have symlinks after install"
        );
        // symlinks use stem names (no .sh suffix)
        assert!(
            config.bin_dir().join("setproxy").exists(),
            "bin link should use configured rename"
        );
        assert!(!config.bin_dir().join("set_proxy").exists());
        assert!(
            managed_shell_profile_path(&config).exists(),
            "managed shell profile should exist after install"
        );

        handle_uninstall(&config, None, false, false).await.unwrap();
        assert!(
            !config
                .presets_dir()
                .join("shell/proxy/set_proxy.sh")
                .exists(),
            "preset should be gone after uninstall"
        );
        let mut rd = fs::read_dir(config.bin_dir()).await.unwrap();
        assert!(
            rd.next_entry().await.unwrap().is_none(),
            "bin dir should be empty after uninstall"
        );
        assert!(
            !managed_shell_profile_path(&config).exists(),
            "managed shell profile should be removed after full uninstall"
        );

        // Idempotency: second uninstall must not error
        handle_uninstall(&config, None, false, false).await.unwrap();

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn append_writes_snippet_to_shell_config() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);

        append_path_to_shell_config(&config, false, &[])
            .await
            .unwrap();

        let config_path = get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        let content = fs::read_to_string(&config_path).await.unwrap();
        assert!(
            content.contains(super::super::SENTINEL_START),
            "sentinel should be present"
        );
    }

    #[tokio::test]
    async fn completion_install_updates_profile_without_installing_presets() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);

        handle_completion_install(&config).await.unwrap();

        let profile = fs::read_to_string(managed_shell_profile_path(&config))
            .await
            .unwrap();
        let completion_marker = match config.shell_type {
            ShellType::Bash => "COMPLETE=bash shine",
            ShellType::Zsh => "COMPLETE=zsh shine",
            ShellType::PowerShell => "$env:COMPLETE = 'powershell'",
            ShellType::Fish | ShellType::Elvish => {
                panic!("native default shell should support completion registration")
            }
        };
        assert!(
            profile.contains(completion_marker),
            "profile should register shine completion: {profile}"
        );
        assert!(
            !config.presets_dir().join("shell/proxy").exists(),
            "completion install must not extract or install shell presets"
        );
    }

    #[tokio::test]
    async fn append_is_idempotent() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);

        append_path_to_shell_config(&config, false, &[])
            .await
            .unwrap();
        append_path_to_shell_config(&config, false, &[])
            .await
            .unwrap();

        let config_path = get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        let content = fs::read_to_string(&config_path).await.unwrap();
        let count = content.matches(super::super::SENTINEL_START).count();
        assert_eq!(count, 1, "sentinel should appear exactly once");
    }

    #[tokio::test]
    async fn append_is_idempotent_with_source_wrappers() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        let source_commands = vec!["setproxy".to_string(), "usetproxy".to_string()];

        append_path_to_shell_config(&config, false, &source_commands)
            .await
            .unwrap();
        append_path_to_shell_config(&config, false, &source_commands)
            .await
            .unwrap();

        let config_path = get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        let content = fs::read_to_string(&config_path).await.unwrap();
        assert_eq!(
            content.matches(super::super::SENTINEL_START).count(),
            1,
            "sentinel should appear exactly once"
        );
        assert!(
            !content.contains("setproxy()"),
            "source wrappers should live in the managed profile: {content}"
        );

        let profile_path = managed_shell_profile_path(&config);
        let profile = fs::read_to_string(&profile_path).await.unwrap();
        let setproxy_marker = wrapper_marker("setproxy", &config.shell_type);
        let usetproxy_marker = wrapper_marker("usetproxy", &config.shell_type);
        assert_eq!(
            profile.matches(&setproxy_marker).count(),
            1,
            "setproxy wrapper should not be duplicated: {content}"
        );
        assert_eq!(
            profile.matches(&usetproxy_marker).count(),
            1,
            "usetproxy wrapper should not be duplicated: {content}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn append_writes_source_entry_and_managed_profile() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        let source_commands = vec!["setproxy".to_string()];

        append_path_to_shell_config(&config, false, &source_commands)
            .await
            .unwrap();

        let config_path = get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        let content = fs::read_to_string(&config_path).await.unwrap();
        assert!(
            content.contains(managed_profile_source_marker(&config.shell_type)),
            "shell config should only source managed profile: {content}"
        );
        assert!(
            !content.contains("export PATH"),
            "shell config should not contain direct PATH setup: {content}"
        );
        assert!(
            !content.contains("setproxy()"),
            "shell config should not contain direct wrapper functions: {content}"
        );

        let profile = fs::read_to_string(managed_shell_profile_path(&config))
            .await
            .unwrap();
        assert!(
            profile.contains(managed_profile_path_marker(&config.shell_type)),
            "managed profile should contain PATH setup: {profile}"
        );
        assert!(
            profile.contains(&wrapper_marker("setproxy", &config.shell_type)),
            "managed profile should contain source wrapper: {profile}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn append_writes_both_windows_powershell_profiles() {
        let dir = make_temp_dir().await;
        let mut config = Config::new_for_test(&dir);
        config.shell_type = ShellType::PowerShell;
        let source_commands = vec!["setproxy".to_string(), "usetproxy".to_string()];

        append_path_to_shell_config(&config, false, &source_commands)
            .await
            .unwrap();

        let profile = fs::read_to_string(managed_shell_profile_path(&config))
            .await
            .unwrap();
        for config_path in
            super::super::get_shell_config_paths(&config.shell_type, &config.home_dir).unwrap()
        {
            let content = fs::read_to_string(&config_path).await.unwrap();
            assert!(
                content.contains(". (Join-Path $HOME 'shell/profile.ps1')"),
                "PowerShell profile should source managed shine profile from {}: {content}",
                config_path.display()
            );
        }
        assert!(
            profile.contains("function setproxy"),
            "managed PowerShell profile should contain setproxy wrapper: {profile}"
        );
        assert!(
            profile.contains("function usetproxy"),
            "managed PowerShell profile should contain usetproxy wrapper: {profile}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn append_refreshes_stale_sentinel_with_managed_profile_source() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        let config_path = get_shell_config_path(&config.shell_type, &config.home_dir).unwrap();
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).await.unwrap();
        }
        let sentinel_start = super::super::SENTINEL_START;
        let sentinel_end = "# <<< shine <<<";
        fs::write(
            &config_path,
            format!(
                "before\n\n{sentinel_start}\nif [[ \":$PATH:\" != *\":$HOME/.shine/bin:\"* ]]; then\n  export PATH=\"$HOME/.shine/bin:$PATH\"\nfi\n{sentinel_end}\nafter\n"
            ),
        )
        .await
        .unwrap();

        let source_commands = vec!["setproxy".to_string(), "usetproxy".to_string()];
        let update = append_path_to_shell_config(&config, false, &source_commands)
            .await
            .unwrap();

        assert!(
            matches!(update.config_status, PathUpdateStatus::Updated(_)),
            "stale sentinel should be refreshed"
        );
        let content = fs::read_to_string(&config_path).await.unwrap();
        assert!(
            content.contains(managed_profile_source_marker(&config.shell_type)),
            "shell config should source managed profile: {content}"
        );
        assert!(
            !content.contains("export PATH"),
            "stale PATH setup should be removed from shell config: {content}"
        );
        assert!(
            !content.contains("setproxy()"),
            "source wrappers should not be added directly to shell config: {content}"
        );
        assert!(
            content.contains("before"),
            "non-managed content should be preserved"
        );
        assert!(
            content.contains("after"),
            "non-managed content should be preserved"
        );
        let profile = fs::read_to_string(managed_shell_profile_path(&config))
            .await
            .unwrap();
        let setproxy_marker = wrapper_marker("setproxy", &config.shell_type);
        let usetproxy_marker = wrapper_marker("usetproxy", &config.shell_type);
        assert!(
            profile.contains(&setproxy_marker),
            "setproxy wrapper should be added to managed profile: {profile}"
        );
        assert!(
            profile.contains(&usetproxy_marker),
            "usetproxy wrapper should be added to managed profile: {profile}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn installed_source_commands_for_categories_are_scoped() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("agent"), false).await.unwrap();
        handle_install(&config, Some("proxy"), false).await.unwrap();

        let commands = crate::core_runtime::from_config(&config)
            .await
            .unwrap()
            .installed_shell_source_commands(Some("proxy"))
            .await
            .unwrap();

        assert_eq!(
            commands,
            vec!["setproxy".to_string(), "usetproxy".to_string()]
        );
        assert!(!commands.contains(&"ccenv".to_string()));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_presets_install_links_disk_scripts_without_extraction() {
        let dir = make_temp_dir().await;
        // new_for_test sets presets_dir = dir/presets, bin_dir = dir/bin
        // Create a script in presets_dir/shell/custom/ to simulate user-managed presets.
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        let script = cat_dir.join("my_tool.sh");
        fs::write(&script, b"#!/bin/bash\n# My tool.\necho hi\n")
            .await
            .unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).await.unwrap().permissions();
        perms.set_mode(perms.mode() | 0o111);
        fs::set_permissions(&script, perms).await.unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("custom"), false)
            .await
            .unwrap();

        // The script must NOT have been extracted from embedded assets into
        // presets_dir — the only file there is the one we created above.
        let count = {
            let mut rd = fs::read_dir(&cat_dir).await.unwrap();
            let mut n = 0u32;
            while rd.next_entry().await.unwrap().is_some() {
                n += 1;
            }
            n
        };
        assert_eq!(count, 1, "no embedded assets should have been extracted");

        // A bin symlink for the script should have been created.
        let link = config.bin_dir().join("my_tool");
        assert!(link.exists(), "bin symlink should point at disk script");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_presets_install_applies_metadata_rename() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"set_proxy.sh\"\ntarget = \"setproxy\"\n",
        )
        .await
        .unwrap();
        let script = cat_dir.join("set_proxy.sh");
        fs::write(&script, b"#!/bin/bash\n# Set proxy.\necho hi\n")
            .await
            .unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).await.unwrap().permissions();
        perms.set_mode(perms.mode() | 0o111);
        fs::set_permissions(&script, perms).await.unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("custom"), false)
            .await
            .unwrap();

        assert!(config.bin_dir().join("setproxy").exists());
        assert!(!config.bin_dir().join("set_proxy").exists());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_presets_install_links_non_executable_source_scripts() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/proxy");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"set_proxy.sh\"\ntarget = \"setproxy\"\nneeds_source = true\n[[files]]\nsource = \"uset_proxy.sh\"\ntarget = \"usetproxy\"\nneeds_source = true\n",
        )
        .await
        .unwrap();
        fs::write(
            &cat_dir.join("set_proxy.sh"),
            b"#!/bin/bash\n# Set proxy.\n",
        )
        .await
        .unwrap();
        fs::write(
            &cat_dir.join("uset_proxy.sh"),
            b"#!/bin/bash\n# Unset proxy.\n",
        )
        .await
        .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("proxy"), false).await.unwrap();

        assert!(config.bin_dir().join("setproxy").exists());
        assert!(config.bin_dir().join("usetproxy").exists());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn init_template_creates_parseable_shell_metadata() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();

        let (path, overwritten) =
            shine_core::init_template::write_shine_toml_template(&cat_dir, false, SHELL_TEMPLATE)
                .unwrap();
        fs::write(
            cat_dir.join("my_tool.sh"),
            b"#!/bin/bash\n# My tool.\necho hi\n",
        )
        .await
        .unwrap();

        let config = Config::new_for_test(&dir);
        let categories = metadata::load_installed_categories(&config, Some("custom"))
            .await
            .unwrap();

        assert_eq!(path, cat_dir.join("shine.toml"));
        assert!(!overwritten);
        assert_eq!(categories.len(), 1);
        assert_eq!(
            categories[0].description.as_deref(),
            Some("My shell helper commands.")
        );
        assert_eq!(
            categories[0].files[0].source_rel,
            PathBuf::from("my_tool.sh")
        );
        assert_eq!(categories[0].files[0].command_name, "mytool");
        assert!(!categories[0].files[0].needs_source);
        assert_eq!(
            categories[0].files[0]
                .permissions
                .as_ref()
                .map(|permissions| permissions.schema_version),
            Some(1)
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn init_template_refuses_existing_file_unless_forced() {
        let dir = make_temp_dir().await;
        fs::write(dir.join("shine.toml"), b"old").await.unwrap();

        let err = shine_core::init_template::write_shine_toml_template(&dir, false, SHELL_TEMPLATE)
            .unwrap_err();
        assert!(
            err.to_string().contains("use --force to overwrite"),
            "unexpected error: {err:#}"
        );
        assert_eq!(fs::read(dir.join("shine.toml")).await.unwrap(), b"old");

        let (_path, overwritten) =
            shine_core::init_template::write_shine_toml_template(&dir, true, SHELL_TEMPLATE)
                .unwrap();
        assert!(overwritten);
        let content = fs::read_to_string(dir.join("shine.toml")).await.unwrap();
        assert!(content.contains("target = \"mytool\""));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn template_render_error_does_not_link_raw_script() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/proxy");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"set_proxy.sh\"\ntarget = \"setproxy\"\nneeds_source = true\n",
        )
        .await
        .unwrap();
        let script = cat_dir.join("set_proxy.sh");
        fs::write(
            &script,
            b"#!/bin/bash\n# shine-template: true\necho @@PROXY_HOST@@\n",
        )
        .await
        .unwrap();
        make_executable(&script).await;

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        fs::write(config.rendered_dir(), b"not a directory")
            .await
            .unwrap();

        let err = handle_install(&config, Some("proxy"), false)
            .await
            .expect_err("install should fail when rendered_dir cannot be created");

        assert!(
            err.to_string()
                .contains("creating rendered script directory"),
            "unexpected error: {err:#}"
        );
        assert!(
            !config.bin_dir().join("setproxy").exists(),
            "failed render must not link the raw template script"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn embedded_agent_installs_bun_launcher_without_rendering_credentials() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("agent"), false).await.unwrap();

        let source = config.presets_dir().join("shell/agent/cc.ts");
        assert!(source.exists());
        assert!(!config.rendered_dir().join("shell/agent/cc.ts").exists());
        let launcher = crate::bin_links::command_path_for_name(
            config.bin_dir(),
            std::ffi::OsStr::new("ccenv"),
        );
        let launcher_content = fs::read_to_string(&launcher).await.unwrap();
        assert!(launcher_content.contains("shine-managed"));
        let recorded_target = launcher_content
            .lines()
            .find_map(|line| line.strip_prefix("# shine-target: "))
            .expect("launcher should record its source target");
        assert_eq!(
            fs::canonicalize(recorded_target).await.unwrap(),
            fs::canonicalize(&source).await.unwrap()
        );
        assert!(launcher_content.contains("bun"));

        let source_commands = crate::core_runtime::from_config(&config)
            .await
            .unwrap()
            .installed_shell_source_commands(None)
            .await
            .unwrap();
        assert!(!source_commands.contains(&"ccenv".to_string()));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn embedded_source_and_link_upgrade_report_target_once() {
        let dir = make_temp_dir().await;
        let config = Config::new_for_test(&dir);
        fs::create_dir_all(config.presets_dir()).await.unwrap();
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        handle_install(&config, Some("utils"), false).await.unwrap();

        let source = config.presets_dir().join("shell/utils/copyfile.sh");
        fs::write(&source, b"#!/bin/sh\necho stale\n")
            .await
            .unwrap();
        make_executable(&source).await;

        let stale_source = dir.join("stale-copyfile.sh");
        fs::write(&stale_source, b"#!/bin/sh\necho stale link\n")
            .await
            .unwrap();
        make_executable(&stale_source).await;
        let link = config.bin_dir().join("copyfile");
        fs::remove_file(&link).await.unwrap();
        fs::symlink(&stale_source, &link).await.unwrap();

        let mut separator = crate::output::SectionSeparator::new();
        let report = handle_upgrade_installed(&config, false, &mut separator)
            .await
            .unwrap();

        assert_eq!(report.updated_targets, vec!["utils/copyfile"]);
        assert_eq!(report.updated_categories, vec!["utils"]);
        assert_eq!(report.links_updated, 1);
        assert_eq!(fs::read_link(&link).await.unwrap(), source);
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_presets_upgrade_does_not_install_preset_only_scripts() {
        let dir = make_temp_dir().await;
        let proxy_dir = dir.join("presets/shell/proxy");
        let extra_dir = dir.join("presets/shell/extra");
        fs::create_dir_all(&proxy_dir).await.unwrap();
        fs::create_dir_all(&extra_dir).await.unwrap();

        fs::write(
            proxy_dir.join("shine.toml"),
            b"[[files]]\nsource = \"set_proxy.sh\"\ntarget = \"setproxy\"\nneeds_source = true\n",
        )
        .await
        .unwrap();
        let setproxy = proxy_dir.join("set_proxy.sh");
        fs::write(
            &setproxy,
            b"#!/bin/bash\n# shine-template: true\necho @@PROXY_HOST@@\n",
        )
        .await
        .unwrap();
        make_executable(&setproxy).await;

        let extra_tool = extra_dir.join("extra_tool.sh");
        fs::write(&extra_tool, b"#!/bin/bash\n# Extra tool.\necho extra\n")
            .await
            .unwrap();
        make_executable(&extra_tool).await;

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("proxy"), false).await.unwrap();
        assert!(config.bin_dir().join("setproxy").exists());
        assert!(
            !config.bin_dir().join("extra_tool").exists(),
            "extra preset should start as present but not installed"
        );

        fs::write(
            &setproxy,
            b"#!/bin/bash\n# shine-template: true\necho changed @@PROXY_HOST@@\n",
        )
        .await
        .unwrap();
        make_executable(&setproxy).await;

        let mut sep = crate::output::SectionSeparator::new();
        let report = handle_upgrade_installed(&config, false, &mut sep)
            .await
            .unwrap();

        assert_eq!(
            report.templates_updated, 1,
            "changed shell template should be reported under shell presets"
        );
        assert_eq!(report.updated_targets, vec!["proxy/setproxy"]);
        assert_eq!(report.updated_categories, vec!["proxy"]);
        assert!(config.bin_dir().join("setproxy").exists());
        assert!(
            !config.bin_dir().join("extra_tool").exists(),
            "upgrade must not install preset-only scripts"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_bun_preset_installs_launcher_and_uninstall_removes_it() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"tool.ts\"\ntarget = \"mytool\"\nruntime = \"bun\"\n",
        )
        .await
        .unwrap();
        // A non-executable .ts source: bun launchers do not require the exec bit.
        fs::write(cat_dir.join("tool.ts"), b"console.log('hi')\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("custom"), false)
            .await
            .unwrap();

        let launcher = config.bin_dir().join("mytool");
        assert!(launcher.exists(), "bun launcher should be installed");
        assert!(!launcher.is_symlink(), "bun launcher is a regular file");
        let content = fs::read_to_string(&launcher).await.unwrap();
        assert!(content.contains("exec bun --no-install"));
        assert!(
            content.contains(
                &config
                    .installed_shell_dir()
                    .join("custom/tool.ts")
                    .display()
                    .to_string()
            )
        );
        assert!(
            !config.bin_dir().join("tool").exists(),
            "command should use the target rename, not the .ts stem"
        );

        handle_uninstall(&config, Some("custom"), false, false)
            .await
            .unwrap();
        assert!(
            !launcher.exists(),
            "managed bun launcher must be removed on uninstall"
        );
        assert!(
            cat_dir.join("tool.ts").exists(),
            "external source must be preserved"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_bun_preset_with_env_wraps_launcher_in_shine_env_run() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"tool.ts\"\ntarget = \"mytool\"\nruntime = \"bun\"\nenv = [\"API_URL\", \"SERVICE_TOKEN=API_TOKEN\"]\n",
        )
        .await
        .unwrap();
        fs::write(cat_dir.join("tool.ts"), b"console.log(Bun.env.API_URL)\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("custom"), false)
            .await
            .unwrap();

        let launcher = fs::read_to_string(config.bin_dir().join("mytool"))
            .await
            .unwrap();
        assert!(launcher.contains("command -v shine"));
        assert!(launcher.contains(
            "exec shine env run --no-workspace --with 'API_URL' --with 'SERVICE_TOKEN=API_TOKEN' -- bun --no-install "
        ));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_bun_preset_with_locked_package_uses_fallback_and_records_hash() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"tool.ts\"\ntarget = \"mytool\"\nruntime = \"bun\"\n",
        )
        .await
        .unwrap();
        fs::write(cat_dir.join("tool.ts"), b"import 'zod'\n")
            .await
            .unwrap();
        fs::write(
            cat_dir.join("package.json"),
            b"{\"dependencies\":{\"zod\":\"4.0.0\"}}",
        )
        .await
        .unwrap();
        fs::write(cat_dir.join("bun.lock"), b"lockfileVersion = 1\n")
            .await
            .unwrap();
        fs::create_dir_all(cat_dir.join("node_modules/zod"))
            .await
            .unwrap();
        fs::write(cat_dir.join("node_modules/zod/index.js"), b"export {}")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        handle_install(&config, Some("custom"), false)
            .await
            .unwrap();

        let launcher = fs::read_to_string(config.bin_dir().join("mytool"))
            .await
            .unwrap();
        assert!(launcher.contains("exec bun --install=fallback"));
        assert!(
            !config
                .installed_shell_dir()
                .join("custom/node_modules")
                .exists()
        );
        let manifest =
            crate::shells::deployment::ShellManifest::load(&shine_core::runtime::RealHost, &config)
                .await
                .unwrap();
        let entry = manifest.find("shell/custom/mytool").unwrap();
        assert_eq!(entry.bun_dependencies.as_deref(), Some("locked"));
        assert!(entry.dependency_hash.is_some());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn external_bun_preset_with_template_transform_targets_rendered_copy() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"tool.ts\"\ntarget = \"mytool\"\nruntime = \"bun\"\ntransforms = [\"template\"]\n",
        )
        .await
        .unwrap();
        fs::write(cat_dir.join("tool.ts"), b"const host = '@@PROXY_HOST@@'\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config
            .env
            .insert("PROXY_HOST".into(), "proxy.example".into());
        fs::create_dir_all(config.bin_dir()).await.unwrap();

        handle_install(&config, Some("custom"), false)
            .await
            .unwrap();

        let rendered = config.rendered_dir().join("shell/custom/tool.ts");
        assert!(
            rendered.exists(),
            "template transform should render the .ts"
        );
        assert!(
            fs::read_to_string(&rendered)
                .await
                .unwrap()
                .contains("proxy.example"),
            "rendered bun script should have @@PROXY_HOST@@ substituted"
        );
        let launcher = fs::read_to_string(config.bin_dir().join("mytool"))
            .await
            .unwrap();
        assert!(
            launcher.contains(&rendered.display().to_string()),
            "launcher must target the rendered copy: {launcher}"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn live_transformed_bun_renders_again_on_demand() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"tool.ts\"\ntarget = \"mytool\"\nruntime = \"bun\"\ntransforms = [\"template\"]\n",
        )
        .await
        .unwrap();
        let source = cat_dir.join("tool.ts");
        fs::write(&source, b"console.log('@@PROXY_HOST@@')\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        config.external_shell_mode = crate::config::ExternalShellMode::Live;
        config
            .env
            .insert("PROXY_HOST".into(), "first.example".into());
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        handle_install(&config, Some("custom"), false)
            .await
            .unwrap();

        let rendered = config.rendered_dir().join("shell/custom/tool.ts");
        assert!(
            fs::read_to_string(&rendered)
                .await
                .unwrap()
                .contains("first.example")
        );
        config
            .env
            .insert("PROXY_HOST".into(), "second.example".into());
        crate::shells::deployment::handle_render_live(&config, "shell/custom/mytool")
            .await
            .unwrap();
        assert!(
            fs::read_to_string(&rendered)
                .await
                .unwrap()
                .contains("second.example")
        );
        let last_good = fs::read(&rendered).await.unwrap();
        fs::write(&source, b"console.log('@@MISSING_LIVE_VALUE@@')\n")
            .await
            .unwrap();
        assert!(
            crate::shells::deployment::handle_render_live(&config, "shell/custom/mytool")
                .await
                .is_err()
        );
        assert_eq!(
            fs::read(&rendered).await.unwrap(),
            last_good,
            "failed live transform must preserve the last-known-good output"
        );

        let launcher = fs::read_to_string(config.bin_dir().join("mytool"))
            .await
            .unwrap();
        assert!(launcher.contains("__shell-render"));
        assert!(launcher.contains("--config-dir"));
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn snapshot_upgrade_applies_external_raw_source_change() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"tool.sh\"\ntarget = \"mytool\"\n",
        )
        .await
        .unwrap();
        let source = cat_dir.join("tool.sh");
        fs::write(&source, b"#!/bin/sh\necho first\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        handle_install(&config, Some("custom"), false)
            .await
            .unwrap();
        let installed = config.installed_shell_dir().join("custom/tool.sh");
        assert!(
            fs::read_to_string(&installed)
                .await
                .unwrap()
                .contains("first")
        );

        fs::write(&source, b"#!/bin/sh\necho second\n")
            .await
            .unwrap();
        let mut separator = crate::output::SectionSeparator::new();
        let report = handle_upgrade_installed(&config, false, &mut separator)
            .await
            .unwrap();
        assert_eq!(report.snapshots_updated, 1);
        assert_eq!(report.updated_targets, vec!["custom/mytool"]);
        assert_eq!(report.updated_categories, vec!["custom"]);
        assert!(
            fs::read_to_string(&installed)
                .await
                .unwrap()
                .contains("second")
        );
        assert_eq!(
            fs::read_link(config.bin_dir().join("mytool"))
                .await
                .unwrap(),
            installed
        );
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn upgrade_migrates_legacy_external_link_to_snapshot() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"tool.sh\"\ntarget = \"mytool\"\n",
        )
        .await
        .unwrap();
        let source = cat_dir.join("tool.sh");
        fs::write(&source, b"#!/bin/sh\necho legacy\n")
            .await
            .unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        fs::symlink(&source, config.bin_dir().join("mytool"))
            .await
            .unwrap();

        let mut separator = crate::output::SectionSeparator::new();
        let report = handle_upgrade_installed(&config, false, &mut separator)
            .await
            .unwrap();
        assert_eq!(report.snapshots_updated, 1);
        assert_eq!(
            fs::read_link(config.bin_dir().join("mytool"))
                .await
                .unwrap(),
            config.installed_shell_dir().join("custom/tool.sh")
        );
        assert!(
            crate::shells::deployment::ShellManifest::load(&shine_core::runtime::RealHost, &config)
                .await
                .unwrap()
                .find("shell/custom/mytool")
                .is_some()
        );
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn upgrade_switches_snapshot_raw_link_to_explicit_live_source() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("presets/shell/custom");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("shine.toml"),
            b"[[files]]\nsource = \"tool.sh\"\ntarget = \"mytool\"\n",
        )
        .await
        .unwrap();
        let source = cat_dir.join("tool.sh");
        fs::write(&source, b"#!/bin/sh\necho live\n").await.unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.bin_dir()).await.unwrap();
        handle_install(&config, Some("custom"), false)
            .await
            .unwrap();

        config.external_shell_mode = crate::config::ExternalShellMode::Live;
        let mut separator = crate::output::SectionSeparator::new();
        handle_upgrade_installed(&config, false, &mut separator)
            .await
            .unwrap();
        assert_eq!(
            fs::read_link(config.bin_dir().join("mytool"))
                .await
                .unwrap(),
            source
        );
        let manifest =
            crate::shells::deployment::ShellManifest::load(&shine_core::runtime::RealHost, &config)
                .await
                .unwrap();
        assert_eq!(
            manifest.find("shell/custom/mytool").unwrap().mode,
            crate::config::ExternalShellMode::Live
        );
        fs::remove_dir_all(&dir).await.unwrap();
    }
}
