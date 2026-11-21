use super::metadata;
use crate::colors;
use crate::config::Config;
use crate::output::escape_controls;
use crate::path_display;
use crate::status::FileStatus;
use anyhow::Result;
use shine_core::runtime::AppFileInspection;
use std::collections::BTreeMap;
use std::path::Path;

use super::AppListMode;

pub async fn handle_info(
    config: &Config,
    category: &str,
    run_generators: bool,
    diff: bool,
) -> Result<()> {
    for line in crate::config::presets_source_note_lines(config) {
        println!("{line}");
    }
    println!();
    let mut observer = shine_core::runtime::NullObserver;
    let frontend = crate::core_runtime::frontend_from_config(config).await?;
    let inspections = frontend
        .inspect_apps_with_options(
            shine_core::runtime::AppInspectionOptions {
                run_generators,
                categories: vec![category.to_string()],
            },
            &mut observer,
        )
        .await
        .map_err(shine_core::frontend::FrontendServiceError::into_source)?
        .files;
    let selected = inspections
        .iter()
        .filter(|inspection| inspection.category.name == category)
        .collect::<Vec<_>>();
    let cat = selected
        .first()
        .map(|inspection| &inspection.category)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "app preset category not found: {}",
                escape_controls(category)
            )
        })?;

    // Header
    if let Some(desc) = &cat.description {
        println!(
            "{}  {}",
            colors::bold(&escape_controls(&cat.name)),
            colors::dim(&escape_controls(desc))
        );
    } else {
        println!("{}", colors::bold(&escape_controls(&cat.name)));
    }
    println!();

    // A declaration that no file uses may be unresolvable. Each row still has its
    // own resolved destination, so display those paths without a root in that case.
    let runtime = frontend.runtime();
    let home = &runtime.context().home_dir;
    let destination_root = runtime.app_category_destination_root(cat).ok().flatten();
    if let Some(root) = &destination_root {
        println!(
            "  {}  {}",
            colors::dim("Destination"),
            escape_controls(&path_display::format_home(root, home))
        );
    }
    println!("  {}  {}", colors::dim("Files      "), selected.len());
    println!();

    for inspection in &selected {
        print_app_info_file(inspection, destination_root.as_deref(), home);
        if diff {
            crate::info::print_app_inspection_diff(config, inspection).await?;
        }
    }

    for inspection in &selected {
        if let Some(scope) = &inspection.code_analysis {
            for line in crate::code_analysis::lines(scope, true) {
                println!("{line}");
            }
        }
    }
    let generator_failures = selected
        .iter()
        .filter(|inspection| {
            matches!(
                inspection.assessment_diagnostic,
                Some("app_generator_evaluation_failed" | "app_generator_trust_required")
            )
        })
        .collect::<Vec<_>>();
    for inspection in &generator_failures {
        println!();
        let reason = if inspection.assessment_diagnostic == Some("app_generator_trust_required") {
            "generator trust required"
        } else {
            "generator evaluation failed"
        };
        println!(
            "{}",
            colors::yellow(&format!(
                "! {}: {reason}",
                escape_controls(&inspection.file.source_rel.display().to_string())
            ))
        );
        if let Some(error) = &inspection.assessment_error {
            println!("{}", colors::dim(&format!("  {}", escape_controls(error))));
        }
    }

    print_app_info_summary(&selected, category);

    if !generator_failures.is_empty() {
        anyhow::bail!("one or more App generators could not be evaluated");
    }
    Ok(())
}

fn print_app_info_file(inspection: &AppFileInspection, root: Option<&Path>, home: &Path) {
    let detail_indent = "       ";
    let source = inspection.file.source_rel.as_path();
    let destination = inspection.destination.as_deref();
    let relative_destination =
        destination.and_then(|destination| destination.strip_prefix(root?).ok());
    let inline_mapping = relative_destination
        .filter(|relative| *relative != source)
        .map_or_else(String::new, |relative| {
            let target = escape_controls(&path_display::format(relative));
            format!("  {}", colors::dim(&format!("→ {target}")))
        });
    let (label, symbol) = if destination_managed_by_other_source(inspection) {
        ("destination managed by another source", "!")
    } else {
        app_info_status(inspection.status)
    };
    let status = colors::status_label(&format!("[{label}]"), symbol);
    println!(
        "  {}  {}  {status}{inline_mapping}",
        colors::symbol(symbol),
        escape_controls(&path_display::format(source))
    );
    if let Some(description) = &inspection.file.description {
        println!(
            "{detail_indent}{}",
            colors::dim(&escape_controls(description))
        );
    }
    if relative_destination.is_none() {
        let detail = match destination {
            Some(path) => format!(
                "→ {}",
                escape_controls(&path_display::format_home(path, home))
            ),
            None => "(destination unresolvable)".to_string(),
        };
        println!("{detail_indent}{}", colors::dim(&detail));
    }
}

fn app_info_status(status: FileStatus) -> (&'static str, &'static str) {
    match status {
        FileStatus::NotInstalled => ("not installed", "✗"),
        FileStatus::UpToDate => ("up to date", "✓"),
        FileStatus::UpdateAvail => ("update available", "↑"),
        FileStatus::GeneratorNotEvaluated => ("generator not evaluated", "!"),
        FileStatus::GeneratorEvaluationFailed => ("generator evaluation failed", "!"),
        FileStatus::GeneratorTrustRequired => ("generator trust required", "!"),
        FileStatus::Partial => ("missing managed keys", "~"),
        FileStatus::UserModified => ("user modified", "~"),
        FileStatus::Missing => ("missing on disk", "!"),
    }
}

fn destination_managed_by_other_source(file: &AppFileInspection) -> bool {
    file.manifest_entry.as_ref().is_some_and(|entry| {
        Some(&entry.destination) == file.destination.as_ref()
            && !entry
                .source
                .strip_prefix(&format!("app/{}/", file.category.name))
                .is_some_and(|source| Path::new(source) == file.file.source_rel)
    })
}

fn print_app_info_summary(files: &[&AppFileInspection], category: &str) {
    let mut counts = BTreeMap::new();
    let mut deferred = 0;
    let mut conflicts = 0;
    for file in files {
        if destination_managed_by_other_source(file) {
            conflicts += 1;
        } else if file.status != FileStatus::GeneratorNotEvaluated {
            *counts.entry(file.status).or_insert(0) += 1;
        }
        // A missing, modified, or uninstalled file can also have deferred generator output.
        if file.assessment_diagnostic == Some("app_generator_not_evaluated") {
            deferred += 1;
        }
    }
    if deferred > 0 {
        counts.insert(FileStatus::GeneratorNotEvaluated, deferred);
    }
    let mut summary = counts
        .iter()
        .map(|(&status, &count)| {
            let label = match (status, count) {
                (FileStatus::GeneratorNotEvaluated, 2..) => "generators not evaluated",
                (FileStatus::GeneratorEvaluationFailed, 2..) => "generator evaluations failed",
                _ => app_info_status(status).0,
            };
            format!("{count} {label}")
        })
        .collect::<Vec<_>>();
    if conflicts > 0 {
        let label = if conflicts == 1 {
            "destination managed by another source"
        } else {
            "destinations managed by other sources"
        };
        summary.push(format!("{conflicts} {label}"));
    }
    println!("\n  {}", summary.join(" · "));

    let controls = category.chars().any(char::is_control);
    if deferred > 0 {
        let hint = if controls {
            "Re-run with `--run-generators` and the original category (control characters; copyable command omitted).".to_string()
        } else {
            let command = if category.starts_with('-') {
                let operand = crate::shell_quote::command_positionals(&[category]);
                format!("shine app info --run-generators {operand}")
            } else {
                let operand = crate::shell_quote::command_arg(category);
                format!("shine app info {operand} --run-generators")
            };
            format!("Run generators to check output with `{command}`.")
        };
        println!("  {}", colors::dim(&hint));
    }

    // Install operates on the entire category, including any conflicting file.
    if conflicts > 0 {
        return;
    }
    let target_arg = crate::shell_quote::command_arg(&format!("app/{category}"));
    let install = format!("shine install {target_arg}");
    if counts.contains_key(&FileStatus::Missing) || counts.contains_key(&FileStatus::Partial) {
        print_app_install_hint(controls, &install, "restore missing files or managed keys");
    } else if counts.contains_key(&FileStatus::NotInstalled) {
        print_app_install_hint(controls, &install, "install");
    }

    if files.iter().any(|file| {
        file.status == FileStatus::UserModified
            && file
                .manifest_entry
                .as_ref()
                .is_some_and(|entry| Some(&entry.destination) == file.destination.as_ref())
    }) {
        print_app_install_hint(
            controls,
            &format!("{install} --replace-managed"),
            "replace user-modified managed files",
        );
    }
}

fn print_app_install_hint(controls: bool, command: &str, purpose: &str) {
    let hint = if controls {
        format!(
            "Use shine install with the original target (control characters; copyable command omitted) to {purpose}."
        )
    } else {
        format!("Run `{command}` to {purpose}.")
    };
    println!("  {}", colors::dim(&hint));
}

pub async fn handle_list(config: &Config) -> Result<()> {
    for line in crate::config::presets_source_note_lines(config) {
        println!("{line}");
    }
    println!();
    print_app_catalog(config, AppCatalogView::Categories).await
}

#[doc(hidden)]
pub async fn handle_list_with_presets_note(
    config: &Config,
    print_presets_note: bool,
) -> Result<()> {
    if print_presets_note {
        crate::config::print_presets_note(config);
    }
    print_app_catalog(config, AppCatalogView::Available).await
}

enum AppCatalogView {
    Categories,
    Available,
}

async fn print_app_catalog(config: &Config, view: AppCatalogView) -> Result<()> {
    let categories = metadata::load_active_categories(config, None).await?;

    if categories.is_empty() {
        println!("{}", colors::dim("No app preset categories found."));
        return Ok(());
    }

    println!("{}\n", colors::bold("App Preset Categories"));

    let name_width = categories
        .iter()
        .map(|c| console::measure_text_width(&escape_controls(&c.name)))
        .max()
        .unwrap_or(0);
    let description_indent = " ".repeat(4 + name_width);

    for cat in &categories {
        let effective_desc = cat.description.as_deref().or_else(|| {
            if cat.files.len() == 1 {
                cat.files[0].description.as_deref()
            } else {
                None
            }
        });

        let name = escape_controls(&cat.name);
        let name_pad = " ".repeat(name_width.saturating_sub(console::measure_text_width(&name)));
        if matches!(view, AppCatalogView::Categories) {
            if let Some(description) = effective_desc
                .map(escape_controls)
                .filter(|description| !description.trim().is_empty())
            {
                let mut output = String::new();
                crate::preset_report::write_wrapped_with_continuation(
                    &mut output,
                    &format!("  {name}{name_pad}  "),
                    &description_indent,
                    &description,
                );
                print!("{output}");
            } else {
                println!("  {name}");
            }
            continue;
        }

        let count = cat.files.len() + cat.secret_files.len();
        let file_count = if count > 1 {
            format!("  {}", colors::dim(&format!("{count} files")))
        } else {
            String::new()
        };

        let desc_part = effective_desc
            .map(|d| format!("  {}", escape_controls(d)))
            .unwrap_or_default();

        println!("  {name}{name_pad}{desc_part}{file_count}");

        // Per-file rows for explicit multi-file categories
        if cat.has_explicit_files && cat.list_mode == AppListMode::Files && cat.files.len() > 1 {
            for file in &cat.files {
                let name = escape_controls(&file.source_rel.display().to_string());
                if let Some(desc) = &file.description {
                    println!(
                        "    {}  {}",
                        colors::dim(&name),
                        colors::dim(&escape_controls(desc))
                    );
                } else {
                    println!("    {}", colors::dim(&name));
                }
            }
        }
    }

    println!();
    let hints = match view {
        AppCatalogView::Categories => [
            "View files and status with `shine app info <CATEGORY>`.",
            "Preview installation with `shine app install <CATEGORY> --dry-run`.",
        ],
        AppCatalogView::Available => [
            "Run `shine install app/<CATEGORY>` to install a specific category.",
            "Run `shine app install` to install all.",
        ],
    };
    for hint in hints {
        println!("{}", colors::dim(hint));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_uses_embedded_metadata_for_vim() {
        let categories = metadata::load_embedded_categories(Some("vim")).unwrap();
        let vim = categories.iter().find(|c| c.name == "vim").unwrap();
        assert!(vim.uses_metadata);
        assert_eq!(vim.destination_root.as_deref(), Some("~/.vim"));
    }
}
