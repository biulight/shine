use crate::apps::{
    AppManifest, hash_content, load_embedded_categories, resolve_install_destination,
};
use crate::colors;
use crate::commands::CheckCommands;
use crate::config::Config;
use crate::presets;
use crate::shells::{SENTINEL_START, get_shell_config_path};
use anyhow::Result;

pub(crate) async fn handle_check(config: &Config, command: Option<CheckCommands>) -> Result<()> {
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

async fn check_shell(config: &Config) -> Result<()> {
    let categories = presets::list_categories("shell");

    println!("Shell presets:");

    if categories.is_empty() {
        println!("  (no embedded shell presets found)");
        return Ok(());
    }

    let presets_shell = config.presets_dir().join("shell");
    let bin_dir = config.bin_dir();

    for cat in &categories {
        for script in &cat.scripts {
            let script_path = presets_shell.join(&cat.name).join(&script.name);
            let link_path = bin_dir.join(&script.name);

            let file_exists = script_path.exists();
            let link_exists = link_path.exists() || {
                // also check as a symlink that may be broken
                tokio::fs::symlink_metadata(&link_path)
                    .await
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false)
            };

            let (symbol, status) = match (file_exists, link_exists) {
                (true, true) => ("✓", "installed"),
                (true, false) => ("~", "preset file present but bin symlink missing"),
                (false, true) => ("~", "bin symlink present but preset file missing"),
                (false, false) => ("✗", "not installed"),
            };

            println!(
                "  {}  {}/{}  {}",
                colors::symbol(symbol),
                cat.name,
                script.name,
                status
            );
        }
    }

    // Check PATH sentinel in shell config
    let config_path = get_shell_config_path(&config.shell_type, &config.home_dir)?;
    let (path_symbol, path_status) = match tokio::fs::read_to_string(&config_path).await {
        Ok(content) if content.contains(SENTINEL_START) => {
            ("✓", format!("PATH configured  ({})", config_path.display()))
        }
        Ok(_) => (
            "✗",
            format!("PATH not configured  ({})", config_path.display()),
        ),
        Err(_) => (
            "✗",
            format!(
                "PATH not configured  (shell config not found: {})",
                config_path.display()
            ),
        ),
    };
    println!("  {}  {}", colors::symbol(path_symbol), path_status);

    Ok(())
}

/// Per-file status used for aggregation within a category.
/// Higher discriminant = higher priority (wins in fold).
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
enum FileStatus {
    NotInstalled, // ✗  none installed
    UpToDate,     // ✓  all good
    UpdateAvail,  // ↑  newer version available
    Partial,      // ~  mix of states
    UserModified, // ~  user touched the file
    Missing,      // !  in manifest but dest deleted
}

async fn check_app(config: &Config) -> Result<()> {
    println!("App configs:");

    let categories = match load_embedded_categories(None) {
        Ok(cats) => cats,
        Err(e) => {
            println!("  (failed to load embedded app presets: {e})");
            return Ok(());
        }
    };

    if categories.is_empty() {
        println!("  (no embedded app presets found)");
        return Ok(());
    }

    let manifest = AppManifest::load(config.shine_dir()).await?;

    let mut up_to_date = 0usize;
    let mut update_available = 0usize;
    let mut user_modified = 0usize;
    let mut missing = 0usize;
    let mut not_installed = 0usize;

    for cat in &categories {
        // Collect a FileStatus for every file in this category.
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
                                    let asset_key =
                                        format!("app/{}/{}", cat.name, file.source_rel.display());
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

        // Aggregate: detect a partial install (mix of NotInstalled and something else).
        let has_installed = file_statuses.iter().any(|s| *s != FileStatus::NotInstalled);
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

        // Determine destination to display.
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

        let (symbol, status_label) = match cat_status {
            FileStatus::Missing => ("!", "destination missing (was installed)"),
            FileStatus::UserModified => ("~", "user modified"),
            FileStatus::Partial => ("~", "partial install"),
            FileStatus::UpdateAvail => ("↑", "update available — run `shine app install`"),
            FileStatus::UpToDate => ("✓", "up-to-date"),
            FileStatus::NotInstalled => ("✗", "not installed"),
        };

        let dest_part = dest_display
            .map(|d| format!("  →  {}", d))
            .unwrap_or_default();
        println!(
            "  {}  {}{}  ({})",
            colors::symbol(symbol),
            cat.name,
            dest_part,
            status_label
        );

        match cat_status {
            FileStatus::Missing => missing += 1,
            FileStatus::UserModified | FileStatus::Partial => user_modified += 1,
            FileStatus::UpdateAvail => update_available += 1,
            FileStatus::UpToDate => up_to_date += 1,
            FileStatus::NotInstalled => not_installed += 1,
        }
    }

    let total = up_to_date + update_available + user_modified + missing + not_installed;
    if total == 0 {
        println!("  (no app presets found)");
        return Ok(());
    }

    let mut parts: Vec<String> = Vec::new();
    if up_to_date > 0 {
        parts.push(format!("{} up-to-date", up_to_date));
    }
    if update_available > 0 {
        parts.push(format!("{} update available", update_available));
    }
    if user_modified > 0 {
        parts.push(format!("{} user-modified", user_modified));
    }
    if missing > 0 {
        parts.push(format!("{} destination missing", missing));
    }
    if not_installed > 0 {
        parts.push(format!("{} not installed", not_installed));
    }
    println!("\nSummary: {}", parts.join(", "));

    Ok(())
}
