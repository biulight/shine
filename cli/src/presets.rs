use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use tokio::fs;

#[derive(rust_embed::RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/../presets"]
struct PresetAssets;

pub(crate) struct ExtractReport {
    pub created: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
    pub overwritten: Vec<PathBuf>,
}

pub(crate) struct RemoveReport {
    pub removed: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
}

/// Remove embedded-asset files under `prefix/` from `target_dir`.
///
/// Only files known to `PresetAssets` are candidates — user-added files are
/// never touched. Empty subdirectories within the prefix root are cleaned up
/// after file removal. Missing files are recorded in `skipped`.
/// When `dry_run` is true, nothing is removed.
pub(crate) async fn remove_prefix(
    prefix: &str,
    target_dir: &Path,
    dry_run: bool,
) -> Result<RemoveReport> {
    let normalized = prefix.trim_end_matches('/');
    let filter = format!("{normalized}/");

    let mut report = RemoveReport {
        removed: Vec::new(),
        skipped: Vec::new(),
    };

    let mut dirs_to_check: std::collections::BTreeSet<PathBuf> = Default::default();

    for asset_path in PresetAssets::iter() {
        let relative: &str = asset_path.as_ref();
        if !relative.starts_with(filter.as_str()) {
            continue;
        }
        let dest = target_dir.join(relative);
        if dest.exists() {
            if let Some(parent) = dest.parent() {
                dirs_to_check.insert(parent.to_path_buf());
            }
            if !dry_run {
                fs::remove_file(&dest)
                    .await
                    .with_context(|| format!("removing preset file: {dest:?}"))?;
            }
            report.removed.push(dest);
        } else {
            report.skipped.push(dest);
        }
    }

    if !dry_run {
        // Walk directories deepest-first (BTreeSet sorts lexicographically;
        // reversing gives deepest paths first).
        let prefix_root = target_dir.join(normalized);
        for dir in dirs_to_check.into_iter().rev() {
            if dir.starts_with(&prefix_root) && dir != prefix_root {
                let _ = fs::remove_dir(&dir).await; // ignore error if non-empty
            }
        }
        let _ = fs::remove_dir(&prefix_root).await;
    }

    Ok(report)
}

/// Extract only assets whose path starts with `prefix/`.
pub(crate) async fn extract_prefix(
    prefix: &str,
    target_dir: &Path,
    overwrite: bool,
) -> Result<ExtractReport> {
    let normalized = prefix.trim_end_matches('/');
    let filter = format!("{normalized}/");
    extract_matching(|p| p.starts_with(filter.as_str()), target_dir, overwrite).await
}

/// Extract all embedded assets.
#[allow(dead_code)]
pub(crate) async fn extract_all(target_dir: &Path, overwrite: bool) -> Result<ExtractReport> {
    extract_matching(|_| true, target_dir, overwrite).await
}

async fn extract_matching(
    predicate: impl Fn(&str) -> bool,
    target_dir: &Path,
    overwrite: bool,
) -> Result<ExtractReport> {
    let mut report = ExtractReport {
        created: Vec::new(),
        skipped: Vec::new(),
        overwritten: Vec::new(),
    };

    for asset_path in PresetAssets::iter() {
        let relative: &str = asset_path.as_ref();

        if relative.contains("..") || Path::new(relative).is_absolute() {
            bail!("Unsafe asset path rejected: {relative}");
        }

        if !predicate(relative) {
            continue;
        }

        let dest = target_dir.join(relative);

        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .await
                .with_context(|| format!("creating directory: {parent:?}"))?;
        }

        if dest.exists() && !overwrite {
            report.skipped.push(dest);
            continue;
        }

        let file = PresetAssets::get(relative)
            .with_context(|| format!("embedded asset missing: {relative}"))?;

        let existed = dest.exists();

        fs::write(&dest, file.data.as_ref())
            .await
            .with_context(|| format!("writing preset: {dest:?}"))?;

        #[cfg(unix)]
        if relative.ends_with(".sh") {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&dest)
                .await
                .with_context(|| format!("reading metadata: {dest:?}"))?
                .permissions();
            perms.set_mode(perms.mode() | 0o111);
            fs::set_permissions(&dest, perms)
                .await
                .with_context(|| format!("setting permissions: {dest:?}"))?;
        }

        if existed {
            report.overwritten.push(dest);
        } else {
            report.created.push(dest);
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::fs;

    async fn make_temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shine-presets-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).await.unwrap();
        dir
    }

    #[test]
    fn embedded_assets_not_empty() {
        assert!(PresetAssets::iter().count() > 0, "no assets embedded");
    }

    #[tokio::test]
    async fn extract_prefix_only_extracts_matching_files() {
        let dir = make_temp_dir().await;
        let report = extract_prefix("shell/proxy", &dir, false).await.unwrap();

        assert!(!report.created.is_empty());
        for path in &report.created {
            assert!(
                path.starts_with(dir.join("shell/proxy")),
                "{path:?} should be under shell/proxy/"
            );
        }

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn extract_prefix_shell_only_gets_shell_files() {
        let dir = make_temp_dir().await;
        let report = extract_prefix("shell", &dir, false).await.unwrap();

        assert!(!report.created.is_empty());
        for path in &report.created {
            assert!(
                path.starts_with(dir.join("shell")),
                "{path:?} should be under shell/"
            );
        }

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn extracts_all_files_into_empty_dir() {
        let dir = make_temp_dir().await;
        let report = extract_all(&dir, false).await.unwrap();

        assert!(!report.created.is_empty());
        assert!(report.skipped.is_empty());
        assert!(report.overwritten.is_empty());

        for path in &report.created {
            assert!(path.exists(), "{path:?} should exist");
            let content = fs::read(path).await.unwrap();
            assert!(!content.is_empty(), "{path:?} should not be empty");
        }

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn skips_existing_files_when_overwrite_false() {
        let dir = make_temp_dir().await;
        let marker = b"original content";

        extract_prefix("shell/proxy", &dir, false).await.unwrap();

        let first_file = PresetAssets::iter()
            .find(|p| p.starts_with("shell/proxy/"))
            .unwrap();
        let dest = dir.join(first_file.as_ref());
        fs::write(&dest, marker).await.unwrap();

        let report = extract_prefix("shell/proxy", &dir, false).await.unwrap();
        assert!(!report.skipped.is_empty());

        let content = fs::read(&dest).await.unwrap();
        assert_eq!(content, marker, "existing file should not be overwritten");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn overwrites_when_overwrite_true() {
        let dir = make_temp_dir().await;
        let marker = b"marker";

        extract_prefix("shell/proxy", &dir, false).await.unwrap();

        let first_file = PresetAssets::iter()
            .find(|p| p.starts_with("shell/proxy/"))
            .unwrap();
        let dest = dir.join(first_file.as_ref());
        fs::write(&dest, marker).await.unwrap();

        let report = extract_prefix("shell/proxy", &dir, true).await.unwrap();
        assert!(!report.overwritten.is_empty());

        let content = fs::read(&dest).await.unwrap();
        assert_ne!(content, marker, "file should have been overwritten");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn creates_nested_directories() {
        let dir = make_temp_dir().await;
        extract_prefix("shell", &dir, false).await.unwrap();

        let nested = dir.join("shell").join("proxy");
        assert!(
            nested.is_dir(),
            "shell/proxy/ subdirectory should be created"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn sets_executable_bit_on_sh_files() {
        use std::os::unix::fs::PermissionsExt;

        let dir = make_temp_dir().await;
        let report = extract_prefix("shell", &dir, false).await.unwrap();

        for path in &report.created {
            if path.extension().and_then(|e| e.to_str()) == Some("sh") {
                let mode = fs::metadata(path).await.unwrap().permissions().mode();
                assert!(mode & 0o111 != 0, "{path:?} should be executable");
            }
        }

        fs::remove_dir_all(&dir).await.unwrap();
    }

    // --- remove_prefix tests ---

    #[tokio::test]
    async fn remove_prefix_removes_extracted_files() {
        let dir = make_temp_dir().await;
        let extract = extract_prefix("shell", &dir, false).await.unwrap();
        assert!(!extract.created.is_empty());

        let remove = remove_prefix("shell", &dir, false).await.unwrap();

        assert_eq!(remove.removed.len(), extract.created.len());
        for path in &remove.removed {
            assert!(!path.exists(), "{path:?} should be gone");
        }

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn remove_prefix_leaves_user_added_files() {
        let dir = make_temp_dir().await;
        extract_prefix("shell", &dir, false).await.unwrap();

        let user_file = dir.join("shell").join("my_custom.sh");
        fs::write(&user_file, b"custom").await.unwrap();

        remove_prefix("shell", &dir, false).await.unwrap();

        assert!(user_file.exists(), "user file must survive remove_prefix");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn remove_prefix_is_idempotent() {
        let dir = make_temp_dir().await;
        extract_prefix("shell", &dir, false).await.unwrap();

        remove_prefix("shell", &dir, false).await.unwrap();
        let r2 = remove_prefix("shell", &dir, false).await.unwrap();

        assert!(r2.removed.is_empty());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn remove_prefix_dry_run_mutates_nothing() {
        let dir = make_temp_dir().await;
        let extract = extract_prefix("shell", &dir, false).await.unwrap();

        let report = remove_prefix("shell", &dir, true).await.unwrap();

        assert_eq!(report.removed.len(), extract.created.len());
        for path in &extract.created {
            assert!(path.exists(), "{path:?} should still exist after dry-run");
        }

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn remove_prefix_returns_empty_when_target_dir_missing() {
        let missing =
            std::env::temp_dir().join(format!("shine-presets-miss-{}", uuid::Uuid::new_v4()));

        let report = remove_prefix("shell", &missing, false).await.unwrap();

        assert!(report.removed.is_empty());
        assert!(!missing.exists());
    }
}
