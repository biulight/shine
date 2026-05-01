use crate::apps::{AppEntry, AppManifest, apply_transforms, hash_content};
use crate::colors;
use crate::config::Config;
use crate::env::EnvConfig;
use anyhow::Result;

pub(crate) async fn handle_upgrade(config: &Config, dry_run: bool) -> Result<()> {
    let env = EnvConfig::load_or_init(config.shine_dir()).await?;
    let env_map = env.as_map();

    if dry_run {
        println!("{}", colors::dim("[dry-run] No files will be modified."));
    }

    let mut manifest = AppManifest::load(config.shine_dir()).await?;

    let candidates: Vec<AppEntry> = manifest
        .entries
        .iter()
        .filter(|e| e.uses_env)
        .cloned()
        .collect();

    if candidates.is_empty() {
        println!(
            "{}",
            colors::dim("No env-templated files found in the manifest.")
        );
        println!(
            "{}",
            colors::dim(
                "Install a preset that uses the `template` transform to enable env upgrade."
            )
        );
        return Ok(());
    }

    println!(
        "{}  {}",
        colors::bold("Env Upgrade"),
        colors::dim(&format!("{} file(s) to check", candidates.len()))
    );

    let mut updated = 0usize;
    let mut skipped = 0usize;
    let mut user_modified = 0usize;

    for entry in &candidates {
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
        let rendered = match apply_transforms(&transforms, &source_bytes, env_map) {
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
