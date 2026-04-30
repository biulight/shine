use crate::apps::{
    AppCategory, AppManifest, hash_content, load_embedded_categories, resolve_install_destination,
};
use crate::colors;
use crate::commands::CheckCommands;
use crate::config::Config;
use crate::presets;
use crate::shells::{SENTINEL_START, get_shell_config_path};
use anyhow::Result;

pub(crate) async fn handle_check(config: &Config, command: Option<CheckCommands>) -> Result<()> {
    crate::config::print_presets_note(config);
    match command {
        None => {
            check_shell(config).await?;
            println!();
            check_app(config).await?;
        }
        Some(CheckCommands::Shell) => check_shell(config).await?,
        Some(CheckCommands::App) => check_app(config).await?,
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Shared row types
// ---------------------------------------------------------------------------

/// Per-file status used for aggregation within a category.
/// Higher discriminant = higher priority (wins in fold).
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub(crate) enum FileStatus {
    NotInstalled,
    UpToDate,
    UpdateAvail,
    Partial,
    UserModified,
    Missing,
}

pub(crate) struct ShellRow {
    pub(crate) symbol: String,
    pub(crate) label: String,
    pub(crate) status_sym: &'static str,
    pub(crate) status_text: &'static str,
    /// `true` when at least one of preset-file or bin-symlink exists.
    pub(crate) is_installed: bool,
}

pub(crate) struct AppRow {
    pub(crate) sym: &'static str,
    pub(crate) label: String,
    pub(crate) dest: Option<String>,
    pub(crate) status_text: &'static str,
    pub(crate) file_status: FileStatus,
}

// ---------------------------------------------------------------------------
// Shared row builders (data-only, no printing)
// ---------------------------------------------------------------------------

/// Build shell preset rows.  Does not include the PATH sentinel line.
pub(crate) async fn build_shell_rows(config: &Config) -> Result<Vec<ShellRow>> {
    let categories = presets::list_categories("shell");
    if categories.is_empty() {
        return Ok(Vec::new());
    }

    let presets_shell = config.presets_dir().join("shell");
    let bin_dir = config.bin_dir();
    let mut rows: Vec<ShellRow> = Vec::new();

    for cat in &categories {
        for script in &cat.scripts {
            let script_path = presets_shell.join(&cat.name).join(&script.name);
            let link_name = crate::bin_links::link_stem(std::path::Path::new(&script.name));
            let link_path = bin_dir.join(&link_name);

            let file_exists = script_path.exists();
            let link_exists = link_path.exists() || {
                tokio::fs::symlink_metadata(&link_path)
                    .await
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false)
            };

            let (sym, status_text) = match (file_exists, link_exists) {
                (true, true) => ("✓", "installed"),
                (true, false) => ("~", "preset present, bin symlink missing"),
                (false, true) => ("~", "bin symlink present, preset missing"),
                (false, false) => ("✗", "not installed"),
            };

            rows.push(ShellRow {
                symbol: colors::symbol(sym),
                label: format!("{}/{}", cat.name, script.name),
                status_sym: sym,
                status_text,
                is_installed: file_exists || link_exists,
            });
        }
    }

    Ok(rows)
}

/// Build app config rows for the given pre-loaded categories.
pub(crate) async fn build_app_rows(
    config: &Config,
    categories: &[AppCategory],
) -> Result<Vec<AppRow>> {
    let manifest = AppManifest::load(config.shine_dir()).await?;
    let mut rows: Vec<AppRow> = Vec::new();

    for cat in categories {
        if cat.has_explicit_files {
            for file in &cat.files {
                let (dest_opt, status) = match resolve_install_destination(cat, file, config) {
                    Err(_) => (None, FileStatus::Missing),
                    Ok(dest) => {
                        let manifest_entry = manifest.find_by_dest(&dest);
                        let status = match manifest_entry {
                            None => FileStatus::NotInstalled,
                            Some(entry) => {
                                if !dest.exists() {
                                    FileStatus::Missing
                                } else {
                                    match tokio::fs::read(&dest).await {
                                        Err(_) => FileStatus::Missing,
                                        Ok(dest_bytes) => {
                                            let dest_hash = hash_content(&dest_bytes);
                                            let manifest_hash = entry.content_hash;
                                            if dest_hash != manifest_hash {
                                                FileStatus::UserModified
                                            } else {
                                                let asset_key = format!(
                                                    "app/{}/{}",
                                                    cat.name,
                                                    file.source_rel.display()
                                                );
                                                let embedded_hash =
                                                    presets::read_asset_bytes(&asset_key)
                                                        .map(|b| hash_content(&b));
                                                match embedded_hash {
                                                    Some(emb) if emb != manifest_hash => {
                                                        FileStatus::UpdateAvail
                                                    }
                                                    _ => FileStatus::UpToDate,
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        };
                        (Some(dest), status)
                    }
                };

                let label = file
                    .display_name
                    .clone()
                    .unwrap_or_else(|| format!("{}/{}", cat.name, file.source_rel.display()));

                let dest_str = dest_opt.map(|d| {
                    d.to_string_lossy()
                        .into_owned()
                        .replace(config.home_dir.to_string_lossy().as_ref(), "~")
                });

                let (sym, status_text) = match status {
                    FileStatus::Missing => ("!", "destination missing"),
                    FileStatus::UserModified => ("~", "user modified"),
                    FileStatus::UpdateAvail => ("↑", "update available"),
                    FileStatus::UpToDate => ("✓", "up-to-date"),
                    FileStatus::NotInstalled | FileStatus::Partial => ("✗", "not installed"),
                };

                rows.push(AppRow {
                    sym,
                    label,
                    dest: dest_str,
                    status_text,
                    file_status: status,
                });
            }
        } else {
            let mut file_statuses: Vec<FileStatus> = Vec::new();

            for file in &cat.files {
                let dest = match resolve_install_destination(cat, file, config) {
                    Ok(d) => d,
                    Err(_) => {
                        file_statuses.push(FileStatus::Missing);
                        continue;
                    }
                };

                let manifest_entry = manifest.find_by_dest(&dest);

                let status = match manifest_entry {
                    None => FileStatus::NotInstalled,
                    Some(entry) => {
                        if !dest.exists() {
                            FileStatus::Missing
                        } else {
                            match tokio::fs::read(&dest).await {
                                Err(_) => FileStatus::Missing,
                                Ok(dest_bytes) => {
                                    let dest_hash = hash_content(&dest_bytes);
                                    let manifest_hash = entry.content_hash;
                                    if dest_hash != manifest_hash {
                                        FileStatus::UserModified
                                    } else {
                                        let asset_key = format!(
                                            "app/{}/{}",
                                            cat.name,
                                            file.source_rel.display()
                                        );
                                        let embedded_hash = presets::read_asset_bytes(&asset_key)
                                            .map(|b| hash_content(&b));
                                        match embedded_hash {
                                            Some(emb) if emb != manifest_hash => {
                                                FileStatus::UpdateAvail
                                            }
                                            _ => FileStatus::UpToDate,
                                        }
                                    }
                                }
                            }
                        }
                    }
                };

                file_statuses.push(status);
            }

            let has_installed = file_statuses.iter().any(|s| {
                matches!(
                    s,
                    FileStatus::UpToDate | FileStatus::UpdateAvail | FileStatus::UserModified
                )
            });
            let has_not_installed = file_statuses.contains(&FileStatus::NotInstalled);
            let cat_status = if has_installed && has_not_installed {
                FileStatus::Partial
            } else {
                file_statuses
                    .iter()
                    .copied()
                    .max()
                    .unwrap_or(FileStatus::NotInstalled)
            };

            let dest_display: Option<String> = if let Some(root) = &cat.destination_root {
                Some(
                    shellexpand::tilde(root)
                        .into_owned()
                        .replace(config.home_dir.to_string_lossy().as_ref(), "~"),
                )
            } else if cat.files.len() == 1 {
                resolve_install_destination(cat, &cat.files[0], config)
                    .ok()
                    .map(|p| {
                        let s = p.to_string_lossy().into_owned();
                        s.replace(config.home_dir.to_string_lossy().as_ref(), "~")
                    })
            } else {
                None
            };

            let (sym, status_text) = match cat_status {
                FileStatus::Missing => ("!", "destination missing"),
                FileStatus::UserModified => ("~", "user modified"),
                FileStatus::Partial => ("~", "partial install"),
                FileStatus::UpdateAvail => ("↑", "update available"),
                FileStatus::UpToDate => ("✓", "up-to-date"),
                FileStatus::NotInstalled => ("✗", "not installed"),
            };

            rows.push(AppRow {
                sym,
                label: cat.name.clone(),
                dest: dest_display,
                status_text,
                file_status: cat_status,
            });
        }
    }

    Ok(rows)
}

// ---------------------------------------------------------------------------
// check_shell / check_app — thin wrappers that print everything
// ---------------------------------------------------------------------------

async fn check_shell(config: &Config) -> Result<()> {
    let categories = presets::list_categories("shell");

    println!("{}", colors::bold("Shell Presets"));

    if categories.is_empty() {
        println!("  {}", colors::dim("(no embedded shell presets found)"));
        return Ok(());
    }

    let rows = build_shell_rows(config).await?;

    let label_width = rows.iter().map(|r| r.label.len()).max().unwrap_or(0);

    for row in &rows {
        let pad = " ".repeat(label_width.saturating_sub(row.label.len()));
        println!(
            "  {}  {}{}  {}",
            row.symbol,
            row.label,
            pad,
            colors::status_label(row.status_text, row.status_sym),
        );
    }

    // PATH sentinel check
    let config_path = get_shell_config_path(&config.shell_type, &config.home_dir)?;
    let (path_sym, path_label, path_detail) = match tokio::fs::read_to_string(&config_path).await {
        Ok(content) if content.contains(SENTINEL_START) => {
            ("✓", "PATH configured", config_path.display().to_string())
        }
        Ok(_) => (
            "✗",
            "PATH not configured",
            config_path.display().to_string(),
        ),
        Err(_) => (
            "✗",
            "PATH not configured",
            format!("shell config not found: {}", config_path.display()),
        ),
    };

    let path_label_pad = " ".repeat(label_width.saturating_sub(path_label.len()));
    println!(
        "  {}  {}{}  {}",
        colors::symbol(path_sym),
        path_label,
        path_label_pad,
        colors::dim(&path_detail),
    );

    Ok(())
}

async fn check_app(config: &Config) -> Result<()> {
    println!("{}", colors::bold("App Configs"));

    let categories = match load_embedded_categories(None) {
        Ok(cats) => cats,
        Err(e) => {
            println!(
                "  {}",
                colors::dim(&format!("(failed to load embedded app presets: {e})"))
            );
            return Ok(());
        }
    };

    if categories.is_empty() {
        println!("  {}", colors::dim("(no embedded app presets found)"));
        return Ok(());
    }

    let rows = build_app_rows(config, &categories).await?;

    let mut up_to_date = 0usize;
    let mut update_available = 0usize;
    let mut user_modified = 0usize;
    let mut missing = 0usize;
    let mut not_installed = 0usize;

    let label_width = rows.iter().map(|r| r.label.len()).max().unwrap_or(0);

    for row in &rows {
        let pad = " ".repeat(label_width.saturating_sub(row.label.len()));
        let dest_part = row
            .dest
            .as_deref()
            .map(|d| format!("  {}  {}", colors::dim("→"), colors::dim(d)))
            .unwrap_or_default();

        let run_hint = if row.sym == "↑" {
            format!("  {}", colors::dim("run `shine app install`"))
        } else {
            String::new()
        };

        println!(
            "  {}  {}{}{}  {}{}",
            colors::symbol(row.sym),
            row.label,
            pad,
            dest_part,
            colors::status_label(row.status_text, row.sym),
            run_hint,
        );

        match row.file_status {
            FileStatus::Missing => missing += 1,
            FileStatus::UserModified | FileStatus::Partial => user_modified += 1,
            FileStatus::UpdateAvail => update_available += 1,
            FileStatus::UpToDate => up_to_date += 1,
            FileStatus::NotInstalled => not_installed += 1,
        }
    }

    let total = up_to_date + update_available + user_modified + missing + not_installed;
    if total == 0 {
        println!("  {}", colors::dim("(no app presets found)"));
        return Ok(());
    }

    let mut parts: Vec<String> = Vec::new();
    if up_to_date > 0 {
        parts.push(colors::green(&format!("{up_to_date} up-to-date")));
    }
    if update_available > 0 {
        parts.push(colors::cyan(&format!(
            "{update_available} update available"
        )));
    }
    if user_modified > 0 {
        parts.push(colors::yellow(&format!("{user_modified} user-modified")));
    }
    if missing > 0 {
        parts.push(colors::yellow(&format!("{missing} destination missing")));
    }
    if not_installed > 0 {
        parts.push(colors::dim(&format!("{not_installed} not installed")));
    }

    let sep = colors::dim(" · ");
    println!("\n{}  {}", colors::bold("Summary"), parts.join(&sep));

    Ok(())
}
