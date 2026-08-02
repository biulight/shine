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
    handle_list_with_presets_note(config, true).await
}

#[doc(hidden)]
pub async fn handle_list_with_presets_note(
    config: &crate::config::Config,
    print_presets_note: bool,
) -> anyhow::Result<()> {
    if print_presets_note {
        crate::config::print_presets_note(config);
    }
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

    let bun_available = crate::platform::command_exists_on_path("bun");

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
            if script.runtime == crate::bin_links::LinkRuntime::Bun {
                let status = if bun_available {
                    colors::green("available")
                } else {
                    colors::yellow("not found on PATH")
                };
                println!(
                    "{continuation_indent}{} {status}",
                    colors::dim("runtime: bun ·")
                );
            }
            println!();
        }
    }

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

    Ok(())
}

pub async fn handle_info(config: &crate::config::Config, target: &str) -> anyhow::Result<()> {
    use anyhow::bail;

    crate::config::print_presets_note(config);
    let categories = metadata::load_active_categories(config, None).await?;
    let target = target.trim();
    if target.is_empty() {
        bail!("shell info target must not be empty");
    }

    let (category, files) = if let Some(category) = categories.iter().find(|cat| cat.name == target)
    {
        (category, category.files.iter().collect::<Vec<_>>())
    } else if let Some((category_name, command_name)) = target.split_once('/') {
        let Some(category) = categories.iter().find(|cat| cat.name == category_name) else {
            bail!("shell preset category not found: {category_name}");
        };
        let Some(file) = category
            .files
            .iter()
            .find(|file| file.command_name == command_name)
        else {
            bail!("shell preset command not found: {target}");
        };
        (category, vec![file])
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
                "shell preset target not found: {target}\n\nRun `shine shell list` to see available presets."
            ),
            [(category, file)] => (*category, vec![*file]),
            _ => {
                let choices = matches
                    .iter()
                    .map(|(category, file)| format!("{}/{}", category.name, file.command_name))
                    .collect::<Vec<_>>()
                    .join(", ");
                bail!("ambiguous shell preset target `{target}`; use one of: {choices}");
            }
        }
    };

    let rows = crate::status::build_shell_rows(config).await?;
    println!("{}", colors::bold(&category.name));
    if let Some(description) = &category.description {
        println!("  {}", colors::dim(description));
    }

    let bun_available = crate::platform::command_exists_on_path("bun");
    let mut any_installed = false;
    for file in files {
        let label = format!("{}/{}", category.name, file.command_name);
        let row = rows.iter().find(|row| row.label == label);
        let command_path = crate::bin_links::command_path_for_name(
            config.bin_dir(),
            std::ffi::OsStr::new(&file.command_name),
        );
        any_installed |= command_path.exists()
            || tokio::fs::symlink_metadata(&command_path)
                .await
                .is_ok_and(|metadata| metadata.file_type().is_symlink());
        println!();
        println!("  {}", colors::bold(&file.command_name));
        println!(
            "    {:<12} shell/{}/{}",
            "Source",
            category.name,
            file.source_rel.display()
        );
        let runtime = match file.runtime {
            crate::bin_links::LinkRuntime::Native => "native".to_string(),
            crate::bin_links::LinkRuntime::Bun if bun_available => "bun (available)".to_string(),
            crate::bin_links::LinkRuntime::Bun => "bun (not found on PATH)".to_string(),
        };
        println!("    {:<12} {runtime}", "Runtime");
        println!(
            "    {:<12} {}",
            "Transforms",
            if file.transforms.is_empty() {
                "none".to_string()
            } else {
                file.transforms.join(", ")
            }
        );
        println!(
            "    {:<12} {}",
            "Environment",
            if file.env.is_empty() {
                "none".to_string()
            } else {
                file.env
                    .iter()
                    .map(crate::env::EnvVarSpec::to_with_arg)
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        );
        println!(
            "    {:<12} {}",
            "Status",
            row.map_or("not installed", |row| row.status_text)
        );
        for (index, line) in file.description.iter().enumerate() {
            println!(
                "    {:<12} {}",
                if index == 0 { "Description" } else { "" },
                line
            );
        }
    }

    println!();
    if any_installed {
        println!(
            "{}",
            colors::dim(&format!(
                "Run `shine install shell/{} --replace-managed` to repair this category.",
                category.name
            ))
        );
    } else {
        println!(
            "{}",
            colors::dim(&format!(
                "Run `shine shell install {}` to install this category.",
                category.name
            ))
        );
    }
    Ok(())
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
