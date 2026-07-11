use super::metadata;
use crate::colors;

#[derive(Debug, Default)]
pub struct ShellUpgradeReport {
    pub templates_updated: usize,
    pub links_created: usize,
    pub links_updated: usize,
    pub link_conflicts: usize,
    pub path_changed: bool,
}

pub(super) fn preset_extract_summary_parts(report: &crate::presets::ExtractReport) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    if !report.created.is_empty() {
        parts.push(colors::green(&format_file_action(
            report.created.len(),
            "created",
        )));
    }
    if !report.overwritten.is_empty() {
        parts.push(colors::green(&format_file_action(
            report.overwritten.len(),
            "updated",
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

pub(super) fn unlink_report_summary_parts(
    unlink_report: &crate::bin_links::UnlinkReport,
) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    if !unlink_report.removed.is_empty() {
        parts.push(colors::green(&format!(
            "{} removed",
            unlink_report.removed.len()
        )));
    }
    if !unlink_report.skipped.is_empty() {
        parts.push(colors::dim(&format!(
            "{} skipped",
            unlink_report.skipped.len()
        )));
    }
    parts
}

pub(super) fn remove_report_summary_parts(
    remove_report: &crate::presets::RemoveReport,
) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    if !remove_report.removed.is_empty() {
        parts.push(colors::green(&format_file_action(
            remove_report.removed.len(),
            "removed",
        )));
    }
    if !remove_report.skipped.is_empty() {
        parts.push(colors::dim(&format_file_action(
            remove_report.skipped.len(),
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

pub(super) fn upgrade_link_report_summary_parts(
    link_report: &crate::bin_links::LinkReport,
    verbose: bool,
) -> Vec<String> {
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
    if verbose && !link_report.skipped.is_empty() {
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
    parts
}

fn format_file_action(count: usize, action: &str) -> String {
    let noun = if count == 1 { "file" } else { "files" };
    format!("{count} {noun} {action}")
}

pub async fn handle_list(config: &crate::config::Config) -> anyhow::Result<()> {
    crate::config::print_presets_note(config);
    let categories = if config.is_external_presets {
        metadata::load_installed_categories(config, None).await?
    } else {
        metadata::load_embedded_categories(None)?
    };

    if categories.is_empty() {
        println!("{}", colors::dim("No shell preset categories found."));
        return Ok(());
    }

    println!("{}\n", colors::bold("Shell Preset Categories"));

    for cat in &categories {
        let word = if cat.files.len() == 1 {
            "script"
        } else {
            "scripts"
        };
        println!(
            "  {}  {}",
            cat.name,
            colors::dim(&format!("{} {}", cat.files.len(), word))
        );

        let names: Vec<&str> = cat.files.iter().map(|s| s.command_name.as_str()).collect();
        let max_name = names.iter().map(|s| s.len()).max().unwrap_or(0);
        let gap = 4;
        let desc_col = max_name + gap;
        let continuation_indent = " ".repeat(4 + desc_col);

        for (script, name) in cat.files.iter().zip(names.iter()) {
            let padding = " ".repeat(desc_col - name.len());
            match script.description.as_slice() {
                [] => println!("    {name}"),
                [first, rest @ ..] => {
                    println!("    {name}{padding}{first}");
                    for line in rest {
                        if line.is_empty() {
                            println!();
                        } else {
                            println!("{continuation_indent}{line}");
                        }
                    }
                }
            }
            println!();
        }
    }

    println!(
        "{}",
        colors::dim("Run `shine shell install <CATEGORY>` to install a specific category.")
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

    Ok(())
}
