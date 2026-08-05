use anyhow::{Context, Result, bail};
#[cfg(test)]
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tokio::fs;

#[derive(rust_embed::RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/presets"]
struct PresetAssets;

fn overlay_dir_cell() -> &'static Mutex<Option<PathBuf>> {
    static OVERLAY_DIR: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    OVERLAY_DIR.get_or_init(|| Mutex::new(None))
}

pub fn set_overlay_dir(dir: Option<&Path>) {
    let mut guard = overlay_dir_cell()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = dir.map(Path::to_path_buf);
}

fn overlay_dir() -> Option<PathBuf> {
    overlay_dir_cell()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

pub struct ExtractReport {
    pub created: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
    pub overwritten: Vec<PathBuf>,
}

pub struct RemoveReport {
    pub removed: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
}

#[cfg(test)]
pub struct ScriptInfo {
    pub name: String,
    pub description: Vec<String>,
}

#[cfg(test)]
pub struct CategoryInfo {
    pub name: String,
    pub scripts: Vec<ScriptInfo>,
}

pub fn asset_paths(prefix: &str) -> Vec<String> {
    let normalized = prefix.trim_end_matches('/');
    let mut paths: BTreeSet<_> = embedded_asset_paths(normalized).into_iter().collect();
    if let Some(dir) = overlay_dir() {
        collect_overlay_paths(&dir, normalized, &mut paths);
    }
    paths.into_iter().collect()
}

/// Return paths from the binary's embedded preset bundle only.
///
/// Unlike [`asset_paths`], this deliberately excludes the active overlay. It is
/// used when users ask for a pristine copy of what the current binary ships.
pub fn embedded_asset_paths(prefix: &str) -> Vec<String> {
    let normalized = prefix.trim_end_matches('/');
    let filter = if normalized.is_empty() {
        String::new()
    } else {
        format!("{normalized}/")
    };
    let mut paths = BTreeSet::new();
    for asset_path in PresetAssets::iter() {
        let relative: &str = asset_path.as_ref();
        if filter.is_empty() || relative.starts_with(filter.as_str()) {
            paths.insert(relative.to_string());
        }
    }
    paths.into_iter().collect()
}

pub fn read_asset_bytes(path: &str) -> Option<Vec<u8>> {
    if !is_safe_asset_path(path) {
        return None;
    }
    if let Some(dir) = overlay_dir() {
        let overlay_path = dir.join(path);
        if overlay_path.is_file()
            && let Ok(bytes) = std::fs::read(&overlay_path)
        {
            return Some(bytes);
        }
    }
    read_embedded_asset_bytes(path)
}

/// Read a file from the binary's embedded preset bundle, ignoring overlays.
pub fn read_embedded_asset_bytes(path: &str) -> Option<Vec<u8>> {
    if !is_safe_asset_path(path) {
        return None;
    }
    PresetAssets::get(path).map(|file| file.data.as_ref().to_vec())
}

fn collect_overlay_paths(root: &Path, prefix: &str, out: &mut BTreeSet<String>) {
    let prefix_path = root.join(prefix);
    if !prefix_path.is_dir() {
        return;
    }

    let mut stack = vec![prefix_path];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let Ok(rel) = path.strip_prefix(root) else {
                continue;
            };
            let Some(rel) = rel.to_str() else {
                continue;
            };
            let rel = rel.replace('\\', "/");
            if is_safe_asset_path(&rel) {
                out.insert(rel);
            }
        }
    }
}

fn is_safe_asset_path(path: &str) -> bool {
    !path.contains("..") && !Path::new(path).is_absolute()
}

/// Extract a `shine-dest:` annotation from a single comment line.
///
/// Recognises `# shine-dest:` (shell/TOML/INI) and `" shine-dest:` (VimScript).
pub fn extract_annotation_from_line(line: &str) -> Option<String> {
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
pub fn parse_dest_annotation(content: &[u8]) -> Option<String> {
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

/// Return `true` if the script opts into env-variable substitution.
///
/// Looks for `# shine-template: true` in the shebang or leading comment block.
pub fn parse_template_annotation(content: &[u8]) -> bool {
    let text = match std::str::from_utf8(content) {
        Ok(t) => t,
        Err(_) => return false,
    };
    for line in text.lines() {
        if line.starts_with("#!") {
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed == "# shine-template: true" {
            return true;
        }
        // Stop scanning once we leave the leading comment block.
        if !trimmed.starts_with('#') && !trimmed.is_empty() {
            break;
        }
    }
    false
}

/// Parse the leading comment block from a shell script, skipping the shebang line
/// and any `shine-dest:` annotation line.
///
/// Collects consecutive lines starting with `# ` or bare `#` until the first
/// non-comment, non-shebang line. Trailing empty description lines are trimmed.
pub fn parse_script_description(content: &[u8]) -> Vec<String> {
    let Ok(text) = std::str::from_utf8(content) else {
        return vec![];
    };
    let mut desc = Vec::new();

    for line in text.lines() {
        if line.starts_with("#!") {
            continue;
        }
        if extract_annotation_from_line(line).is_some() {
            continue;
        }
        if line.trim_start() == "# shine-template: true" {
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

/// Parse a leading `//` comment block from a bun source (`.ts`/`.js`/`.mts`/`.mjs`)
/// as its description — the JS/TS-comment mirror of `parse_script_description`'s
/// `#` handling for `.sh`/`.ps1`. An optional `#!` shebang is skipped; `// ` lines
/// are collected (a bare `//` is a blank line); the first non-comment line ends the
/// block; trailing blank lines are trimmed. There are no `shine-dest`/`shine-template`
/// annotations to skip here — those are `#`/`.sh`-only.
pub fn parse_bun_description(content: &[u8]) -> Vec<String> {
    let Ok(text) = std::str::from_utf8(content) else {
        return vec![];
    };
    let mut desc = Vec::new();

    for line in text.lines() {
        if line.starts_with("#!") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("// ") {
            desc.push(rest.to_string());
        } else if line.trim_end() == "//" {
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
#[cfg(test)]
pub fn list_categories(prefix: &str) -> Vec<CategoryInfo> {
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

        if file_name.is_empty() || !file_name.ends_with(".sh") {
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
#[cfg(test)]
pub async fn list_fs_shell_categories(presets_dir: &Path) -> Vec<CategoryInfo> {
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
#[cfg(test)]
pub async fn collect_fs_shell_scripts(presets_dir: &Path, prefix: &str) -> Result<Vec<PathBuf>> {
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
pub async fn remove_prefix(prefix: &str, target_dir: &Path, dry_run: bool) -> Result<RemoveReport> {
    let normalized = prefix.trim_end_matches('/');

    let mut report = RemoveReport {
        removed: Vec::new(),
        skipped: Vec::new(),
    };

    let mut dirs_to_check: std::collections::BTreeSet<PathBuf> = Default::default();

    for relative in asset_paths(normalized) {
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
pub async fn extract_prefix(
    prefix: &str,
    target_dir: &Path,
    overwrite: bool,
) -> Result<ExtractReport> {
    let normalized = prefix.trim_end_matches('/');
    let filter = format!("{normalized}/");
    extract_matching(
        asset_paths(""),
        |p| p.starts_with(filter.as_str()),
        read_asset_bytes,
        target_dir,
        overwrite,
    )
    .await
}

/// Extract one prefix from the binary's embedded preset bundle only.
pub async fn extract_embedded_prefix(
    prefix: &str,
    target_dir: &Path,
    overwrite: bool,
) -> Result<ExtractReport> {
    let normalized = prefix.trim_end_matches('/');
    let filter = format!("{normalized}/");
    extract_matching(
        embedded_asset_paths(""),
        |p| p.starts_with(filter.as_str()),
        read_embedded_asset_bytes,
        target_dir,
        overwrite,
    )
    .await
}

/// Extract all embedded assets.
pub async fn extract_all(target_dir: &Path, overwrite: bool) -> Result<ExtractReport> {
    extract_matching(
        asset_paths(""),
        |_| true,
        read_asset_bytes,
        target_dir,
        overwrite,
    )
    .await
}

async fn extract_matching(
    paths: impl IntoIterator<Item = String>,
    predicate: impl Fn(&str) -> bool,
    read: impl Fn(&str) -> Option<Vec<u8>>,
    target_dir: &Path,
    overwrite: bool,
) -> Result<ExtractReport> {
    let mut report = ExtractReport {
        created: Vec::new(),
        skipped: Vec::new(),
        overwritten: Vec::new(),
    };

    for relative in paths {
        let relative = relative.as_str();

        if !is_safe_asset_path(relative) {
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

        let file = read(relative).with_context(|| format!("preset asset missing: {relative}"))?;

        let existed = dest.exists();

        fs::write(&dest, &file)
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
    use std::sync::OnceLock;
    use tokio::fs;

    fn overlay_lock_mutex() -> &'static tokio::sync::Mutex<()> {
        static OVERLAY_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        OVERLAY_LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
    }

    /// Async-test variant: holds the lock across `.await` points safely.
    async fn overlay_lock() -> tokio::sync::MutexGuard<'static, ()> {
        overlay_lock_mutex().lock().await
    }

    /// Sync-test variant: used by plain `#[test]` functions with no Tokio runtime.
    fn overlay_lock_sync() -> tokio::sync::MutexGuard<'static, ()> {
        overlay_lock_mutex().blocking_lock()
    }

    async fn make_temp_dir() -> PathBuf {
        crate::test_support::make_temp_dir("shine-presets").await
    }

    #[test]
    fn embedded_assets_not_empty() {
        assert!(PresetAssets::iter().count() > 0, "no assets embedded");
    }

    #[tokio::test]
    async fn overlay_asset_paths_include_new_files() {
        let dir = make_temp_dir().await;
        fs::create_dir_all(dir.join("shell/personal"))
            .await
            .unwrap();
        fs::write(dir.join("shell/personal/hello.sh"), b"#!/bin/bash\n")
            .await
            .unwrap();

        let guard = overlay_lock().await;
        set_overlay_dir(Some(&dir));
        let paths = asset_paths("shell");
        set_overlay_dir(None);
        drop(guard);

        assert!(paths.contains(&"shell/personal/hello.sh".to_string()));
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn overlay_read_asset_bytes_overrides_embedded_file() {
        let dir = make_temp_dir().await;
        fs::create_dir_all(dir.join("shell/proxy")).await.unwrap();
        fs::write(dir.join("shell/proxy/set_proxy.sh"), b"overlay\n")
            .await
            .unwrap();

        let guard = overlay_lock().await;
        set_overlay_dir(Some(&dir));
        let bytes = read_asset_bytes("shell/proxy/set_proxy.sh").unwrap();
        set_overlay_dir(None);
        drop(guard);

        assert_eq!(bytes, b"overlay\n");
        fs::remove_dir_all(&dir).await.unwrap();
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
    fn parse_bun_description_extracts_slash_comment_block() {
        let script = b"// First line.\n// Second line.\nconsole.log('hi')\n";
        let desc = parse_bun_description(script);
        assert_eq!(desc, vec!["First line.", "Second line."]);
    }

    #[test]
    fn parse_bun_description_skips_shebang_and_stops_at_code() {
        let script = b"#!/usr/bin/env bun\n// Only line.\nexport const x = 1\n";
        let desc = parse_bun_description(script);
        assert_eq!(desc, vec!["Only line."]);
    }

    #[test]
    fn parse_bun_description_handles_bare_slash_as_empty_line() {
        let script = b"// First.\n//\n// Third.\n";
        let desc = parse_bun_description(script);
        assert_eq!(desc, vec!["First.", "", "Third."]);
    }

    #[test]
    fn parse_bun_description_empty_when_starts_with_code() {
        let script = b"import { foo } from './foo'\n// not a header\n";
        let desc = parse_bun_description(script);
        assert!(desc.is_empty());
    }

    #[test]
    fn parse_description_empty_content() {
        let desc = parse_script_description(b"");
        assert!(desc.is_empty());
    }

    #[test]
    fn list_categories_returns_proxy_and_utils() {
        let _guard = overlay_lock_sync();
        let cats = list_categories("shell");
        let names: Vec<&str> = cats.iter().map(|c| c.name.as_str()).collect();
        assert!(
            names.contains(&"proxy"),
            "proxy category missing: {names:?}"
        );
        assert!(
            names.contains(&"utils"),
            "utils category missing: {names:?}"
        );
    }

    #[test]
    fn list_categories_proxy_scripts_have_descriptions() {
        let _guard = overlay_lock_sync();
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
        let _guard = overlay_lock_sync();
        let cats = list_categories("nonexistent");
        assert!(cats.is_empty());
    }

    #[tokio::test]
    async fn extract_prefix_only_extracts_matching_files() {
        let _guard = overlay_lock().await;
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
        let _guard = overlay_lock().await;
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
        let _guard = overlay_lock().await;
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
        let _guard = overlay_lock().await;
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
        let _guard = overlay_lock().await;
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
        let _guard = overlay_lock().await;
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
        let _guard = overlay_lock().await;
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
        let _guard = overlay_lock().await;
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
        let _guard = overlay_lock().await;
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
        let _guard = overlay_lock().await;
        let dir = make_temp_dir().await;
        extract_prefix("shell", &dir, false).await.unwrap();

        remove_prefix("shell", &dir, false).await.unwrap();
        let r2 = remove_prefix("shell", &dir, false).await.unwrap();

        assert!(r2.removed.is_empty());

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn remove_prefix_dry_run_mutates_nothing() {
        let _guard = overlay_lock().await;
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
        let _guard = overlay_lock().await;
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

    // --- extract_all (presets export) tests ---

    #[tokio::test]
    async fn extract_all_creates_files_in_target_dir() {
        let _guard = overlay_lock().await;
        let dir = make_temp_dir().await;
        let report = extract_all(&dir, false).await.unwrap();

        assert!(
            !report.created.is_empty(),
            "should create at least one file"
        );
        assert!(report.skipped.is_empty());
        assert!(report.overwritten.is_empty());

        for path in &report.created {
            assert!(path.exists(), "{path:?} should exist after export");
        }

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn extract_all_skips_existing_by_default() {
        let _guard = overlay_lock().await;
        let dir = make_temp_dir().await;

        // First export — populates the dir
        let first = extract_all(&dir, false).await.unwrap();
        assert!(!first.created.is_empty());

        // Overwrite one file with marker content
        let marker = b"do-not-overwrite";
        let target_path = &first.created[0];
        fs::write(target_path, marker).await.unwrap();

        // Second export without --force — should skip the modified file
        let second = extract_all(&dir, false).await.unwrap();
        assert!(
            second.skipped.contains(target_path),
            "modified file should be skipped on re-export without --force"
        );

        let content = fs::read(target_path).await.unwrap();
        assert_eq!(
            content, marker,
            "file content must not change without --force"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn extract_all_force_overwrites_existing() {
        let _guard = overlay_lock().await;
        let dir = make_temp_dir().await;

        // First export
        let first = extract_all(&dir, false).await.unwrap();
        assert!(!first.created.is_empty());

        // Overwrite one file with marker content
        let marker = b"old-content";
        let target_path = &first.created[0];
        fs::write(target_path, marker).await.unwrap();

        // Re-export with force
        let second = extract_all(&dir, true).await.unwrap();
        assert!(
            second.overwritten.contains(target_path),
            "modified file should appear in overwritten list with --force"
        );

        let content = fs::read(target_path).await.unwrap();
        assert_ne!(
            content, marker,
            "file content should be overwritten with --force"
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn extract_embedded_prefix_ignores_active_overlay() {
        let _guard = overlay_lock().await;
        let overlay = make_temp_dir().await;
        let target = make_temp_dir().await;
        let overlay_file = overlay.join("app/clash-verge/merge.yaml");
        fs::create_dir_all(overlay_file.parent().unwrap())
            .await
            .unwrap();
        fs::write(&overlay_file, "overlay-only marker")
            .await
            .unwrap();
        set_overlay_dir(Some(&overlay));

        let report = extract_embedded_prefix("app/clash-verge", &target, false)
            .await
            .unwrap();
        set_overlay_dir(None);

        assert!(!report.created.is_empty());
        let copied = fs::read_to_string(target.join("app/clash-verge/merge.yaml"))
            .await
            .unwrap();
        assert_ne!(copied, "overlay-only marker");
        fs::remove_dir_all(overlay).await.unwrap();
        fs::remove_dir_all(target).await.unwrap();
    }
}
