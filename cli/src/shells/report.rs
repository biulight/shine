use super::metadata;
use crate::colors;
use crate::output::escape_controls;

#[derive(Debug, Default)]
pub struct ShellUpgradeReport {
    /// User-facing shell targets that were brought up to date. A target is
    /// counted once even when upgrading it also rewrites a preset source,
    /// rendered template, and bin launcher.
    pub updated_targets: Vec<String>,
    /// Category-level lifecycle identities corresponding to `updated_targets`.
    /// Summary counts use these so discovery and upgrade reporting agree.
    pub updated_categories: Vec<String>,
    pub snapshots_updated: usize,
    pub templates_updated: usize,
    pub links_created: usize,
    pub links_updated: usize,
    pub link_conflicts: usize,
    pub path_changed: bool,
}

pub(super) fn shell_cache_summary_parts(
    report: &shine_core::runtime::ShellCacheReport,
) -> Vec<String> {
    cache_summary_parts(
        report.created.len(),
        report.overwritten.len(),
        report.skipped.len(),
    )
}

fn cache_summary_parts(created: usize, overwritten: usize, skipped: usize) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    if created > 0 {
        parts.push(colors::green(&format_file_action(created, "created")));
    }
    if overwritten > 0 {
        parts.push(colors::green(&format_file_action(overwritten, "updated")));
    }
    if skipped > 0 {
        parts.push(colors::dim(&format_file_action(skipped, "skipped")));
    }
    parts
}

pub(super) fn unlink_report_summary_parts(
    unlink_report: &crate::bin_links::UnlinkReport,
    dry_run: bool,
) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    if !unlink_report.removed.is_empty() {
        parts.push(colors::green(&format!(
            "{} {}",
            unlink_report.removed.len(),
            if dry_run { "would remove" } else { "removed" }
        )));
    }
    if !unlink_report.skipped.is_empty() {
        parts.push(colors::dim(&format!(
            "{} retained (ownership conflict)",
            unlink_report.skipped.len()
        )));
    }
    parts
}

pub(super) fn shell_cache_remove_summary_parts(
    report: &shine_core::runtime::ShellCacheReport,
    dry_run: bool,
) -> Vec<String> {
    let mut parts = Vec::new();
    if !report.removed.is_empty() {
        parts.push(colors::green(&format_file_action(
            report.removed.len(),
            if dry_run { "would remove" } else { "removed" },
        )));
    }
    if !report.skipped.is_empty() {
        parts.push(colors::dim(&format_file_action(
            report.skipped.len(),
            "skipped",
        )));
    }
    parts
}

pub(super) fn link_report_summary_parts(link_report: &crate::bin_links::LinkReport) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    if !link_report.created.is_empty() {
        parts.push(colors::green(&format!(
            "{} created",
            link_report.created.len()
        )));
    }
    if !link_report.overwritten.is_empty() {
        parts.push(colors::green(&format!(
            "{} updated",
            link_report.overwritten.len()
        )));
    }
    if !link_report.skipped.is_empty() {
        parts.push(colors::dim(&format!(
            "{} up to date",
            link_report.skipped.len()
        )));
    }
    if !link_report.conflicts.is_empty() {
        parts.push(colors::yellow(&format!(
            "{} conflicts",
            link_report.conflicts.len()
        )));
    }
    if parts.is_empty() {
        parts.push(colors::dim("0 linked"));
    }
    parts
}

fn format_file_action(count: usize, action: &str) -> String {
    let noun = if count == 1 { "file" } else { "files" };
    format!("{count} {noun} {action}")
}

pub async fn handle_list(config: &crate::config::Config) -> anyhow::Result<()> {
    handle_list_with_presets_note(config, true).await
}

#[doc(hidden)]
pub async fn handle_list_with_presets_note(
    config: &crate::config::Config,
    print_presets_note: bool,
) -> anyhow::Result<()> {
    let overlay = config.active_presets_overlay_dir();
    if print_presets_note {
        crate::config::print_presets_note_with_overlay(config, overlay);
    }
    let catalog = crate::core_runtime::for_shell_catalog(config, overlay).await?;
    print_categories(catalog.shell_categories(None)?);
    Ok(())
}

pub(crate) fn print_categories(mut categories: Vec<metadata::ShellCategory>) {
    categories.retain(|category| !category.files.is_empty());

    if categories.is_empty() {
        println!("{}", colors::dim("No shell preset categories found."));
        return;
    }

    println!("{}\n", colors::bold("Shell Preset Categories"));

    let bun_available = crate::platform::command_exists_on_path("bun");

    for cat in &categories {
        let word = if cat.files.len() == 1 {
            "script"
        } else {
            "scripts"
        };
        println!(
            "  {}  {}",
            escape_controls(&cat.name),
            colors::dim(&format!("{} {}", cat.files.len(), word))
        );

        let names: Vec<String> = cat
            .files
            .iter()
            .map(|s| escape_controls(&s.command_name))
            .collect();
        let max_name = names
            .iter()
            .map(|s| console::measure_text_width(s))
            .max()
            .unwrap_or(0);
        let desc_col = max_name + 4;

        for (script, name) in cat.files.iter().zip(names.iter()) {
            let mut line = format!("    {name}");
            let summary = script
                .description
                .first()
                .and_then(|description| description.lines().next());
            if let Some(summary) = summary.filter(|summary| !summary.is_empty()) {
                let padding = desc_col - console::measure_text_width(name);
                line.push_str(&" ".repeat(padding));
                line.push_str(&escape_controls(summary));
            }
            if !bun_available && script.runtime == crate::bin_links::LinkRuntime::Bun {
                line.push(' ');
                line.push_str(&colors::yellow("[Bun missing]"));
            }
            println!("{line}");
        }
        println!();
    }

    println!(
        "{}",
        colors::dim(
            "Run `shine shell info <CATEGORY>/<COMMAND>` for details (e.g. `shine shell info proxy/setproxy`)."
        )
    );
    println!(
        "{}",
        colors::dim("Run `shine install shell/<CATEGORY>` to install a specific category.")
    );
    println!(
        "{}",
        colors::dim("Run `shine shell install` to install all.")
    );
    println!();
    println!(
        "{}",
        colors::dim(
            "After installation, commands are available directly by name (e.g. `setproxy`)."
        )
    );
}

pub async fn handle_info(config: &crate::config::Config, target: &str) -> anyhow::Result<()> {
    let overlay = crate::config::print_presets_note_and_get_overlay(config);
    let service = crate::core_runtime::frontend_with_overlay(config, overlay).await?;
    print_info(&service, target).await
}

pub(crate) async fn print_info(
    service: &crate::core_runtime::ConfiguredFrontend,
    target: &str,
) -> anyhow::Result<()> {
    use crate::status::{ShellRow, ShellRowStatus};
    use anyhow::bail;

    let categories = service.runtime().shell_categories(None)?;
    if target.trim().is_empty() {
        bail!("shell info target must not be empty");
    }

    let (category, files, command) = if let Some(category) =
        categories.iter().find(|cat| cat.name == target)
    {
        (category, category.files.iter().collect::<Vec<_>>(), None)
    } else if let Some((category_name, command_name)) = target.split_once('/') {
        let Some(category) = categories.iter().find(|cat| cat.name == category_name) else {
            bail!(
                "shell preset category not found: {}",
                escape_controls(category_name)
            );
        };
        let Some(file) = category
            .files
            .iter()
            .find(|file| file.command_name == command_name)
        else {
            bail!(
                "shell preset command not found: {}",
                escape_controls(target)
            );
        };
        (category, vec![file], Some(file.command_name.as_str()))
    } else {
        let matches = categories
            .iter()
            .flat_map(|category| {
                category
                    .files
                    .iter()
                    .filter(move |file| file.command_name == target)
                    .map(move |file| (category, file))
            })
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => bail!(
                "shell preset target not found: {}\n\nRun `shine shell list` to see available presets.",
                escape_controls(target)
            ),
            [(category, file)] => (*category, vec![*file], Some(file.command_name.as_str())),
            _ => {
                let choices = matches
                    .iter()
                    .map(|(category, file)| {
                        escape_controls(&format!("{}/{}", category.name, file.command_name))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                bail!(
                    "ambiguous shell preset target `{}`; use one of: {choices}",
                    escape_controls(target)
                );
            }
        }
    };

    let rows = crate::status::inspect_shell_rows(service).await?;
    let category_name = escape_controls(&category.name);
    if let Some(description) = &category.description {
        println!(
            "{}  {}",
            colors::bold(&category_name),
            colors::dim(&escape_controls(description))
        );
    } else {
        println!("{}", colors::bold(&category_name));
    }

    let bun_available = crate::platform::command_exists_on_path("bun");
    let detail_indent = "       ";
    let mut needs_install = false;
    let mut needs_upgrade = false;
    let mut needs_review = false;
    let mut has_blocker = false;
    for file in files {
        let row = rows
            .iter()
            .find(|row| row.category == category.name && row.command_name == file.command_name);
        let status = row.map_or(ShellRowStatus::NotInstalled, ShellRow::status);
        match status {
            ShellRowStatus::NotInstalled => needs_install = true,
            ShellRowStatus::UpdateAvail => needs_upgrade = true,
            ShellRowStatus::Attention | ShellRowStatus::Partial => needs_review = true,
            ShellRowStatus::UpToDate => {}
        }
        has_blocker |= row.is_some_and(|row| row.link_conflict || row.preset_missing);

        let status_text = row.map_or("not installed", |row| row.status_text);
        let symbol = status.symbol();
        println!(
            "\n  {}  {}  {}",
            colors::symbol(symbol),
            escape_controls(&file.command_name),
            colors::status_label(&format!("[{status_text}]"), symbol)
        );
        for line in &file.description {
            println!("{detail_indent}{}", colors::dim(&escape_controls(line)));
        }
        if command.is_some() {
            let source = escape_controls(&crate::path_display::format(&file.source_rel));
            println!(
                "{detail_indent}{:<12} shell/{category_name}/{source}",
                "Source"
            );
        }
        if file.runtime == crate::bin_links::LinkRuntime::Bun {
            let runtime = if bun_available {
                "bun (available)"
            } else {
                "bun (not found on PATH)"
            };
            println!("{detail_indent}{:<12} {runtime}", "Runtime");
        }
        if !file.transforms.is_empty() {
            println!(
                "{detail_indent}{:<12} {}",
                "Transforms",
                file.transforms.join(", ")
            );
        }
        if !file.env.is_empty() {
            let env = file
                .env
                .iter()
                .map(crate::env::EnvVarSpec::to_with_arg)
                .collect::<Vec<_>>()
                .join(", ");
            println!("{detail_indent}{:<12} {env}", "Environment");
        }
    }

    if !needs_install && !needs_upgrade && !needs_review {
        return Ok(());
    }
    let target = if let Some(command) = command {
        format!("shell/{}/{command}", category.name)
    } else {
        format!("shell/{}", category.name)
    };
    println!();
    if needs_review {
        print_shell_info_hint("info", &target, " --diff", "review the reported problems");
    }
    if !has_blocker {
        if needs_install {
            let purpose = if command.is_some() {
                "install this command".to_string()
            } else {
                format!("install the {category_name} category")
            };
            print_shell_info_hint("install", &target, "", &purpose);
        }
        if needs_upgrade {
            print_shell_info_hint("upgrade", &target, "", "apply available updates");
        }
    }
    Ok(())
}

fn print_shell_info_hint(command: &str, target: &str, options: &str, purpose: &str) {
    let hint = if target.chars().any(char::is_control) {
        format!(
            "Use shine {command}{options} with the original target (control characters; copyable command omitted) to {purpose}."
        )
    } else {
        let target = crate::shell_quote::command_arg(target);
        format!("Run `shine {command} {target}{options}` to {purpose}.")
    };
    println!("{}", colors::dim(&hint));
}

#[cfg(test)]
mod info_tests {
    use super::*;

    #[tokio::test]
    async fn embedded_shell_info_accepts_category_command_and_canonical_target() {
        let dir = crate::test_support::make_temp_dir("shine-shell-info").await;
        let config = crate::test_support::test_config(&dir);

        handle_info(&config, "proxy").await.unwrap();
        handle_info(&config, "setproxy").await.unwrap();
        handle_info(&config, "proxy/setproxy").await.unwrap();

        tokio::fs::remove_dir_all(dir).await.unwrap();
    }

    #[tokio::test]
    async fn shell_info_rejects_unknown_and_empty_targets() {
        let dir = crate::test_support::make_temp_dir("shine-shell-info-errors").await;
        let config = crate::test_support::test_config(&dir);

        assert!(handle_info(&config, "").await.is_err());
        assert!(handle_info(&config, "not-a-preset").await.is_err());

        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
}

#[cfg(test)]
mod lifecycle_render_tests {
    use super::*;

    #[test]
    fn shell_install_summary_parts_keep_legacy_wording() {
        let report = crate::bin_links::LinkReport {
            created: vec!["created".into()],
            overwritten: vec!["updated".into()],
            skipped: vec!["current".into()],
            conflicts: Vec::new(),
        };
        assert_eq!(
            link_report_summary_parts(&report),
            vec!["1 created", "1 updated", "1 up to date"]
        );
    }
}
