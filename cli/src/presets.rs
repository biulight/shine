use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
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

pub(crate) struct ScriptInfo {
    pub name: String,
    pub description: Vec<String>,
}

pub(crate) struct CategoryInfo {
    pub name: String,
    pub scripts: Vec<ScriptInfo>,
}

pub(crate) fn asset_paths(prefix: &str) -> Vec<String> {
    let normalized = prefix.trim_end_matches('/');
    let filter = format!("{normalized}/");
    PresetAssets::iter()
        .filter_map(|asset_path| {
            let relative: &str = asset_path.as_ref();
            if relative.starts_with(filter.as_str()) {
                Some(relative.to_string())
            } else {
                None
            }
        })
        .collect()
}

pub(crate) fn read_asset_bytes(path: &str) -> Option<Vec<u8>> {
    PresetAssets::get(path).map(|file| file.data.as_ref().to_vec())
}

/// Extract a `shine-dest:` annotation from a single comment line.
///
/// Recognises `# shine-dest:` (shell/TOML/INI) and `" shine-dest:` (VimScript).
pub(crate) fn extract_annotation_from_line(line: &str) -> Option<String> {
    const PREFIXES: &[&str] = &["# shine-dest:", "\" shine-dest:"];
    for &prefix in PREFIXES {
        if let Some(rest) = line.trim_start().strip_prefix(prefix) {
            let dest = rest.trim().to_string();
            if !dest.is_empty() {
                return Some(dest);
            }
        }
    }
    None
}

/// Parse the `shine-dest:` annotation from the first (or second, if shebang) line.
pub(crate) fn parse_dest_annotation(content: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(content).ok()?;
    let mut lines = text.lines();
    let first = lines.next()?;
    let candidate = if first.starts_with("#!") {
        lines.next()?
    } else {
        first
    };
    extract_annotation_from_line(candidate)
}

/// Parse the leading comment block from a shell script, skipping the shebang line
/// and any `shine-dest:` annotation line.
///
/// Collects consecutive lines starting with `# ` or bare `#` until the first
/// non-comment, non-shebang line. Trailing empty description lines are trimmed.
pub(crate) fn parse_script_description(content: &[u8]) -> Vec<String> {
    let text = std::str::from_utf8(content).unwrap_or("");
    let mut desc = Vec::new();

    for line in text.lines() {
        if line.starts_with("#!") {
            continue;
        }
        if extract_annotation_from_line(line).is_some() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("# ") {
            desc.push(rest.to_string());
        } else if line == "#" {
            desc.push(String::new());
        } else {
            break;
        }
    }

    while desc.last().is_some_and(|l: &String| l.is_empty()) {
        desc.pop();
    }

    desc
}

/// List all preset categories under `prefix/` and their scripts with descriptions.
///
/// Categories are the immediate subdirectories of `prefix/`. Scripts within each
/// category are sorted by name. Returns categories in alphabetical order.
pub(crate) fn list_categories(prefix: &str) -> Vec<CategoryInfo> {
    let normalized = prefix.trim_end_matches('/');
    let filter = format!("{normalized}/");

    let mut map: BTreeMap<String, Vec<ScriptInfo>> = BTreeMap::new();

    for asset_path in PresetAssets::iter() {
        let relative: &str = asset_path.as_ref();
        if !relative.starts_with(filter.as_str()) {
            continue;
        }
        let rest = &relative[filter.len()..];
        let slash = match rest.find('/') {
            Some(p) => p,
            None => continue,
        };
        let category = &rest[..slash];
        let file_name = &rest[slash + 1..];

        if file_name.is_empty() {
            continue;
        }

        let asset_data = PresetAssets::get(relative);
        let description = asset_data
            .as_ref()
            .map(|f| parse_script_description(f.data.as_ref()))
            .unwrap_or_default();
        map.entry(category.to_string())
            .or_default()
            .push(ScriptInfo {
                name: file_name.to_string(),
                description,
            });
    }

    map.into_iter()
        .map(|(name, mut scripts)| {
            scripts.sort_by(|a, b| a.name.cmp(&b.name));
            CategoryInfo { name, scripts }
        })
        .collect()
}

/// List shell preset categories by scanning the filesystem under `presets_dir/shell/`.
///
/// Each immediate subdirectory of `presets_dir/shell/` is a category; `.sh` files within
/// it are the scripts. Descriptions are parsed from each script's leading comment block.
/// Returns categories in alphabetical order.
pub(crate) async fn list_fs_shell_categories(presets_dir: &Path) -> Vec<CategoryInfo> {
    let shell_root = presets_dir.join("shell");
    if !shell_root.is_dir() {
        return Vec::new();
    }

    let mut categories: std::collections::BTreeMap<String, Vec<ScriptInfo>> =
        std::collections::BTreeMap::new();

    let Ok(mut cat_entries) = fs::read_dir(&shell_root).await else {
        return Vec::new();
    };

    while let Ok(Some(cat_entry)) = cat_entries.next_entry().await {
        let Ok(ft) = cat_entry.file_type().await else {
            continue;
        };
        if !ft.is_dir() {
            continue;
        }
        let category = cat_entry.file_name().to_string_lossy().to_string();
        let cat_dir = shell_root.join(&category);

        let Ok(mut script_entries) = fs::read_dir(&cat_dir).await else {
            continue;
        };
        let mut scripts: Vec<ScriptInfo> = Vec::new();
        while let Ok(Some(script_entry)) = script_entries.next_entry().await {
            let Ok(sft) = script_entry.file_type().await else {
                continue;
            };
            if !sft.is_file() {
                continue;
            }
            let name = script_entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".sh") {
                continue;
            }
            let description = fs::read(script_entry.path())
                .await
                .map(|b| parse_script_description(&b))
                .unwrap_or_default();
            scripts.push(ScriptInfo { name, description });
        }
        scripts.sort_by(|a, b| a.name.cmp(&b.name));
        if !scripts.is_empty() {
            categories.insert(category, scripts);
        }
    }

    categories
        .into_iter()
        .map(|(name, scripts)| CategoryInfo { name, scripts })
        .collect()
}

/// Collect full paths to `.sh` files under `presets_dir/<prefix>/`.
///
/// Used by `shell install` when `is_external_presets` is true, to link scripts
/// that already exist on disk without extracting embedded assets.
pub(crate) async fn collect_fs_shell_scripts(
    presets_dir: &Path,
    prefix: &str,
) -> Result<Vec<PathBuf>> {
    let root = presets_dir.join(prefix);
    if !root.is_dir() {
        return Ok(Vec::new());
    }

    let mut scripts = Vec::new();
    let mut stack = vec![root.clone()];

    while let Some(dir) = stack.pop() {
        let mut entries = fs::read_dir(&dir)
            .await
            .with_context(|| format!("reading directory: {}", dir.display()))?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let ft = entry.file_type().await?;
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() && path.extension().is_some_and(|e| e == "sh") {
                scripts.push(path);
            }
        }
    }

    scripts.sort();
    Ok(scripts)
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

    #[test]
    fn parse_description_extracts_comment_block() {
        let script = b"#!/bin/bash\n# First line.\n# Second line.\n\nsome_command\n";
        let desc = parse_script_description(script);
        assert_eq!(desc, vec!["First line.", "Second line."]);
    }

    #[test]
    fn parse_description_skips_shebang_only() {
        let script = b"#!/bin/bash\nsome_command\n";
        let desc = parse_script_description(script);
        assert!(desc.is_empty());
    }

    #[test]
    fn parse_description_handles_bare_hash_as_empty_line() {
        let script = b"#!/bin/bash\n# First.\n#\n# Third.\n";
        let desc = parse_script_description(script);
        assert_eq!(desc, vec!["First.", "", "Third."]);
    }

    #[test]
    fn parse_description_trims_trailing_empty_lines() {
        let script = b"#!/bin/bash\n# First.\n#\n#\n";
        let desc = parse_script_description(script);
        assert_eq!(desc, vec!["First."]);
    }

    #[test]
    fn parse_description_empty_content() {
        let desc = parse_script_description(b"");
        assert!(desc.is_empty());
    }

    #[test]
    fn list_categories_returns_proxy_and_tools() {
        let cats = list_categories("shell");
        let names: Vec<&str> = cats.iter().map(|c| c.name.as_str()).collect();
        assert!(
            names.contains(&"proxy"),
            "proxy category missing: {names:?}"
        );
        assert!(
            names.contains(&"tools"),
            "tools category missing: {names:?}"
        );
    }

    #[test]
    fn list_categories_proxy_scripts_have_descriptions() {
        let cats = list_categories("shell");
        let proxy = cats.iter().find(|c| c.name == "proxy").unwrap();
        for script in &proxy.scripts {
            assert!(
                !script.description.is_empty(),
                "{} should have a description",
                script.name
            );
        }
    }

    #[test]
    fn list_categories_empty_prefix_returns_empty() {
        let cats = list_categories("nonexistent");
        assert!(cats.is_empty());
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

    // --- list_fs_shell_categories tests ---

    #[tokio::test]
    async fn list_fs_shell_categories_returns_empty_for_missing_dir() {
        let missing =
            std::env::temp_dir().join(format!("shine-presets-no-{}", uuid::Uuid::new_v4()));
        let cats = list_fs_shell_categories(&missing).await;
        assert!(cats.is_empty());
    }

    #[tokio::test]
    async fn list_fs_shell_categories_finds_categories_from_disk() {
        let dir = make_temp_dir().await;
        // Create presets/shell/myplugin/hello.sh
        let cat_dir = dir.join("shell/myplugin");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(
            cat_dir.join("hello.sh"),
            b"#!/bin/bash\n# Says hello.\necho hello\n",
        )
        .await
        .unwrap();

        let cats = list_fs_shell_categories(&dir).await;

        assert_eq!(cats.len(), 1, "should find exactly one category");
        assert_eq!(cats[0].name, "myplugin");
        assert_eq!(cats[0].scripts.len(), 1);
        assert_eq!(cats[0].scripts[0].name, "hello.sh");
        assert_eq!(cats[0].scripts[0].description, vec!["Says hello."]);

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn list_fs_shell_categories_ignores_non_sh_files() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("shell/extras");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(cat_dir.join("readme.md"), b"# readme\n")
            .await
            .unwrap();
        fs::write(cat_dir.join("script.sh"), b"#!/bin/bash\n# A script.\n")
            .await
            .unwrap();

        let cats = list_fs_shell_categories(&dir).await;

        assert_eq!(cats.len(), 1);
        assert_eq!(cats[0].scripts.len(), 1, "only .sh files should be listed");
        assert_eq!(cats[0].scripts[0].name, "script.sh");

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn list_fs_shell_categories_returns_alphabetical_order() {
        let dir = make_temp_dir().await;
        for cat in ["zzz", "aaa", "mmm"] {
            let cat_dir = dir.join("shell").join(cat);
            fs::create_dir_all(&cat_dir).await.unwrap();
            fs::write(cat_dir.join("s.sh"), b"#!/bin/bash\n")
                .await
                .unwrap();
        }

        let cats = list_fs_shell_categories(&dir).await;
        let names: Vec<&str> = cats.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["aaa", "mmm", "zzz"]);

        fs::remove_dir_all(&dir).await.unwrap();
    }

    // --- collect_fs_shell_scripts tests ---

    #[tokio::test]
    async fn collect_fs_shell_scripts_returns_empty_for_missing_dir() {
        let missing =
            std::env::temp_dir().join(format!("shine-presets-noscr-{}", uuid::Uuid::new_v4()));
        let scripts = collect_fs_shell_scripts(&missing, "shell").await.unwrap();
        assert!(scripts.is_empty());
    }

    #[tokio::test]
    async fn collect_fs_shell_scripts_finds_sh_files_recursively() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("shell/myplugin");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(cat_dir.join("a.sh"), b"#!/bin/bash\n")
            .await
            .unwrap();
        fs::write(cat_dir.join("b.sh"), b"#!/bin/bash\n")
            .await
            .unwrap();
        fs::write(cat_dir.join("readme.txt"), b"ignore me\n")
            .await
            .unwrap();

        let scripts = collect_fs_shell_scripts(&dir, "shell").await.unwrap();

        let names: Vec<_> = scripts
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert!(names.contains(&"a.sh"), "a.sh missing: {names:?}");
        assert!(names.contains(&"b.sh"), "b.sh missing: {names:?}");
        assert!(!names.contains(&"readme.txt"), "non-.sh should be excluded");

        fs::remove_dir_all(&dir).await.unwrap();
    }
}
