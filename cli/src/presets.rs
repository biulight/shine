use crate::output::escape_controls;
use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use tokio::fs;

#[derive(rust_embed::RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/presets"]
struct PresetAssets;

/// A live view of the embedded bundle with one explicitly selected overlay.
///
/// Enumeration and reads share the same source selection. This is not a frozen
/// runtime snapshot; Core continues to receive its snapshot through the host.
pub struct PresetAssetReader<'a> {
    overlay_dir: Option<&'a Path>,
}

impl<'a> PresetAssetReader<'a> {
    pub fn new(overlay_dir: Option<&'a Path>) -> Self {
        Self { overlay_dir }
    }

    pub fn asset_paths(&self, prefix: &str) -> Vec<String> {
        let normalized = prefix.trim_end_matches('/');
        let mut paths: BTreeSet<_> = embedded_asset_paths(normalized).into_iter().collect();
        if let Some(dir) = self.overlay_dir {
            collect_overlay_paths(dir, normalized, &mut paths);
        }
        paths.into_iter().collect()
    }

    pub fn read_asset_bytes(&self, path: &str) -> Option<Vec<u8>> {
        if !is_safe_asset_path(path) {
            return None;
        }
        if let Some(dir) = self.overlay_dir {
            let overlay_path = dir.join(path);
            if overlay_path.is_file()
                && let Ok(bytes) = std::fs::read(&overlay_path)
            {
                return Some(bytes);
            }
        }
        read_embedded_asset_bytes(path)
    }
}

pub struct ExtractReport {
    pub created: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
    pub overwritten: Vec<PathBuf>,
}

/// Return paths from the binary's embedded preset bundle only.
///
/// Unlike [`PresetAssetReader::asset_paths`], this deliberately excludes overlays.
/// It is used when users ask for a pristine copy of what the current binary ships.
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
                if entry.file_name() == "node_modules" {
                    continue;
                }
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

impl PresetAssetReader<'_> {
    /// Extract all assets from the embedded bundle and the selected overlay.
    pub async fn extract_all(&self, target_dir: &Path, overwrite: bool) -> Result<ExtractReport> {
        extract_matching(
            self.asset_paths(""),
            |_| true,
            |path| self.read_asset_bytes(path),
            target_dir,
            overwrite,
        )
        .await
    }
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

async fn extract_matching(
    paths: impl IntoIterator<Item = String>,
    predicate: impl Fn(&str) -> bool,
    read: impl Fn(&str) -> Option<Vec<u8>>,
    target_dir: &Path,
    overwrite: bool,
) -> Result<ExtractReport> {
    fs::create_dir_all(target_dir).await?;
    // An explicitly selected root may be an alias, but nested paths must stay
    // inside this resolved root without following links or Windows junctions.
    let root = fs::canonicalize(target_dir).await?;
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

        let dest = root.join(relative);
        let reported_dest = target_dir.join(relative);

        prepare_extraction_parents(&root, &dest).await?;
        let previous = extraction_file_metadata(&dest).await?;
        if previous.is_some() && !overwrite {
            report.skipped.push(reported_dest);
            continue;
        }

        let file = read(relative).with_context(|| format!("preset asset missing: {relative}"))?;

        if !write_extracted_file(
            &root,
            &dest,
            &file,
            previous.as_ref(),
            overwrite,
            relative.ends_with(".sh"),
        )
        .await?
        {
            report.skipped.push(reported_dest);
            continue;
        }

        if previous.is_some() {
            report.overwritten.push(reported_dest);
        } else {
            report.created.push(reported_dest);
        }
    }

    Ok(report)
}

fn is_extraction_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

async fn prepare_extraction_parents(root: &Path, dest: &Path) -> Result<()> {
    let mut current = root.to_path_buf();
    let root_metadata = fs::symlink_metadata(root).await?;
    if !root_metadata.is_dir() || is_extraction_link(&root_metadata) {
        bail!(
            "preset output root must remain a regular directory: {}",
            escape_controls(&root.display().to_string())
        );
    }
    let parent = dest.parent().context("preset output has no parent")?;
    for component in parent.strip_prefix(root)?.components() {
        if !matches!(component, std::path::Component::Normal(_)) {
            bail!("invalid preset output component");
        }
        current.push(component);
        match fs::create_dir(&current).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "creating {}",
                        escape_controls(&current.display().to_string())
                    )
                });
            }
        }
        let metadata = fs::symlink_metadata(&current).await?;
        if !metadata.is_dir() || is_extraction_link(&metadata) {
            bail!(
                "preset output parent must not be a link or special file: {}",
                escape_controls(&current.display().to_string())
            );
        }
    }
    Ok(())
}

async fn extraction_file_metadata(path: &Path) -> Result<Option<std::fs::Metadata>> {
    match fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.is_file() && !is_extraction_link(&metadata) => Ok(Some(metadata)),
        Ok(_) => bail!(
            "preset output must be a regular file, not a link or special file: {}",
            escape_controls(&path.display().to_string())
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error)
            .with_context(|| format!("checking {}", escape_controls(&path.display().to_string()))),
    }
}

struct ExtractionTemp(PathBuf);

impl Drop for ExtractionTemp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn write_extracted_file(
    root: &Path,
    dest: &Path,
    bytes: &[u8],
    previous: Option<&std::fs::Metadata>,
    overwrite: bool,
    executable: bool,
) -> Result<bool> {
    use tokio::io::AsyncWriteExt;
    prepare_extraction_parents(root, dest).await?;
    let temp_path = dest.with_file_name(format!(".shine-extract-{}", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .await?;
    let temp = ExtractionTemp(temp_path);
    let permissions = previous
        .map(std::fs::Metadata::permissions)
        .unwrap_or(file.metadata().await?.permissions());
    #[cfg(unix)]
    let permissions = if executable {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = permissions;
        permissions.set_mode(permissions.mode() | 0o111);
        permissions
    } else {
        permissions
    };
    #[cfg(not(unix))]
    let _ = executable;
    file.set_permissions(permissions).await?;
    file.write_all(bytes).await?;
    file.sync_all().await?;
    drop(file);
    // Recheck after staging; publication replaces the directory entry instead
    // of following a leaf symlink or modifying another name of a hard link.
    prepare_extraction_parents(root, dest).await?;
    let current = extraction_file_metadata(dest).await?;
    if !overwrite && current.is_some() {
        return Ok(false);
    }
    let result = if overwrite {
        crate::persist::finalize_temp(&temp.0, dest).await
    } else {
        crate::persist::finalize_temp_new(&temp.0, dest).await
    };
    match result {
        Ok(()) => Ok(true),
        Err(error)
            if !overwrite
                && error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::AlreadyExists) =>
        {
            Ok(false)
        }
        Err(error) => Err(error).with_context(|| {
            format!(
                "writing preset: {}",
                escape_controls(&dest.display().to_string())
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn external_shell_runtime(
        root: &Path,
    ) -> shine_core::runtime::CoreRuntime<shine_core::runtime::RealHost> {
        use shine_core::runtime::*;
        let snapshot = capture_preset_snapshot(
            &RealHost,
            PresetSnapshotRequest {
                source: PresetSnapshotSource::External(root.to_path_buf()),
                overlay_root: None,
            },
        )
        .await
        .unwrap();
        let mut context = RuntimeContext::isolated(
            root.join("home"),
            root.join("state"),
            root.to_path_buf(),
            root.join("bin"),
            RuntimePlatform::Linux,
        );
        context.is_external_presets = true;
        context.shell = ShellType::Zsh;
        CoreRuntime::new(RealHost, context, snapshot)
    }

    use tokio::fs;

    async fn make_temp_dir() -> PathBuf {
        crate::test_support::make_temp_dir("shine-presets").await
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn extraction_rejects_linked_files_and_parents_without_touching_referents() {
        use std::os::unix::fs::symlink;
        for (parent_link, dangling, force) in [
            (false, false, true),
            (false, true, false),
            (true, false, false),
            (true, false, true),
        ] {
            let root = make_temp_dir().await;
            let target = root.join("output\nFORGED\r\t\u{1b}");
            let outside = root.join("outside");
            fs::create_dir_all(target.join("app/demo")).await.unwrap();
            fs::create_dir_all(&outside).await.unwrap();
            let victim = outside.join("config");
            if !dangling {
                fs::write(&victim, b"user contents").await.unwrap();
            }
            if parent_link {
                fs::remove_dir(target.join("app/demo")).await.unwrap();
                symlink(&outside, target.join("app/demo")).unwrap();
            } else {
                symlink(&victim, target.join("app/demo/config")).unwrap();
            }
            let result = extract_matching(
                ["app/demo/config".to_string()],
                |_| true,
                |_| Some(b"preset contents".to_vec()),
                &target,
                force,
            )
            .await;
            let message = result
                .err()
                .expect("linked extraction was accepted")
                .to_string();
            assert!(!message.contains(['\r', '\t', '\u{1b}']), "{message:?}");
            assert!(message.contains(r"output\nFORGED\r\t\u{1b}"), "{message}");
            if dangling {
                assert!(!victim.exists());
            } else {
                assert_eq!(fs::read(&victim).await.unwrap(), b"user contents");
            }
            fs::remove_dir_all(root).await.unwrap();
        }
    }

    #[tokio::test]
    async fn forced_extraction_replaces_hard_links_without_modifying_other_names() {
        let root = make_temp_dir().await;
        let target = root.join("output");
        fs::create_dir_all(&target).await.unwrap();
        let outside = root.join("outside");
        fs::write(&outside, b"user contents").await.unwrap();
        fs::hard_link(&outside, target.join("config"))
            .await
            .unwrap();
        extract_matching(
            ["config".to_string()],
            |_| true,
            |_| Some(b"preset contents".to_vec()),
            &target,
            true,
        )
        .await
        .unwrap();
        assert_eq!(fs::read(&outside).await.unwrap(), b"user contents");
        assert_eq!(
            fs::read(target.join("config")).await.unwrap(),
            b"preset contents"
        );
        fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_nonforce_extraction_publishes_one_complete_file() {
        let root = make_temp_dir().await;
        let first = vec![b'a'; 64 * 1024];
        let second = vec![b'b'; 64 * 1024];
        let extract = |contents: Vec<u8>| {
            extract_matching(
                ["config".to_string()],
                |_| true,
                move |_| Some(contents.clone()),
                &root,
                false,
            )
        };
        let (a, b) = tokio::join!(extract(first.clone()), extract(second.clone()));
        let a = a.unwrap();
        let b = b.unwrap();
        assert_eq!(a.created.len() + b.created.len(), 1);
        assert_eq!(a.skipped.len() + b.skipped.len(), 1);
        let contents = fs::read(root.join("config")).await.unwrap();
        assert!(contents == first || contents == second);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn extraction_preserves_regular_file_modes_and_publishes_executable_scripts() {
        use std::os::unix::fs::PermissionsExt;
        let root = make_temp_dir().await;
        for (name, original, expected) in [("config", 0o640, 0o640), ("tool.sh", 0o600, 0o711)] {
            let dest = root.join(name);
            fs::write(&dest, b"old").await.unwrap();
            fs::set_permissions(&dest, std::fs::Permissions::from_mode(original))
                .await
                .unwrap();
            extract_matching(
                [name.to_string()],
                |_| true,
                |_| Some(b"new".to_vec()),
                &root,
                true,
            )
            .await
            .unwrap();
            assert_eq!(
                fs::metadata(&dest).await.unwrap().permissions().mode() & 0o777,
                expected
            );
            assert_eq!(fs::read(dest).await.unwrap(), b"new");
        }
        fs::remove_dir_all(root).await.unwrap();
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn extraction_rejects_windows_junction_parents() {
        let root = make_temp_dir().await;
        let target = root.join("output");
        let outside = root.join("outside");
        fs::create_dir_all(&target).await.unwrap();
        fs::create_dir_all(&outside).await.unwrap();
        let link = target.join("linked");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .status()
            .unwrap();
        assert!(status.success());
        let result = extract_matching(
            ["linked/config".to_string()],
            |_| true,
            |_| Some(b"preset".to_vec()),
            &target,
            true,
        )
        .await;
        assert!(result.is_err());
        assert!(!outside.join("config").exists());
        fs::remove_dir(link).await.unwrap();
        fs::remove_dir_all(root).await.unwrap();
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

        let reader = PresetAssetReader::new(Some(&dir));
        let paths = reader.asset_paths("shell");

        assert!(paths.contains(&"shell/personal/hello.sh".to_string()));
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn overlay_asset_paths_skip_node_modules() {
        let dir = make_temp_dir().await;
        fs::create_dir_all(dir.join("shell/personal/node_modules/zod"))
            .await
            .unwrap();
        fs::write(dir.join("shell/personal/package.json"), b"{}")
            .await
            .unwrap();
        fs::write(
            dir.join("shell/personal/bun.lock"),
            b"lockfileVersion = 1\n",
        )
        .await
        .unwrap();
        fs::write(
            dir.join("shell/personal/node_modules/zod/index.js"),
            b"export {}",
        )
        .await
        .unwrap();

        let reader = PresetAssetReader::new(Some(&dir));
        let paths = reader.asset_paths("shell/personal");

        assert!(paths.contains(&"shell/personal/package.json".to_string()));
        assert!(paths.contains(&"shell/personal/bun.lock".to_string()));
        assert!(paths.iter().all(|path| !path.contains("node_modules")));
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn overlay_read_asset_bytes_overrides_embedded_file() {
        let dir = make_temp_dir().await;
        fs::create_dir_all(dir.join("shell/proxy")).await.unwrap();
        fs::write(dir.join("shell/proxy/set_proxy.sh"), b"overlay\n")
            .await
            .unwrap();

        let reader = PresetAssetReader::new(Some(&dir));
        let bytes = reader.read_asset_bytes("shell/proxy/set_proxy.sh").unwrap();

        assert_eq!(bytes, b"overlay\n");
        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn readers_keep_overlay_enumeration_reads_and_extraction_isolated() {
        let root = make_temp_dir().await;
        let a = root.join("a");
        let b = root.join("b");
        let key = "shell/proxy/set_proxy.sh";
        for (dir, marker) in [(&a, b"a"), (&b, b"b")] {
            fs::create_dir_all(dir.join("shell/proxy")).await.unwrap();
            fs::write(dir.join(key), marker).await.unwrap();
        }
        fs::write(a.join("shell/proxy/a-only.txt"), b"only a")
            .await
            .unwrap();
        let readers = [
            PresetAssetReader::new(Some(&a)),
            PresetAssetReader::new(Some(&b)),
            PresetAssetReader::new(None),
        ];
        let embedded = read_embedded_asset_bytes(key).unwrap();
        for _ in 0..2 {
            for (reader, expected) in readers.iter().zip([b"a".as_slice(), b"b", &embedded]) {
                assert_eq!(reader.read_asset_bytes(key).unwrap(), expected);
                assert_eq!(
                    reader.read_asset_bytes("env.toml"),
                    read_embedded_asset_bytes("env.toml")
                );
                for unsafe_path in ["../env.toml", "/env.toml", "shell/../../env.toml"] {
                    assert!(reader.read_asset_bytes(unsafe_path).is_none());
                }
            }
        }
        let paths = readers[0].asset_paths("shell/proxy/");
        assert!(paths.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(paths.contains(&"shell/proxy/a-only.txt".to_string()));
        assert!(
            !readers[1]
                .asset_paths("shell/proxy")
                .contains(&"shell/proxy/a-only.txt".to_string())
        );
        let config_a = crate::config::Config::new_for_test(&root.join("config-a"))
            .with_presets_overlay_dir_override(Some(a.clone()));
        let config_b = crate::config::Config::new_for_test(&root.join("config-b"))
            .with_presets_overlay_dir_override(Some(b.clone()));
        let out_a = root.join("out-a");
        let out_b = root.join("out-b");
        let (a_result, b_result) = tokio::join!(
            crate::preset_commands::handle_preset_export(&config_a, Some(out_a.clone()), false),
            crate::preset_commands::handle_preset_export(&config_b, Some(out_b.clone()), false),
        );
        a_result.unwrap();
        b_result.unwrap();
        assert_eq!(fs::read(out_a.join(key)).await.unwrap(), b"a");
        assert_eq!(fs::read(out_b.join(key)).await.unwrap(), b"b");
        assert!(!out_b.join("shell/proxy/a-only.txt").exists());
        fs::remove_dir_all(root).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn overlay_enumeration_preserves_root_alias_and_excludes_tree_links() {
        use std::os::unix::fs::symlink;
        let root = make_temp_dir().await;
        let overlay = root.join("overlay");
        fs::create_dir_all(overlay.join("shell/personal"))
            .await
            .unwrap();
        fs::write(overlay.join("shell/personal/real.sh"), b"real")
            .await
            .unwrap();
        let alias = root.join("alias");
        symlink(&overlay, &alias).unwrap();
        symlink(
            overlay.join("shell/personal/real.sh"),
            overlay.join("shell/personal/link.sh"),
        )
        .unwrap();
        symlink(overlay.join("shell/personal"), overlay.join("shell/linked")).unwrap();
        let reader = PresetAssetReader::new(Some(&alias));
        let paths = reader.asset_paths("shell");
        assert!(paths.contains(&"shell/personal/real.sh".to_string()));
        assert!(
            !paths
                .iter()
                .any(|path| path.contains("/link.sh") || path.starts_with("shell/linked/"))
        );
        assert_eq!(
            reader.read_asset_bytes("shell/personal/real.sh").unwrap(),
            b"real"
        );
        fs::remove_dir_all(root).await.unwrap();
    }

    #[test]
    fn embedded_shell_metadata_missing_category_returns_empty() {
        let cats = crate::core_runtime::from_embedded_presets_for_platform(
            shine_core::runtime::RuntimePlatform::Linux,
        )
        .shell_categories(Some("nonexistent"))
        .unwrap();
        assert!(cats.is_empty());
    }

    #[tokio::test]
    async fn extracts_all_files_into_empty_dir() {
        let dir = make_temp_dir().await;
        let report = PresetAssetReader::new(None)
            .extract_all(&dir, false)
            .await
            .unwrap();

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

    // Production snapshot discovery and metadata regressions.

    #[tokio::test]
    async fn external_shell_metadata_returns_empty_for_missing_dir() {
        let missing =
            std::env::temp_dir().join(format!("shine-presets-no-{}", uuid::Uuid::new_v4()));
        let runtime = external_shell_runtime(&missing).await;
        let cats = runtime.shell_categories(None).unwrap();
        assert!(
            runtime
                .shell_categories(Some("missing"))
                .unwrap_err()
                .to_string()
                .contains("not found")
        );
        assert!(cats.is_empty());
    }

    #[tokio::test]
    async fn external_shell_metadata_finds_categories_from_disk() {
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

        let cats = external_shell_runtime(&dir)
            .await
            .shell_categories(None)
            .unwrap();

        assert_eq!(cats.len(), 1, "should find exactly one category");
        assert_eq!(cats[0].name, "myplugin");
        assert_eq!(cats[0].files.len(), 1);
        assert_eq!(cats[0].files[0].source_rel, Path::new("hello.sh"));
        assert_eq!(cats[0].files[0].description, vec!["Says hello."]);

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn external_shell_metadata_ignores_non_script_files() {
        let dir = make_temp_dir().await;
        let cat_dir = dir.join("shell/extras");
        fs::create_dir_all(&cat_dir).await.unwrap();
        fs::write(cat_dir.join("readme.md"), b"# readme\n")
            .await
            .unwrap();
        fs::write(cat_dir.join("script.sh"), b"#!/bin/bash\n# A script.\n")
            .await
            .unwrap();

        let cats = external_shell_runtime(&dir)
            .await
            .shell_categories(None)
            .unwrap();

        assert_eq!(cats.len(), 1);
        assert_eq!(
            cats[0].files.len(),
            1,
            "non-script assets must not become commands"
        );
        assert_eq!(cats[0].files[0].source_rel, Path::new("script.sh"));

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn external_shell_metadata_returns_alphabetical_order() {
        let dir = make_temp_dir().await;
        for cat in ["zzz", "aaa", "mmm"] {
            let cat_dir = dir.join("shell").join(cat);
            fs::create_dir_all(&cat_dir).await.unwrap();
            fs::write(cat_dir.join("s.sh"), b"#!/bin/bash\n")
                .await
                .unwrap();
        }

        let cats = external_shell_runtime(&dir)
            .await
            .shell_categories(None)
            .unwrap();
        let names: Vec<&str> = cats.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["aaa", "mmm", "zzz"]);

        fs::remove_dir_all(&dir).await.unwrap();
    }

    // Legacy recursive source discovery through the same metadata entry point.

    #[tokio::test]
    async fn external_shell_sources_finds_sh_files_recursively() {
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

        fs::create_dir_all(cat_dir.join("nested")).await.unwrap();
        fs::write(cat_dir.join("nested/c.sh"), b"#!/bin/sh\n")
            .await
            .unwrap();
        let categories = external_shell_runtime(&dir)
            .await
            .shell_categories(Some("myplugin"))
            .unwrap();
        let scripts = &categories[0].files;

        let names: Vec<_> = scripts
            .iter()
            .map(|file| file.source_rel.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, ["a.sh", "b.sh", "c.sh"]);
        assert!(
            !names.contains(&"readme.txt"),
            "non-script should be excluded"
        );
        assert!(
            scripts
                .iter()
                .any(|file| file.source_rel == Path::new("nested/c.sh"))
        );

        fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn external_shell_metadata_preserves_declared_sources_and_platform_selection() {
        use shine_core::runtime::{CoreRuntime, LinkRuntime, RealHost, RuntimePlatform, ShellType};
        let dir = make_temp_dir().await;
        let category = dir.join("shell/declared");
        fs::create_dir_all(&category).await.unwrap();
        fs::write(category.join("shine.toml"), "[[files]]\nsource = 'native.sh'\ntarget = 'native'\nplatforms = ['unix']\n[[files]]\nsource = 'native.ps1'\ntarget = 'native'\nplatforms = ['windows']\n[[files]]\nsource = 'run.ts'\nruntime = 'bun'\n").await.unwrap();
        for (name, bytes) in [
            ("native.sh", "# Unix description\necho native\n"),
            ("native.ps1", "# Windows description\nWrite-Output native\n"),
            ("run.ts", "// Bun description\nconsole.log('run');\n"),
            ("unlisted.sh", "echo unlisted\n"),
        ] {
            fs::write(category.join(name), bytes).await.unwrap();
        }
        let captured = external_shell_runtime(&dir).await;
        for platform in RuntimePlatform::ALL {
            let mut context = captured.context().clone();
            context.platform = platform;
            context.shell = if platform == RuntimePlatform::Windows {
                ShellType::PowerShell
            } else {
                ShellType::Zsh
            };
            let runtime = CoreRuntime::new(RealHost, context, captured.presets().clone());
            let categories = runtime.shell_categories(Some("declared")).unwrap();
            assert!(categories[0].uses_metadata);
            let files = &categories[0].files;
            assert_eq!(files.len(), 2);
            assert_eq!(files[0].command_name, "native");
            let windows = platform == RuntimePlatform::Windows;
            assert_eq!(
                files[0].source_rel,
                Path::new(if windows { "native.ps1" } else { "native.sh" })
            );
            assert_eq!(
                files[0].description,
                [if windows {
                    "Windows description"
                } else {
                    "Unix description"
                }]
            );
            assert_eq!(files[1].command_name, "run");
            assert_eq!(files[1].runtime, LinkRuntime::Bun);
            assert_eq!(files[1].description, ["Bun description"]);
        }
        fs::remove_dir_all(dir).await.unwrap();
    }

    // --- extract_all (presets export) tests ---

    #[tokio::test]
    async fn extract_all_creates_files_in_target_dir() {
        let dir = make_temp_dir().await;
        let report = PresetAssetReader::new(None)
            .extract_all(&dir, false)
            .await
            .unwrap();

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
        let dir = make_temp_dir().await;

        // First export — populates the dir
        let first = PresetAssetReader::new(None)
            .extract_all(&dir, false)
            .await
            .unwrap();
        assert!(!first.created.is_empty());

        // Overwrite one file with marker content
        let marker = b"do-not-overwrite";
        let target_path = &first.created[0];
        fs::write(target_path, marker).await.unwrap();

        // Second export without --force — should skip the modified file
        let second = PresetAssetReader::new(None)
            .extract_all(&dir, false)
            .await
            .unwrap();
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
        let dir = make_temp_dir().await;

        // First export
        let first = PresetAssetReader::new(None)
            .extract_all(&dir, false)
            .await
            .unwrap();
        assert!(!first.created.is_empty());

        // Overwrite one file with marker content
        let marker = b"old-content";
        let target_path = &first.created[0];
        fs::write(target_path, marker).await.unwrap();

        // Re-export with force
        let second = PresetAssetReader::new(None)
            .extract_all(&dir, true)
            .await
            .unwrap();
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
        let overlay = make_temp_dir().await;
        let target = make_temp_dir().await;
        let overlay_file = overlay.join("app/clash-verge/merge.yaml");
        fs::create_dir_all(overlay_file.parent().unwrap())
            .await
            .unwrap();
        fs::write(&overlay_file, "overlay-only marker")
            .await
            .unwrap();
        let reader = PresetAssetReader::new(Some(&overlay));
        assert_eq!(
            reader
                .read_asset_bytes("app/clash-verge/merge.yaml")
                .unwrap(),
            b"overlay-only marker"
        );

        let report = extract_embedded_prefix("app/clash-verge", &target, false)
            .await
            .unwrap();

        assert!(!report.created.is_empty());
        let copied = fs::read_to_string(target.join("app/clash-verge/merge.yaml"))
            .await
            .unwrap();
        assert_ne!(copied, "overlay-only marker");
        fs::remove_dir_all(overlay).await.unwrap();
        fs::remove_dir_all(target).await.unwrap();
    }
}
