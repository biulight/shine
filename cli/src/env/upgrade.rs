use crate::apps::{AppEntry, AppManifest, apply_transforms, hash_content};
use crate::colors;
use crate::config::Config;
use crate::env::EnvConfig;
use anyhow::Result;
use std::path::PathBuf;

pub(crate) async fn handle_upgrade(config: &Config, dry_run: bool) -> Result<()> {
    let env = EnvConfig::load_or_init(config.shine_dir()).await?;
    let env_map = env.as_map().clone();

    if dry_run {
        println!("{}", colors::dim("[dry-run] No files will be modified."));
    }

    let mut manifest = AppManifest::load(config.shine_dir()).await?;

    let app_candidates: Vec<AppEntry> = manifest
        .entries
        .iter()
        .filter(|e| e.uses_env)
        .cloned()
        .collect();

    let shell_entries = collect_shell_entries(config).await;

    if app_candidates.is_empty() && shell_entries.is_empty() {
        println!(
            "{}",
            colors::dim("No env-templated files found in the manifest.")
        );
        println!(
            "{}",
            colors::dim(
                "Install a preset that uses the `template` transform to enable config upgrade."
            )
        );
        return Ok(());
    }

    let total = app_candidates.len() + shell_entries.len();
    println!(
        "{}  {}",
        colors::bold("Env Templates"),
        colors::dim(&format!("{total} file(s) to check"))
    );

    let mut updated = 0usize;
    let mut skipped = 0usize;
    let mut user_modified = 0usize;

    // --- App manifest entries ---
    for entry in &app_candidates {
        let source_bytes = read_source_bytes(config, &entry.source).await;
        let source_bytes = match source_bytes {
            Some(b) => b,
            None => {
                eprintln!(
                    "  {} {}: source not found, skipped",
                    colors::symbol("!"),
                    entry.source
                );
                skipped += 1;
                continue;
            }
        };

        let transforms = resolve_transforms_for_source(config, &entry.source).await;
        let rendered = match apply_transforms(&transforms, &source_bytes, &env_map) {
            Ok(b) => b,
            Err(e) => {
                eprintln!(
                    "  {} {}: template failed: {e:#}",
                    colors::symbol("✗"),
                    entry.source
                );
                skipped += 1;
                continue;
            }
        };

        let new_hash = hash_content(&rendered);

        // Check if destination was user-modified since last install.
        if let Ok(on_disk) = tokio::fs::read(&entry.destination).await {
            let disk_hash = hash_content(&on_disk);
            if disk_hash != entry.content_hash {
                println!(
                    "  {}  {}  {}",
                    colors::symbol("!"),
                    entry.destination.display(),
                    colors::yellow("user-modified, skipped")
                );
                user_modified += 1;
                continue;
            }
        }

        if new_hash == entry.content_hash {
            println!(
                "  {}  {}",
                colors::dim("-"),
                colors::dim(&entry.destination.display().to_string()),
            );
            skipped += 1;
            continue;
        }

        if dry_run {
            println!(
                "  {}  {}",
                colors::dim("[dry-run]"),
                entry.destination.display(),
            );
            skipped += 1;
            continue;
        }

        if let Some(parent) = entry.destination.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        tokio::fs::write(&entry.destination, &rendered).await?;
        println!(
            "  {}  {}",
            colors::symbol("✓"),
            colors::dim(&entry.destination.display().to_string()),
        );
        manifest.upsert(AppEntry {
            content_hash: new_hash,
            ..entry.clone()
        });
        updated += 1;
    }

    if !dry_run && updated > 0 {
        manifest.save(config.shine_dir()).await?;
    }

    // --- Shell scripts ---
    for entry in &shell_entries {
        // Read template source: embedded assets for non-external, presets_dir for external.
        let source_bytes = read_source_bytes(config, &entry.source_key).await;
        let source_bytes = match source_bytes {
            Some(b) => b,
            None => {
                eprintln!(
                    "  {} {}: source not found, skipped",
                    colors::symbol("!"),
                    entry.rendered_path.display()
                );
                skipped += 1;
                continue;
            }
        };

        if !crate::presets::parse_template_annotation(&source_bytes) {
            skipped += 1;
            continue;
        }

        let rendered = match apply_transforms(&["template".to_string()], &source_bytes, &env_map) {
            Ok(b) => b,
            Err(e) => {
                eprintln!(
                    "  {} {}: template failed: {e:#}",
                    colors::symbol("✗"),
                    entry.rendered_path.display()
                );
                skipped += 1;
                continue;
            }
        };

        // Compare with current rendered output (may not exist yet on first upgrade).
        let current = tokio::fs::read(&entry.rendered_path)
            .await
            .unwrap_or_default();
        if rendered == current {
            println!(
                "  {}  {}",
                colors::dim("-"),
                colors::dim(&entry.rendered_path.display().to_string()),
            );
            skipped += 1;
            continue;
        }

        if dry_run {
            println!(
                "  {}  {}",
                colors::dim("[dry-run]"),
                entry.rendered_path.display(),
            );
            skipped += 1;
            continue;
        }

        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            // Preserve permissions from the presets_dir source file.
            tokio::fs::metadata(&entry.template_path)
                .await
                .map(|m| m.permissions().mode())
                .unwrap_or(0o755)
        };

        if let Some(parent) = entry.rendered_path.parent() {
            tokio::fs::create_dir_all(parent).await.ok();
        }
        tokio::fs::write(&entry.rendered_path, &rendered).await?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(mode);
            let _ = tokio::fs::set_permissions(&entry.rendered_path, perms).await;
        }

        // Migrate old-style bin symlink (presets_dir → rendered_dir) if needed.
        migrate_bin_symlink(config, entry).await;

        println!(
            "  {}  {}",
            colors::symbol("✓"),
            colors::dim(&entry.rendered_path.display().to_string()),
        );
        updated += 1;
    }

    let mut summary: Vec<String> = Vec::new();
    if updated > 0 {
        summary.push(colors::green(&format!("{updated} updated")));
    }
    if user_modified > 0 {
        summary.push(colors::yellow(&format!(
            "{user_modified} user-modified (kept)"
        )));
    }
    if skipped > 0 {
        summary.push(colors::dim(&format!("{skipped} skipped")));
    }
    let sep = colors::dim(" · ");
    println!("\n{}  {}", colors::bold("Done"), summary.join(&sep));

    Ok(())
}

struct ShellEntry {
    /// Key passed to read_source_bytes — e.g. "shell/proxy/set_proxy.sh".
    source_key: String,
    /// Actual template file in presets_dir (used for permission copy).
    template_path: PathBuf,
    /// Target rendered output path in rendered_dir.
    rendered_path: PathBuf,
    /// Bin command name (e.g. "setproxy") for symlink migration.
    command_name: String,
}

/// Collect shell scripts that are installed and may need env re-rendering.
async fn collect_shell_entries(config: &Config) -> Vec<ShellEntry> {
    let shell_root = config.presets_dir().join("shell");
    if !shell_root.exists() {
        return Vec::new();
    }

    let categories = crate::shells::metadata::load_installed_categories(config, None)
        .await
        .unwrap_or_default();

    let mut result = Vec::new();
    for cat in &categories {
        for file in &cat.files {
            let template_path = config
                .presets_dir()
                .join("shell")
                .join(&cat.name)
                .join(&file.source_rel);
            if !template_path.exists() {
                continue;
            }
            let source_key = format!("shell/{}/{}", cat.name, file.source_rel.display());
            let rendered_path = config
                .rendered_dir()
                .join("shell")
                .join(&cat.name)
                .join(&file.source_rel);
            result.push(ShellEntry {
                source_key,
                template_path,
                rendered_path,
                command_name: file.command_name.clone(),
            });
        }
    }
    result
}

/// If the bin symlink for a shell script still points to the old presets_dir location,
/// update it to point to the new rendered_dir location.
async fn migrate_bin_symlink(config: &Config, entry: &ShellEntry) {
    let link_path = config.bin_dir().join(&entry.command_name);
    let target = match tokio::fs::read_link(&link_path).await {
        Ok(t) => t,
        Err(_) => return,
    };
    // Only migrate if the symlink still points inside presets_dir.
    let is_old = if target.is_absolute() {
        target.starts_with(config.presets_dir())
    } else {
        config
            .bin_dir()
            .join(&target)
            .starts_with(config.presets_dir())
    };
    if !is_old {
        return;
    }
    let _ = tokio::fs::remove_file(&link_path).await;
    #[cfg(unix)]
    {
        let _ = tokio::fs::symlink(&entry.rendered_path, &link_path).await;
    }
}

async fn read_source_bytes(config: &Config, source: &str) -> Option<Vec<u8>> {
    if config.is_external_presets {
        let path = config.presets_dir().join(source);
        tokio::fs::read(&path).await.ok()
    } else {
        crate::presets::read_asset_bytes(source)
    }
}

/// Read the transform list for a given source path from installed category metadata.
async fn resolve_transforms_for_source(config: &Config, source: &str) -> Vec<String> {
    // source looks like "app/docker/daemon.jsonc"
    let mut parts = source.splitn(3, '/');
    let _prefix = parts.next(); // "app"
    let cat_name = match parts.next() {
        Some(c) => c,
        None => return Vec::new(),
    };
    let file_rel = match parts.next() {
        Some(f) => f,
        None => return Vec::new(),
    };

    let cats = if config.is_external_presets {
        crate::apps::load_installed_categories(config, Some(cat_name))
            .await
            .unwrap_or_default()
    } else {
        crate::apps::load_embedded_categories(Some(cat_name)).unwrap_or_default()
    };

    cats.iter()
        .find(|c| c.name == cat_name)
        .and_then(|c| {
            c.files
                .iter()
                .find(|f| f.source_rel.to_str() == Some(file_rel))
                .map(|f| f.transforms.clone())
        })
        .unwrap_or_default()
}
