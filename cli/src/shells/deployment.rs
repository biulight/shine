use super::metadata::{ShellCategory, ShellFile};
use crate::config::{Config, ExternalShellMode};
use crate::env::EnvConfig;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const MANIFEST_FILE: &str = "shell-manifest.toml";
pub(crate) const SHELL_MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ShellManifestEntry {
    pub category: String,
    pub command: String,
    pub mode: ExternalShellMode,
    pub source_path: PathBuf,
    pub rendered_path: PathBuf,
    pub runtime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bun_dependencies: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_hash: Option<u64>,
    #[serde(default)]
    pub transforms: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub needs_source: bool,
    pub content_hash: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ShellManifest {
    #[serde(default = "legacy_manifest_schema_version")]
    pub(crate) schema_version: u32,
    #[serde(default)]
    pub entries: Vec<ShellManifestEntry>,
}

fn legacy_manifest_schema_version() -> u32 {
    0
}

impl Default for ShellManifest {
    fn default() -> Self {
        Self {
            schema_version: SHELL_MANIFEST_SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
}

impl ShellManifest {
    pub(crate) async fn load(config: &Config) -> Result<Self> {
        let mut manifest: Self = crate::persist::load_toml_or_default(
            &config.shine_dir().join(MANIFEST_FILE),
            "shell manifest",
        )
        .await?;
        match manifest.schema_version {
            0 => manifest.schema_version = SHELL_MANIFEST_SCHEMA_VERSION,
            SHELL_MANIFEST_SCHEMA_VERSION => {}
            version => bail!(
                "shell manifest schema version {version} is newer than this Shine supports ({SHELL_MANIFEST_SCHEMA_VERSION})"
            ),
        }
        Ok(manifest)
    }

    pub(crate) async fn save(&self, config: &Config) -> Result<()> {
        if self.schema_version != SHELL_MANIFEST_SCHEMA_VERSION {
            bail!(
                "cannot write shell manifest schema version {}; expected {SHELL_MANIFEST_SCHEMA_VERSION}",
                self.schema_version
            );
        }
        crate::persist::save_toml_atomic(
            self,
            &config.shine_dir().join(MANIFEST_FILE),
            "shell manifest",
        )
        .await
    }

    pub(crate) fn find(&self, target: &str) -> Option<&ShellManifestEntry> {
        self.entries
            .iter()
            .find(|entry| canonical_target(entry) == target)
    }

    pub(crate) fn replace_categories(
        &mut self,
        categories: &BTreeSet<String>,
        entries: Vec<ShellManifestEntry>,
    ) {
        self.entries
            .retain(|entry| !categories.contains(&entry.category));
        self.entries.extend(entries);
        self.entries.sort_by_key(canonical_target);
    }

    pub(crate) fn remove_category(&mut self, category: &str) {
        self.entries.retain(|entry| entry.category != category);
    }

    pub(crate) fn remove_target(&mut self, category: &str, command: &str) {
        self.entries
            .retain(|entry| entry.category != category || entry.command != command);
    }

    fn replace_targets(&mut self, targets: &BTreeSet<String>, entries: Vec<ShellManifestEntry>) {
        self.entries
            .retain(|entry| !targets.contains(&canonical_target(entry)));
        self.entries.extend(entries);
        self.entries.sort_by_key(canonical_target);
    }
}

fn canonical_target(entry: &ShellManifestEntry) -> String {
    format!("shell/{}/{}", entry.category, entry.command)
}

pub(crate) fn deployment_source_path(
    config: &Config,
    category: &str,
    source_rel: &Path,
) -> PathBuf {
    if config.is_external_presets && config.external_shell_mode == ExternalShellMode::Snapshot {
        config.installed_shell_dir().join(category).join(source_rel)
    } else {
        config.preset_path(Path::new("shell").join(category).join(source_rel))
    }
}

pub(crate) fn desired_source_path(config: &Config, category: &str, source_rel: &Path) -> PathBuf {
    config.preset_path(Path::new("shell").join(category).join(source_rel))
}

pub(crate) fn rendered_path(config: &Config, category: &str, source_rel: &Path) -> PathBuf {
    config
        .rendered_dir()
        .join("shell")
        .join(category)
        .join(source_rel)
}

pub(crate) fn bun_runtime_spec(
    config: &Config,
    category: &str,
    file: &ShellFile,
) -> Result<crate::bun_runtime::BunRuntimeSpec> {
    if file.runtime != crate::bin_links::LinkRuntime::Bun {
        return Ok(crate::bun_runtime::BunRuntimeSpec::default());
    }
    let relative_root = Path::new("shell").join(category);
    let overlay_root = config
        .active_presets_overlay_dir()
        .map(|root| root.join(&relative_root));
    if let Some(root) = overlay_root
        && root.join(&file.source_rel).is_file()
    {
        return crate::bun_runtime::resolve(&root, true);
    }
    if config.is_external_presets {
        return crate::bun_runtime::resolve(&config.presets_dir().join(relative_root), true);
    }
    Ok(crate::bun_runtime::BunRuntimeSpec::default())
}

pub(crate) async fn effective_transforms(
    file: &ShellFile,
    source_path: &Path,
) -> Result<Vec<String>> {
    if !file.transforms.is_empty() {
        return Ok(file.transforms.clone());
    }
    let bytes = tokio::fs::read(source_path)
        .await
        .with_context(|| format!("reading shell source: {}", source_path.display()))?;
    Ok(if crate::presets::parse_template_annotation(&bytes) {
        vec!["template".to_string()]
    } else {
        Vec::new()
    })
}

pub(crate) async fn materialize_snapshot_categories(
    config: &Config,
    categories: &[ShellCategory],
) -> Result<usize> {
    if !config.is_external_presets || config.external_shell_mode != ExternalShellMode::Snapshot {
        return Ok(0);
    }
    let mut changed = 0;
    for category in categories {
        changed += usize::from(materialize_snapshot_category(config, &category.name).await?);
    }
    Ok(changed)
}

pub(crate) async fn validate_snapshot_categories(
    config: &Config,
    categories: &[ShellCategory],
) -> Result<()> {
    if !config.is_external_presets || config.external_shell_mode != ExternalShellMode::Snapshot {
        return Ok(());
    }
    let env = EnvConfig::load_or_init(config).await?;
    for category in categories {
        for file in &category.files {
            let source_path = desired_source_path(config, &category.name, &file.source_rel);
            let transforms = effective_transforms(file, &source_path).await?;
            if transforms.is_empty() {
                continue;
            }
            let source = tokio::fs::read(&source_path).await?;
            crate::install_core::apply_transforms(&transforms, &source, env.as_map())
                .with_context(|| {
                    format!(
                        "validating transformed shell source: {}",
                        source_path.display()
                    )
                })?;
        }
    }
    Ok(())
}

pub(crate) async fn snapshot_category_current(config: &Config, category: &str) -> Result<bool> {
    if !config.is_external_presets || config.external_shell_mode != ExternalShellMode::Snapshot {
        return Ok(true);
    }
    let relative_root = Path::new("shell").join(category);
    let base_root = config.presets_dir().join(&relative_root);
    let overlay_root = config
        .active_presets_overlay_dir()
        .map(|root| root.join(&relative_root));
    let installed_root = config.installed_shell_dir().join(category);
    if !installed_root.exists() {
        return Ok(false);
    }

    let mut desired_files = BTreeSet::new();
    collect_files(&base_root, &base_root, &mut desired_files).await?;
    if let Some(overlay_root) = &overlay_root {
        collect_files(overlay_root, overlay_root, &mut desired_files).await?;
    }
    let mut installed_files = BTreeSet::new();
    collect_files(&installed_root, &installed_root, &mut installed_files).await?;
    if desired_files != installed_files {
        return Ok(false);
    }
    for relative in desired_files {
        let desired = overlay_root
            .as_ref()
            .map(|root| root.join(&relative))
            .filter(|path| path.is_file())
            .unwrap_or_else(|| base_root.join(&relative));
        if tokio::fs::read(desired).await? != tokio::fs::read(installed_root.join(relative)).await?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn materialize_snapshot_category(config: &Config, category: &str) -> Result<bool> {
    let relative_root = Path::new("shell").join(category);
    let base_root = config.presets_dir().join(&relative_root);
    let overlay_root = config
        .active_presets_overlay_dir()
        .map(|root| root.join(&relative_root));

    let mut relative_files = BTreeSet::new();
    collect_files(&base_root, &base_root, &mut relative_files).await?;
    if let Some(overlay_root) = &overlay_root {
        collect_files(overlay_root, overlay_root, &mut relative_files).await?;
    }
    if relative_files.is_empty() {
        bail!("external shell preset category is empty: {category}");
    }

    let installed_root = config.installed_shell_dir();
    tokio::fs::create_dir_all(&installed_root)
        .await
        .with_context(|| format!("creating {}", installed_root.display()))?;
    let stage = installed_root.join(format!(".{category}-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&stage)
        .await
        .with_context(|| format!("creating snapshot stage: {}", stage.display()))?;

    let result = async {
        for relative in relative_files {
            let source = overlay_root
                .as_ref()
                .map(|root| root.join(&relative))
                .filter(|path| path.is_file())
                .unwrap_or_else(|| base_root.join(&relative));
            let destination = stage.join(&relative);
            if let Some(parent) = destination.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::copy(&source, &destination)
                .await
                .with_context(|| {
                    format!("snapshotting external shell file: {}", source.display())
                })?;
        }

        let destination = installed_root.join(category);
        if trees_equal(&stage, &destination).await? {
            tokio::fs::remove_dir_all(&stage).await?;
            return Ok(false);
        }
        let backup = installed_root.join(format!(".{category}-old-{}", uuid::Uuid::new_v4()));
        let had_destination = destination.exists();
        if had_destination {
            tokio::fs::rename(&destination, &backup)
                .await
                .with_context(|| {
                    format!("staging previous shell snapshot: {}", destination.display())
                })?;
        }
        if let Err(error) = tokio::fs::rename(&stage, &destination).await {
            if had_destination {
                let _ = tokio::fs::rename(&backup, &destination).await;
            }
            return Err(error)
                .with_context(|| format!("installing shell snapshot: {}", destination.display()));
        }
        if had_destination {
            let _ = tokio::fs::remove_dir_all(&backup).await;
        }
        Ok(true)
    }
    .await;

    if result.is_err() {
        let _ = tokio::fs::remove_dir_all(&stage).await;
    }
    result
}

async fn trees_equal(left: &Path, right: &Path) -> Result<bool> {
    if !right.exists() {
        return Ok(false);
    }
    let mut left_files = BTreeSet::new();
    let mut right_files = BTreeSet::new();
    collect_files(left, left, &mut left_files).await?;
    collect_files(right, right, &mut right_files).await?;
    if left_files != right_files {
        return Ok(false);
    }
    for relative in left_files {
        if tokio::fs::read(left.join(&relative)).await?
            != tokio::fs::read(right.join(&relative)).await?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn collect_files(root: &Path, current: &Path, files: &mut BTreeSet<PathBuf>) -> Result<()> {
    if !current.exists() {
        return Ok(());
    }
    let mut pending = vec![current.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let mut entries = tokio::fs::read_dir(&dir)
            .await
            .with_context(|| format!("reading shell preset directory: {}", dir.display()))?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let kind = entry.file_type().await?;
            if kind.is_dir() {
                if entry.file_name() == "node_modules" {
                    continue;
                }
                pending.push(path);
            } else if kind.is_file() {
                files.insert(
                    path.strip_prefix(root)
                        .context("shell preset path escaped category root")?
                        .to_path_buf(),
                );
            } else if kind.is_symlink() {
                let target = tokio::fs::metadata(&path).await.with_context(|| {
                    format!("resolving shell preset symlink: {}", path.display())
                })?;
                if target.is_file() {
                    files.insert(
                        path.strip_prefix(root)
                            .context("shell preset path escaped category root")?
                            .to_path_buf(),
                    );
                } else {
                    bail!(
                        "shell snapshot does not support directory symlinks: {}",
                        path.display()
                    );
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ManifestUpdateScope {
    Categories,
    Commands,
}

pub(crate) async fn update_manifest(
    config: &Config,
    categories: &[ShellCategory],
    scope: ManifestUpdateScope,
) -> Result<()> {
    let mut manifest = ShellManifest::load(config).await?;
    let selected: BTreeSet<String> = categories.iter().map(|cat| cat.name.clone()).collect();
    let selected_targets = categories
        .iter()
        .flat_map(|category| {
            category
                .files
                .iter()
                .map(|file| format!("shell/{}/{}", category.name, file.command_name))
        })
        .collect::<BTreeSet<_>>();
    let mut entries = Vec::new();
    for category in categories {
        for file in &category.files {
            let source_path = deployment_source_path(config, &category.name, &file.source_rel);
            let bytes = tokio::fs::read(&source_path).await.with_context(|| {
                format!("reading installed shell source: {}", source_path.display())
            })?;
            let transforms = effective_transforms(file, &source_path).await?;
            let rendered_path = rendered_path(config, &category.name, &file.source_rel);
            let effective_source = if transforms.is_empty() {
                source_path.as_path()
            } else {
                rendered_path.as_path()
            };
            let env = file
                .env
                .iter()
                .map(|spec| spec.to_with_arg())
                .collect::<Vec<_>>();
            let bun_runtime = bun_runtime_spec(config, &category.name, file)?;
            let render_target = (config.is_external_presets
                && config.external_shell_mode == ExternalShellMode::Live
                && !transforms.is_empty())
            .then(|| format!("shell/{}/{}", category.name, file.command_name));
            let link_path = crate::bin_links::command_path_for_name(
                config.bin_dir(),
                std::ffi::OsStr::new(&file.command_name),
            );
            if !crate::bin_links::link_is_current(
                &link_path,
                effective_source,
                file.runtime,
                bun_runtime.dependency_mode,
                &env,
                render_target.as_deref(),
            )
            .await?
            {
                continue;
            }
            entries.push(ShellManifestEntry {
                category: category.name.clone(),
                command: file.command_name.clone(),
                mode: if config.is_external_presets {
                    config.external_shell_mode
                } else {
                    ExternalShellMode::Snapshot
                },
                source_path,
                rendered_path,
                runtime: match file.runtime {
                    crate::bin_links::LinkRuntime::Native => "native",
                    crate::bin_links::LinkRuntime::Bun => "bun",
                }
                .to_string(),
                bun_dependencies: bun_runtime
                    .dependency_mode
                    .as_manifest_value()
                    .map(str::to_string),
                dependency_hash: bun_runtime.dependency_hash,
                transforms,
                env,
                needs_source: file.needs_source,
                content_hash: crate::install_core::hash_content(&bytes),
            });
        }
    }
    match scope {
        ManifestUpdateScope::Categories => manifest.replace_categories(&selected, entries),
        ManifestUpdateScope::Commands => manifest.replace_targets(&selected_targets, entries),
    }
    manifest.save(config).await
}

pub async fn handle_render_live(config: &Config, target: &str) -> Result<()> {
    let manifest = ShellManifest::load(config).await?;
    let entry = manifest
        .find(target)
        .with_context(|| format!("live shell command is not installed: {target}"))?;
    if entry.mode != ExternalShellMode::Live {
        bail!("shell command is not installed in live mode: {target}");
    }
    if entry.transforms.is_empty() {
        return Ok(());
    }
    if !entry.rendered_path.starts_with(config.rendered_dir()) {
        bail!("invalid live rendered path recorded for {target}");
    }

    let _lock = RenderLock::acquire(&entry.rendered_path).await?;
    let source = tokio::fs::read(&entry.source_path)
        .await
        .with_context(|| format!("reading live source: {}", entry.source_path.display()))?;
    let env = EnvConfig::load_or_init(config).await?;
    let rendered = crate::install_core::apply_transforms(&entry.transforms, &source, env.as_map())
        .with_context(|| format!("live transform failed for {target}"))?;
    if tokio::fs::read(&entry.rendered_path)
        .await
        .is_ok_and(|current| current == rendered)
    {
        return Ok(());
    }
    crate::persist::atomic_write(&entry.rendered_path, &rendered).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = tokio::fs::metadata(&entry.source_path)
            .await
            .map(|meta| meta.permissions().mode())
            .unwrap_or(0o755);
        tokio::fs::set_permissions(&entry.rendered_path, std::fs::Permissions::from_mode(mode))
            .await?;
    }
    Ok(())
}

struct RenderLock {
    path: PathBuf,
}

impl RenderLock {
    async fn acquire(rendered: &Path) -> Result<Self> {
        let lock_dir = rendered
            .parent()
            .context("live rendered path has no parent")?;
        tokio::fs::create_dir_all(lock_dir).await?;
        let name = rendered
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("shell");
        let path = lock_dir.join(format!(".{name}.shine-lock"));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match tokio::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&path)
                .await
            {
                Ok(_) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = tokio::fs::metadata(&path)
                        .await
                        .ok()
                        .and_then(|meta| meta.modified().ok())
                        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                        .is_some_and(|age| age > Duration::from_secs(30));
                    if stale {
                        let _ = tokio::fs::remove_file(&path).await;
                        continue;
                    }
                    if tokio::time::Instant::now() >= deadline {
                        bail!("timed out waiting for live shell render lock");
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => return Err(error).context("creating live shell render lock"),
            }
        }
    }
}

impl Drop for RenderLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_manifest_without_bun_dependency_fields_deserializes() {
        let manifest: ShellManifest = toml::from_str(
            r#"
[[entries]]
category = "tools"
command = "tool"
mode = "snapshot"
source_path = "/tmp/tool.ts"
rendered_path = "/tmp/rendered/tool.ts"
runtime = "bun"
transforms = []
env = []
needs_source = false
content_hash = 42
"#,
        )
        .unwrap();

        assert_eq!(manifest.entries.len(), 1);
        assert_eq!(manifest.entries[0].bun_dependencies, None);
        assert_eq!(manifest.entries[0].dependency_hash, None);
    }

    #[tokio::test]
    async fn shell_manifest_legacy_and_future_versions_are_gated() {
        let root = crate::test_support::make_temp_dir("shine-shell-manifest-version").await;
        let config = Config::new_for_test(&root);
        tokio::fs::write(config.shine_dir().join(MANIFEST_FILE), "entries = []\n")
            .await
            .unwrap();

        let legacy = ShellManifest::load(&config).await.unwrap();
        assert_eq!(legacy.schema_version, SHELL_MANIFEST_SCHEMA_VERSION);
        let after_read = tokio::fs::read_to_string(config.shine_dir().join(MANIFEST_FILE))
            .await
            .unwrap();
        assert!(!after_read.contains("schema_version"));
        legacy.save(&config).await.unwrap();
        let written = tokio::fs::read_to_string(config.shine_dir().join(MANIFEST_FILE))
            .await
            .unwrap();
        assert!(written.contains("schema_version = 1"));

        tokio::fs::write(
            config.shine_dir().join(MANIFEST_FILE),
            "schema_version = 2\nentries = []\n",
        )
        .await
        .unwrap();
        let error = ShellManifest::load(&config).await.unwrap_err();
        assert!(error.to_string().contains("newer than this Shine supports"));
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn overlay_package_without_overlay_script_does_not_enable_builtin_dependencies() {
        let root = crate::test_support::make_temp_dir("shine-shell-overlay-package-only").await;
        let overlay = root.join("overlay");
        let category = overlay.join("shell/tools");
        tokio::fs::create_dir_all(&category).await.unwrap();
        tokio::fs::write(category.join("package.json"), b"{\"dependencies\":{}}")
            .await
            .unwrap();
        tokio::fs::write(category.join("bun.lock"), b"lockfileVersion = 1\n")
            .await
            .unwrap();

        let mut config = Config::new_for_test(&root);
        config.presets_overlay_dir_override = Some(overlay);
        let file = ShellFile {
            source_rel: PathBuf::from("tool.ts"),
            command_name: "tool".to_string(),
            description: Vec::new(),
            needs_source: false,
            runtime: crate::bin_links::LinkRuntime::Bun,
            transforms: Vec::new(),
            env: Vec::new(),
        };

        let spec = bun_runtime_spec(&config, "tools", &file).unwrap();
        assert_eq!(spec, crate::bun_runtime::BunRuntimeSpec::default());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn collect_files_keeps_dependency_manifests_but_skips_node_modules() {
        let root = crate::test_support::make_temp_dir("shine-shell-dependency-files").await;
        tokio::fs::write(root.join("package.json"), b"{}")
            .await
            .unwrap();
        tokio::fs::write(root.join("bun.lock"), b"lockfileVersion = 1\n")
            .await
            .unwrap();
        tokio::fs::create_dir_all(root.join("node_modules/zod"))
            .await
            .unwrap();
        tokio::fs::write(root.join("node_modules/zod/index.js"), b"export {}")
            .await
            .unwrap();
        tokio::fs::create_dir_all(root.join("nested/node_modules/pkg"))
            .await
            .unwrap();
        tokio::fs::write(root.join("nested/node_modules/pkg/index.js"), b"export {}")
            .await
            .unwrap();

        let mut files = BTreeSet::new();
        collect_files(&root, &root, &mut files).await.unwrap();
        assert!(files.contains(Path::new("package.json")));
        assert!(files.contains(Path::new("bun.lock")));
        assert!(files.iter().all(|path| !path.starts_with("node_modules")));
        assert!(
            files
                .iter()
                .all(|path| !path.starts_with("nested/node_modules"))
        );

        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
