//! Shared primitives for `apps::metadata` and `shells::metadata`'s
//! shine.toml/category loaders: embedded-category-name discovery,
//! filesystem-category-name discovery, platform filtering, and the
//! filesystem tree walk + base/overlay merge used to auto-collect files for
//! categories without an explicit `[[files]]` list.
//!
//! Deliberately scoped to just these primitives rather than a full generic
//! loader: the two domains' leaf schemas (`AppCategory`/`ShellCategory`),
//! per-file validation rules, and `Option`-vs-always-`Some` return shapes
//! differ enough that forcing them onto one trait-parameterized loader would
//! be harder to read than the duplication it removes.

use anyhow::{Context, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use tokio::fs;

use crate::config::Config;
use crate::presets;

/// Names of categories under `root` (e.g. `"shell"` or `"app"`) among the
/// embedded assets, optionally filtered to a single name.
pub(crate) fn collect_embedded_category_names(root: &str, filter: Option<&str>) -> Vec<String> {
    let mut names = BTreeSet::new();
    let prefix = format!("{root}/");
    for asset_path in presets::asset_paths(root) {
        let Some(rest) = asset_path.strip_prefix(&prefix) else {
            continue;
        };
        let Some((category, _)) = rest.split_once('/') else {
            continue;
        };
        if filter.is_none_or(|f| f == category) {
            names.insert(category.to_string());
        }
    }
    names.into_iter().collect()
}

/// Names of category subdirectories under `root` on disk, optionally
/// filtered to a single name. `what` labels `root` in the read-directory
/// error context (e.g. `"shell presets directory"`).
pub(crate) async fn collect_fs_category_names(
    root: &Path,
    filter: Option<&str>,
    what: &str,
) -> Result<Vec<String>> {
    if let Some(filter) = filter {
        let path = root.join(filter);
        if path.exists() {
            return Ok(vec![filter.to_string()]);
        }
        return Ok(Vec::new());
    }

    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut names = BTreeSet::new();
    let mut entries = fs::read_dir(root)
        .await
        .with_context(|| format!("reading {what}: {}", root.display()))?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_dir() {
            names.insert(entry.file_name().to_string_lossy().to_string());
        }
    }
    Ok(names.into_iter().collect())
}

/// Shared platform-filter logic for a `shine.toml` file entry's optional
/// `platforms` list. `current` is `"windows"` or `"unix"`
/// (`platform::current_platform()`). `context` labels the offending entry in
/// the error message (e.g. `"shell/proxy/shine.toml"`).
pub(crate) fn platform_matches(
    platforms: Option<&[String]>,
    current: &str,
    context: &str,
) -> Result<bool> {
    let Some(platforms) = platforms else {
        return Ok(true);
    };

    let mut matches = false;
    for platform in platforms {
        let normalized = platform.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "windows" | "unix" => matches |= normalized == current,
            _ => anyhow::bail!(
                "{context} has unsupported platform `{platform}`; expected `windows` or `unix`"
            ),
        }
    }
    Ok(matches)
}

/// Recursively walks `root`, returning the sorted paths for which `keep`
/// returns `Some`. `keep` receives each file's path relative to `root` and
/// both filters (return `None` to skip) and normalizes/validates it (return
/// `Err` to propagate a validation failure, e.g. an invalid file name).
/// `what` labels `root`/subdirectories in the read-directory error context.
pub(crate) async fn collect_fs_tree(
    root: &Path,
    what: &str,
    keep: impl Fn(&Path) -> Result<Option<PathBuf>>,
) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = fs::read_dir(&dir)
            .await
            .with_context(|| format!("reading {what}: {}", dir.display()))?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let ft = entry.file_type().await?;
            if ft.is_dir() {
                stack.push(path);
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            let rel = path.strip_prefix(root).with_context(|| {
                format!(
                    "failed to resolve {} relative to {}",
                    path.display(),
                    root.display()
                )
            })?;
            if let Some(normalized) = keep(rel)? {
                out.push(normalized);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// [`collect_fs_tree`] over `config.presets_dir()`, merged with
/// `config.active_presets_overlay_dir()` if set — the base/overlay merge
/// shared by every "auto-collect files for a category with no explicit
/// `[[files]]` list" path. `category_rel` is the category's path relative to
/// the presets root, e.g. `Path::new("shell").join(name)`.
pub(crate) async fn merge_fs_tree(
    config: &Config,
    category_rel: &Path,
    what: &str,
    keep: impl Fn(&Path) -> Result<Option<PathBuf>> + Copy,
) -> Result<Vec<PathBuf>> {
    let base_category = config.presets_dir().join(category_rel);
    let mut items: BTreeSet<PathBuf> = if base_category.is_dir() {
        collect_fs_tree(&base_category, what, keep)
            .await?
            .into_iter()
            .collect()
    } else {
        BTreeSet::new()
    };
    if let Some(overlay) = config.active_presets_overlay_dir() {
        let overlay_category = overlay.join(category_rel);
        if overlay_category.is_dir() {
            items.extend(collect_fs_tree(&overlay_category, what, keep).await?);
        }
    }
    Ok(items.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_matches_defaults_to_true_when_unset() {
        assert!(platform_matches(None, "windows", "ctx").unwrap());
    }

    #[test]
    fn platform_matches_checks_current_platform() {
        let platforms = vec!["windows".to_string()];
        assert!(platform_matches(Some(&platforms), "windows", "ctx").unwrap());
        assert!(!platform_matches(Some(&platforms), "unix", "ctx").unwrap());
    }

    #[test]
    fn platform_matches_rejects_unknown_platform_with_context() {
        let platforms = vec!["plan9".to_string()];
        let err = platform_matches(Some(&platforms), "unix", "app/foo/shine.toml")
            .unwrap_err()
            .to_string();
        assert!(err.contains("app/foo/shine.toml"));
        assert!(err.contains("unsupported platform"));
    }

    #[test]
    fn collect_embedded_category_names_filters_to_requested_name() {
        let names = collect_embedded_category_names("shell", Some("proxy"));
        assert_eq!(names, vec!["proxy".to_string()]);
    }

    #[tokio::test]
    async fn collect_fs_tree_keeps_only_entries_the_predicate_returns_some_for() {
        let dir = crate::test_support::make_temp_dir("shine-preset-meta").await;
        tokio::fs::write(dir.join("keep.txt"), b"").await.unwrap();
        tokio::fs::write(dir.join("skip.txt"), b"").await.unwrap();
        tokio::fs::create_dir_all(dir.join("nested")).await.unwrap();
        tokio::fs::write(dir.join("nested/keep2.txt"), b"")
            .await
            .unwrap();

        let result = collect_fs_tree(&dir, "test dir", |rel| {
            if rel.file_name().and_then(|n| n.to_str()) == Some("skip.txt") {
                Ok(None)
            } else {
                Ok(Some(rel.to_path_buf()))
            }
        })
        .await
        .unwrap();

        assert_eq!(
            result,
            vec![PathBuf::from("keep.txt"), PathBuf::from("nested/keep2.txt"),]
        );

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn collect_fs_tree_propagates_keep_errors() {
        let dir = crate::test_support::make_temp_dir("shine-preset-meta").await;
        tokio::fs::write(dir.join("bad.txt"), b"").await.unwrap();

        let result = collect_fs_tree(&dir, "test dir", |_rel| anyhow::bail!("invalid entry")).await;

        assert!(result.is_err());
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn merge_fs_tree_combines_base_and_overlay() {
        let dir = crate::test_support::make_temp_dir("shine-preset-meta").await;
        let base = dir.join("presets/app/sample");
        let overlay_root = dir.join("overlay");
        let overlay = overlay_root.join("app/sample");
        tokio::fs::create_dir_all(&base).await.unwrap();
        tokio::fs::create_dir_all(&overlay).await.unwrap();
        tokio::fs::write(base.join("a.txt"), b"").await.unwrap();
        tokio::fs::write(overlay.join("b.txt"), b"").await.unwrap();

        let mut config = crate::test_support::test_config(&dir);
        config.presets_overlay_dir_override = Some(overlay_root);

        let result = merge_fs_tree(&config, Path::new("app/sample"), "test dir", |rel| {
            Ok(Some(rel.to_path_buf()))
        })
        .await
        .unwrap();

        assert_eq!(result, vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")]);

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
