//! Terminal adapter and explicit output write for deterministic Preset bundles.

use crate::commands::PresetReportFormat;
use anyhow::Result;
use shine_core::runtime::PresetPackReportV3;
use std::path::{Path, PathBuf};

pub async fn handle_pack(
    path: &Path,
    output: &Path,
    force: bool,
    format: PresetReportFormat,
) -> Result<bool> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let mut artifact =
        shine_core::runtime::pack_preset_path(&shine_core::runtime::RealHost, &cwd, path).await;
    if artifact.report.valid {
        let category_input = if path.ends_with("shine.toml") {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        let output = absolute(&cwd, output);
        match resolved_output_path(&output).and_then(|output| {
            Ok((
                std::fs::canonicalize(absolute(&cwd, category_input))?,
                output,
            ))
        }) {
            Ok((category, resolved)) if resolved.starts_with(&category) => {
                invalidate(&mut artifact.report, "output_inside_category");
            }
            Ok((_, resolved)) => {
                // Rename replaces the final path entry rather than following its symlink.
                // Count even a dangling final symlink as an existing output.
                if resolved.symlink_metadata().is_ok() && !force {
                    invalidate(&mut artifact.report, "output_exists");
                } else if shine_core::persist::atomic_write(&resolved, &artifact.bytes)
                    .await
                    .is_err()
                {
                    invalidate(&mut artifact.report, "output_write_failed");
                }
            }
            Err(_) => invalidate(&mut artifact.report, "output_write_failed"),
        }
    }

    match format {
        PresetReportFormat::Text => print_text_report(&artifact.report),
        PresetReportFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&artifact.report)?)
        }
    }
    Ok(artifact.report.valid)
}

fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

// Resolve existing ancestors in order: `link/..` must follow the link before
// taking its parent. Nonexistent output directories stay virtual until validation.
fn resolved_output_path(output: &Path) -> std::io::Result<PathBuf> {
    use std::path::Component;
    let parent = output
        .parent()
        .ok_or_else(|| std::io::Error::other("output has no parent"))?;
    let name = output
        .file_name()
        .ok_or_else(|| std::io::Error::other("output has no filename"))?;
    let mut resolved = PathBuf::new();
    for part in parent.components() {
        match part {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir => resolved.push(part.as_os_str()),
            Component::Normal(_) => {
                resolved.push(part.as_os_str());
                match std::fs::symlink_metadata(&resolved) {
                    Ok(_) => {
                        resolved = std::fs::canonicalize(&resolved)?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
        }
    }
    Ok(resolved.join(name))
}

fn invalidate(report: &mut PresetPackReportV3, code: &str) {
    report.valid = false;
    report.files = 0;
    report.archive_bytes = 0;
    report.bundle_sha256 = None;
    report.diagnostics.push(code.to_string());
}

fn print_text_report(report: &PresetPackReportV3) {
    println!(
        "Preset pack: {}",
        if report.valid { "created" } else { "blocked" }
    );
    if let Some(target) = &report.target {
        println!("  Target: {target}");
    }
    for diagnostic in &report.diagnostics {
        println!("  error[{diagnostic}]");
    }
    if report.valid {
        println!("  Files: {}", report.files);
        if report.unisolated_code {
            println!("  Code: unisolated executable entry");
        }
        if let Some(source) = &report.author_capability_source {
            println!("  Author capability statement: {source} (unverified)");
        }
        println!("  Archive bytes: {}", report.archive_bytes);
        println!(
            "  SHA-256: {}",
            report.bundle_sha256.as_deref().unwrap_or_default()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn category_fixture() -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("shine-pack-output-{}", uuid::Uuid::new_v4()));
        let category = root.join("app/demo");
        std::fs::create_dir_all(&category).unwrap();
        std::fs::write(category.join("shine.toml"), "dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.txt'\n").unwrap();
        std::fs::write(category.join("config.txt"), "demo").unwrap();
        (root, category)
    }

    #[tokio::test]
    async fn pack_resolves_parent_components_before_output_scope_check() {
        let (root, category) = category_fixture();
        let output = root.join("app/../app/demo/new/bundle.tar.gz");
        assert!(
            !handle_pack(&category, &output, true, PresetReportFormat::Json)
                .await
                .unwrap()
        );
        assert!(!category.join("new").exists());
        let outside = category.join("../bundles/release.tar.gz");
        assert!(
            handle_pack(
                &category.join("../demo/shine.toml"),
                &outside,
                false,
                PresetReportFormat::Json
            )
            .await
            .unwrap()
        );
        assert!(root.join("app/bundles/release.tar.gz").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pack_rejects_output_through_symlink_into_category() {
        use std::os::unix::fs::symlink;
        let (root, category) = category_fixture();
        let alias = root.join("alias");
        symlink(&category, &alias).unwrap();
        for input in [&category, &alias] {
            for force in [false, true] {
                assert!(
                    !handle_pack(
                        input,
                        &alias.join("new/bundle.tar.gz"),
                        force,
                        PresetReportFormat::Json
                    )
                    .await
                    .unwrap()
                );
                assert!(!category.join("new").exists());
            }
        }
        // Taking a symlink's parent uses the linked directory, not the link's location.
        assert_eq!(
            resolved_output_path(&alias.join("../release.tar.gz")).unwrap(),
            std::fs::canonicalize(root.join("app"))
                .unwrap()
                .join("release.tar.gz")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pack_requires_force_for_dangling_output_symlink() {
        use std::os::unix::fs::symlink;
        let (root, category) = category_fixture();
        let output = root.join("bundle.tar.gz");
        let target = category.join("missing");
        symlink(&target, &output).unwrap();
        assert!(
            !handle_pack(&category, &output, false, PresetReportFormat::Json)
                .await
                .unwrap()
        );
        assert!(output.is_symlink());
        assert!(
            handle_pack(&category, &output, true, PresetReportFormat::Json)
                .await
                .unwrap()
        );
        assert!(!output.is_symlink());
        assert!(!target.exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
