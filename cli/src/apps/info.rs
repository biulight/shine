use super::metadata;
use crate::colors;
use crate::config::Config;
use crate::path_display;
use anyhow::Result;

use super::AppListMode;

pub async fn handle_info(config: &Config, category: &str) -> Result<()> {
    crate::config::print_presets_note(config);
    let mut observer = utils::runtime::NullObserver;
    let inspections = crate::core_runtime::from_config(config)?
        .inspect_apps(&mut observer)
        .await?;
    let selected = inspections
        .iter()
        .filter(|inspection| inspection.category.name == category)
        .collect::<Vec<_>>();
    let cat = selected
        .first()
        .map(|inspection| &inspection.category)
        .ok_or_else(|| anyhow::anyhow!("app preset category not found: {category}"))?;

    // Header
    if let Some(desc) = &cat.description {
        println!("{}  {}", colors::bold(&cat.name), colors::dim(desc));
    } else {
        println!("{}", colors::bold(&cat.name));
    }
    println!();

    if let Some(dest_root) = &cat.destination_root {
        println!(
            "  {}  {}",
            colors::dim("Destination"),
            path_display::format_tilde_path(dest_root, &config.home_dir)
        );
    }
    println!("  {}  {}", colors::dim("Files      "), cat.files.len());
    println!();

    let col_width = cat
        .files
        .iter()
        .map(|f| f.source_rel.display().to_string().len())
        .max()
        .unwrap_or(0);

    let mut any_installed = false;

    for inspection in selected {
        let file = &inspection.file;
        let source_name = file.source_rel.display().to_string();
        let padding = " ".repeat(col_width.saturating_sub(source_name.len()));
        let dest_str = match &inspection.destination {
            Some(dest) => {
                let directly_installed = inspection
                    .manifest_entry
                    .as_ref()
                    .is_some_and(|entry| entry.destination == *dest);
                let status = if directly_installed {
                    any_installed = true;
                    match inspection.status {
                        utils::runtime::InspectionFileStatus::Partial => {
                            format!("  {}", colors::yellow("installed, missing managed keys"))
                        }
                        utils::runtime::InspectionFileStatus::UserModified => {
                            format!("  {}", colors::yellow("installed, user-modified"))
                        }
                        utils::runtime::InspectionFileStatus::Missing => {
                            format!("  {}", colors::yellow("installed, missing on disk"))
                        }
                        _ => format!("  {}", colors::green("installed, up to date")),
                    }
                } else {
                    String::new()
                };
                format!(
                    "{}  {}{}",
                    colors::dim("→"),
                    colors::dim(&path_display::format_home(dest, &config.home_dir)),
                    status
                )
            }
            None => colors::dim("(destination unresolvable)"),
        };

        let file_desc = file
            .description
            .as_deref()
            .map(|d| format!("  {}", colors::dim(d)))
            .unwrap_or_default();

        println!("  {source_name}{padding}  {dest_str}{file_desc}");
    }

    println!();
    if any_installed {
        println!(
            "{}",
            colors::dim(&format!(
                "Installed. Run `shine install app/{category} --replace-managed` to repair managed files."
            ))
        );
    } else {
        println!(
            "{}",
            colors::dim(&format!(
                "Not installed. Run `shine install app/{category}` to install."
            ))
        );
    }

    Ok(())
}

pub async fn handle_list(config: &Config) -> Result<()> {
    handle_list_with_presets_note(config, true).await
}

#[doc(hidden)]
pub async fn handle_list_with_presets_note(
    config: &Config,
    print_presets_note: bool,
) -> Result<()> {
    if print_presets_note {
        crate::config::print_presets_note(config);
    }
    let categories = metadata::load_active_categories(config, None).await?;

    if categories.is_empty() {
        println!("{}", colors::dim("No app preset categories found."));
        return Ok(());
    }

    println!("{}\n", colors::bold("App Preset Categories"));

    let name_width = categories.iter().map(|c| c.name.len()).max().unwrap_or(0);

    for cat in &categories {
        let effective_desc = cat.description.as_deref().or_else(|| {
            if cat.files.len() == 1 {
                cat.files[0].description.as_deref()
            } else {
                None
            }
        });

        let name_pad = " ".repeat(name_width.saturating_sub(cat.name.len()));
        let file_count = if cat.files.len() > 1 {
            format!("  {}", colors::dim(&format!("{} files", cat.files.len())))
        } else {
            String::new()
        };

        let desc_part = effective_desc.map(|d| format!("  {d}")).unwrap_or_default();

        println!("  {}{}{}{}", cat.name, name_pad, desc_part, file_count);

        // Per-file rows for explicit multi-file categories
        if cat.has_explicit_files && cat.list_mode == AppListMode::Files && cat.files.len() > 1 {
            for file in &cat.files {
                let name = file.source_rel.display().to_string();
                if let Some(desc) = &file.description {
                    println!("    {}  {}", colors::dim(&name), colors::dim(desc));
                } else {
                    println!("    {}", colors::dim(&name));
                }
            }
        }
    }

    println!();
    println!(
        "{}",
        colors::dim("Run `shine install app/<CATEGORY>` to install a specific category.")
    );
    println!("{}", colors::dim("Run `shine app install` to install all."));

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
