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
use crate::platform::OperatingSystem;
use crate::presets;

#[cfg(test)]
pub(crate) fn collect_pristine_embedded_category_names(root: &str) -> Vec<String> {
    let mut names = BTreeSet::new();
    let prefix = format!("{root}/");
    for asset_path in presets::embedded_asset_paths(root) {
        let Some(rest) = asset_path.strip_prefix(&prefix) else {
            continue;
        };
        let Some((category, _)) = rest.split_once('/') else {
            continue;
        };
        names.insert(category.to_string());
    }
    names.into_iter().collect()
}

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
/// `platforms` list. Exact selectors are `macos`, `linux`, and `windows`;
/// `unix` is the compatibility group for macOS and Linux. `context` labels the offending entry in
/// the error message (e.g. `"shell/proxy/shine.toml"`).
pub(crate) fn platform_matches(
    platforms: Option<&[String]>,
    current: OperatingSystem,
    context: &str,
) -> Result<bool> {
    let Some(platforms) = platforms else {
        return Ok(true);
    };
    if platforms.is_empty() {
        anyhow::bail!(
            "{context} platforms must not be empty; expected `macos`, `linux`, `windows`, or `unix`"
        );
    }

    let mut matches = false;
    for platform in platforms {
        let normalized = platform.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "macos" | "linux" | "windows" => matches |= normalized == current.as_str(),
            "unix" => matches |= current.is_unix(),
            _ => anyhow::bail!(
                "{context} has unsupported platform `{platform}`; expected `macos`, `linux`, `windows`, or `unix`"
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

    fn normalize_generated_block(block: &str) -> String {
        String::from_utf8(crate::install_core::normalize_eol(block.as_bytes())).unwrap()
    }

    fn generated_block_replacement(current: &str, expected: &str) -> String {
        if current.contains("\r\n") {
            expected.replace('\n', "\r\n")
        } else {
            expected.to_string()
        }
    }

    #[test]
    fn platform_matches_defaults_to_true_when_unset() {
        assert!(platform_matches(None, OperatingSystem::Windows, "ctx").unwrap());
    }

    #[test]
    fn platform_matches_exact_os_and_unix_group() {
        let macos = vec!["macos".to_string()];
        assert!(platform_matches(Some(&macos), OperatingSystem::Macos, "ctx").unwrap());
        assert!(!platform_matches(Some(&macos), OperatingSystem::Linux, "ctx").unwrap());

        let unix = vec!["unix".to_string()];
        assert!(platform_matches(Some(&unix), OperatingSystem::Macos, "ctx").unwrap());
        assert!(platform_matches(Some(&unix), OperatingSystem::Linux, "ctx").unwrap());
        assert!(!platform_matches(Some(&unix), OperatingSystem::Windows, "ctx").unwrap());
    }

    #[test]
    fn platform_matches_rejects_empty_platforms() {
        let platforms = Vec::new();
        let err = platform_matches(Some(&platforms), OperatingSystem::Linux, "ctx")
            .unwrap_err()
            .to_string();
        assert!(err.contains("must not be empty"));
    }

    #[test]
    fn platform_matches_rejects_unknown_platform_with_context() {
        let platforms = vec!["plan9".to_string()];
        let err = platform_matches(
            Some(&platforms),
            OperatingSystem::Linux,
            "app/foo/shine.toml",
        )
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

    #[test]
    fn built_in_preset_platform_capability_docs_are_current() {
        const START: &str = "<!-- BEGIN GENERATED PRESET PLATFORM CAPABILITIES -->";
        const END: &str = "<!-- END GENERATED PRESET PLATFORM CAPABILITIES -->";

        let mut capabilities = crate::apps::built_in_platform_availability().unwrap();
        capabilities.extend(crate::shells::metadata::built_in_platform_availability().unwrap());

        let mut expected = String::from(START);
        expected.push_str("\n| Preset capability | macOS | Linux | Windows |\n");
        expected.push_str("| --- | --- | --- | --- |\n");
        for (target, platforms) in capabilities {
            let supported = |platform| {
                if platforms.contains(&platform) {
                    "✓"
                } else {
                    "—"
                }
            };
            expected.push_str(&format!(
                "| `{target}` | {} | {} | {} |\n",
                supported(OperatingSystem::Macos),
                supported(OperatingSystem::Linux),
                supported(OperatingSystem::Windows),
            ));
        }
        expected.push_str(END);

        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        if !repository_root.join("docs/manual").is_dir() {
            // Published crates intentionally exclude the documentation repositories.
            return;
        }
        let update = std::env::var_os("SHINE_UPDATE_PRESET_CAPABILITIES").as_deref()
            == Some(std::ffi::OsStr::new("1"));
        for relative in [
            "docs/manual/reference/built-in-presets.md",
            "website/i18n/zh-Hans/docusaurus-plugin-content-docs/current/reference/built-in-presets.md",
        ] {
            let path = repository_root.join(relative);
            let document = std::fs::read_to_string(&path).unwrap();
            let start = document
                .find(START)
                .unwrap_or_else(|| panic!("{} is missing {START}", path.display()));
            let end = document[start..]
                .find(END)
                .map(|offset| start + offset + END.len())
                .unwrap_or_else(|| panic!("{} is missing {END}", path.display()));
            if update {
                let replacement = generated_block_replacement(&document[start..end], &expected);
                let mut updated = document;
                updated.replace_range(start..end, &replacement);
                std::fs::write(&path, updated).unwrap();
                continue;
            }
            assert_eq!(
                normalize_generated_block(&document[start..end]),
                expected,
                "{} has a stale built-in preset platform capability list; replace its generated block with the right-hand value",
                path.display()
            );
        }
    }

    #[test]
    fn generated_capability_blocks_accept_and_preserve_crlf() {
        let expected = "<!-- start -->\n| row |\n<!-- end -->";
        let checked_out = "<!-- start -->\r\n| row |\r\n<!-- end -->";

        assert_eq!(normalize_generated_block(checked_out), expected);
        assert_eq!(
            generated_block_replacement(checked_out, expected),
            checked_out
        );
        assert_eq!(generated_block_replacement(expected, expected), expected);
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
