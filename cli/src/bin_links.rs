use anyhow::{Context, Result};
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[cfg(not(unix))]
const EXECUTABLE_EXTENSIONS: &[&str] = &["sh", "ps1"];

pub(crate) struct LinkReport {
    pub created: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
    pub conflicts: Vec<(PathBuf, PathBuf)>,
    pub overwritten: Vec<PathBuf>,
}

pub(crate) struct UnlinkReport {
    pub removed: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
}

/// Remove symlinks in `bin_dir` whose link target starts with `managed_root`.
///
/// Non-symlinks and symlinks pointing outside `managed_root` are untouched.
/// Missing `bin_dir` is treated as a no-op (returns empty report).
/// When `dry_run` is true, nothing is removed.
pub(crate) async fn unlink_managed(
    bin_dir: &Path,
    managed_root: &Path,
    dry_run: bool,
) -> Result<UnlinkReport> {
    let mut report = UnlinkReport {
        removed: Vec::new(),
        skipped: Vec::new(),
    };

    let mut read_dir = match tokio::fs::read_dir(bin_dir).await {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(report),
        Err(e) => return Err(e).with_context(|| format!("reading bin dir: {bin_dir:?}")),
    };

    while let Some(entry) = read_dir
        .next_entry()
        .await
        .with_context(|| format!("iterating bin dir: {bin_dir:?}"))?
    {
        let path = entry.path();
        let meta = match tokio::fs::symlink_metadata(&path).await {
            Ok(m) => m,
            Err(_) => continue,
        };

        if !meta.file_type().is_symlink() {
            report.skipped.push(path);
            continue;
        }

        let target = match tokio::fs::read_link(&path).await {
            Ok(t) => t,
            Err(_) => {
                report.skipped.push(path);
                continue;
            }
        };

        // Lexical prefix check — works even if the target file no longer exists.
        let is_managed = if target.is_absolute() {
            target.starts_with(managed_root)
        } else {
            // Resolve relative target against the link's parent directory.
            bin_dir.join(&target).starts_with(managed_root)
        };

        if is_managed {
            if !dry_run {
                tokio::fs::remove_file(&path)
                    .await
                    .with_context(|| format!("removing symlink: {path:?}"))?;
            }
            report.removed.push(path);
        } else {
            report.skipped.push(path);
        }
    }

    Ok(report)
}

/// Create flat symlinks in `bin_dir` for each executable file in `sources`.
///
/// - Existing correct symlinks are skipped (idempotent).
/// - Conflicting entries (wrong target or regular file) are recorded and skipped
///   unless `overwrite` is true.
/// - Two sources sharing the same filename → second is recorded as a conflict.
pub(crate) async fn link_executables(
    bin_dir: &Path,
    sources: &[PathBuf],
    overwrite: bool,
) -> Result<LinkReport> {
    let mut report = LinkReport {
        created: Vec::new(),
        skipped: Vec::new(),
        conflicts: Vec::new(),
        overwritten: Vec::new(),
    };

    let mut seen: HashSet<OsString> = HashSet::new();

    for source in sources {
        if !is_executable(source) {
            continue;
        }

        if source.file_name().is_none() {
            continue;
        }
        let stem = link_stem(source);

        if !seen.insert(stem.clone()) {
            report.conflicts.push((bin_dir.join(&stem), source.clone()));
            continue;
        }

        let link_path = bin_dir.join(&stem);

        match tokio::fs::symlink_metadata(&link_path).await {
            Ok(meta) if meta.file_type().is_symlink() => {
                match tokio::fs::read_link(&link_path).await {
                    Ok(existing) if existing == *source => {
                        report.skipped.push(link_path);
                    }
                    _ => {
                        if overwrite {
                            tokio::fs::remove_file(&link_path).await.with_context(|| {
                                format!("removing stale symlink: {link_path:?}")
                            })?;
                            create_symlink(source, &link_path).await?;
                            report.overwritten.push(link_path);
                        } else {
                            report.conflicts.push((link_path, source.clone()));
                        }
                    }
                }
            }
            Ok(_) => {
                if overwrite {
                    tokio::fs::remove_file(&link_path)
                        .await
                        .with_context(|| format!("removing existing file: {link_path:?}"))?;
                    create_symlink(source, &link_path).await?;
                    report.overwritten.push(link_path);
                } else {
                    report.conflicts.push((link_path, source.clone()));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                create_symlink(source, &link_path).await?;
                report.created.push(link_path);
            }
            Err(e) => {
                return Err(e).with_context(|| format!("stat failed: {link_path:?}"));
            }
        }
    }

    Ok(report)
}

fn link_stem(path: &Path) -> std::ffi::OsString {
    match path.extension().and_then(|e| e.to_str()) {
        Some("sh" | "bash" | "zsh" | "fish" | "ps1") => {
            path.file_stem().map(|s| s.to_owned()).unwrap_or_default()
        }
        _ => path.file_name().map(|n| n.to_owned()).unwrap_or_default(),
    }
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|ext| EXECUTABLE_EXTENSIONS.contains(&ext))
            .unwrap_or(false)
    }
}

async fn create_symlink(source: &Path, link_path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        tokio::fs::symlink(source, link_path)
            .await
            .with_context(|| format!("creating symlink {link_path:?} -> {source:?}"))
    }
    #[cfg(not(unix))]
    {
        eprintln!(
            "[shine] bin symlinks not yet supported on this platform; skipping {:?}",
            link_path
        );
        let _ = (source, link_path);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::fs;

    async fn make_dirs() -> (PathBuf, PathBuf) {
        let id = uuid::Uuid::new_v4();
        let src_dir = std::env::temp_dir().join(format!("shine-bl-src-{id}"));
        let bin_dir = std::env::temp_dir().join(format!("shine-bl-bin-{id}"));
        fs::create_dir_all(&src_dir).await.unwrap();
        fs::create_dir_all(&bin_dir).await.unwrap();
        (src_dir, bin_dir)
    }

    /// Write a file and set the executable bit so `is_executable` returns true.
    #[cfg(unix)]
    async fn make_executable(dir: &Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        fs::write(&path, b"#!/bin/sh\n").await.unwrap();
        let mut perms = fs::metadata(&path).await.unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).await.unwrap();
        path
    }

    async fn make_plain(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"data").await.unwrap();
        path
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn creates_symlink_for_executable_source() {
        let (src, bin) = make_dirs().await;
        let exe = make_executable(&src, "run.sh").await;

        let report = link_executables(&bin, std::slice::from_ref(&exe), false)
            .await
            .unwrap();

        assert_eq!(report.created.len(), 1);
        let link = &report.created[0];
        assert!(link.is_symlink());
        assert_eq!(fs::read_link(link).await.unwrap(), exe);
        // symlink name is the stem, not the full filename
        assert_eq!(link.file_name().unwrap(), "run");

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn skips_non_executable_source() {
        let (src, bin) = make_dirs().await;
        let plain = make_plain(&src, "readme.txt").await;

        let report = link_executables(&bin, &[plain], false).await.unwrap();

        assert!(report.created.is_empty());
        assert!(report.skipped.is_empty());

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn skips_when_correct_symlink_already_exists() {
        let (src, bin) = make_dirs().await;
        let exe = make_executable(&src, "run.sh").await;
        tokio::fs::symlink(&exe, bin.join("run")).await.unwrap();

        let report = link_executables(&bin, &[exe], false).await.unwrap();

        assert!(report.created.is_empty());
        assert_eq!(report.skipped.len(), 1);

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reports_conflict_when_regular_file_exists() {
        let (src, bin) = make_dirs().await;
        let exe = make_executable(&src, "run.sh").await;
        make_plain(&bin, "run").await;

        let report = link_executables(&bin, &[exe], false).await.unwrap();

        assert!(report.created.is_empty());
        assert_eq!(report.conflicts.len(), 1);

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn overwrites_stale_symlink_when_overwrite_true() {
        let (src, bin) = make_dirs().await;
        let exe = make_executable(&src, "run.sh").await;
        let other = make_executable(&src, "other.sh").await;
        tokio::fs::symlink(&other, bin.join("run")).await.unwrap();

        let report = link_executables(&bin, std::slice::from_ref(&exe), true)
            .await
            .unwrap();

        assert_eq!(report.overwritten.len(), 1);
        assert_eq!(fs::read_link(bin.join("run")).await.unwrap(), exe);

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn flattens_nested_preset_path_into_bin_dir() {
        let (src, bin) = make_dirs().await;
        let sub = src.join("shell").join("proxy");
        fs::create_dir_all(&sub).await.unwrap();
        let exe = {
            use std::os::unix::fs::PermissionsExt;
            let path = sub.join("set_proxy.sh");
            fs::write(&path, b"#!/bin/sh\n").await.unwrap();
            let mut perms = fs::metadata(&path).await.unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).await.unwrap();
            path
        };

        let report = link_executables(&bin, &[exe], false).await.unwrap();

        assert_eq!(report.created.len(), 1);
        assert!(bin.join("set_proxy").exists());

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reports_collision_when_two_sources_share_basename() {
        let (src, bin) = make_dirs().await;
        let sub1 = src.join("a");
        let sub2 = src.join("b");
        fs::create_dir_all(&sub1).await.unwrap();
        fs::create_dir_all(&sub2).await.unwrap();
        let exe1 = make_executable(&sub1, "run.sh").await;
        let exe2 = make_executable(&sub2, "run.sh").await;

        let report = link_executables(&bin, &[exe1, exe2], false).await.unwrap();

        assert_eq!(report.created.len(), 1);
        assert_eq!(report.conflicts.len(), 1);

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    // --- unlink_managed tests ---

    #[cfg(unix)]
    #[tokio::test]
    async fn unlink_removes_symlink_pointing_into_managed_root() {
        let (src, bin) = make_dirs().await;
        let exe = make_executable(&src, "run.sh").await;
        tokio::fs::symlink(&exe, bin.join("run.sh")).await.unwrap();

        let report = unlink_managed(&bin, &src, false).await.unwrap();

        assert_eq!(report.removed.len(), 1);
        assert!(!bin.join("run.sh").exists());

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unlink_skips_symlink_outside_managed_root() {
        let (src, bin) = make_dirs().await;
        let outside = std::env::temp_dir().join(format!("shine-bl-out-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&outside).await.unwrap();
        let exe = make_executable(&outside, "run.sh").await;
        tokio::fs::symlink(&exe, bin.join("run.sh")).await.unwrap();

        let report = unlink_managed(&bin, &src, false).await.unwrap();

        assert_eq!(report.skipped.len(), 1);
        assert!(bin.join("run.sh").is_symlink());

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
        fs::remove_dir_all(&outside).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unlink_skips_regular_files_in_bin_dir() {
        let (src, bin) = make_dirs().await;
        make_plain(&bin, "user_script.sh").await;

        let report = unlink_managed(&bin, &src, false).await.unwrap();

        assert!(report.removed.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert!(bin.join("user_script.sh").exists());

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unlink_dry_run_reports_but_does_not_remove() {
        let (src, bin) = make_dirs().await;
        let exe = make_executable(&src, "run.sh").await;
        tokio::fs::symlink(&exe, bin.join("run.sh")).await.unwrap();

        let report = unlink_managed(&bin, &src, true).await.unwrap();

        assert_eq!(report.removed.len(), 1);
        assert!(bin.join("run.sh").is_symlink(), "dry-run must not remove");

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unlink_is_idempotent_on_empty_bin_dir() {
        let (src, bin) = make_dirs().await;

        let r1 = unlink_managed(&bin, &src, false).await.unwrap();
        let r2 = unlink_managed(&bin, &src, false).await.unwrap();

        assert!(r1.removed.is_empty());
        assert!(r2.removed.is_empty());

        fs::remove_dir_all(&src).await.unwrap();
        fs::remove_dir_all(&bin).await.unwrap();
    }

    #[tokio::test]
    async fn unlink_returns_empty_report_when_bin_dir_missing() {
        let missing = std::env::temp_dir().join(format!("shine-bl-miss-{}", uuid::Uuid::new_v4()));
        let managed = std::env::temp_dir().join(format!("shine-bl-mgd-{}", uuid::Uuid::new_v4()));

        let report = unlink_managed(&missing, &managed, false).await.unwrap();

        assert!(report.removed.is_empty());
        assert!(report.skipped.is_empty());
    }
}
