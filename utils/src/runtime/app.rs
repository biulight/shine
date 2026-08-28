use crate::env::EnvVarSpec;
use crate::install::file_ops::{
    InstallOutcome, UninstallOutcome, install_bytes_with_host, uninstall_entry_with_host,
};
use crate::install::{AppEntry, AppInstallStrategy, AppManifest, hash_content};
use crate::lifecycle::{
    LifecycleEffect, LifecycleOperation, LifecycleOutcomeV1, LifecycleResultV1, LifecycleStatus,
};
use crate::runtime::{CoreRuntime, FileSystemHost};
use anyhow::{Context, Result, bail};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct AppCategory {
    pub name: String,
    pub description: Option<String>,
    pub destination_root: Option<String>,
    pub files: Vec<AppFile>,
    pub list_mode: AppListMode,
    pub post_upgrade: Vec<AppHook>,
    pub post_install: Vec<AppHook>,
    pub uses_metadata: bool,
    pub has_explicit_files: bool,
    pub artifact: Option<AppArtifact>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppListMode {
    Category,
    Files,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppHook {
    pub command: String,
    pub args: Vec<String>,
    pub show_output: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArtifactRuntime {
    #[default]
    Native,
    Bun,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppArtifact {
    pub script: String,
    pub teardown: Option<String>,
    pub runtime: ArtifactRuntime,
}

#[derive(Debug, Clone)]
pub struct AppFile {
    pub source_rel: PathBuf,
    pub target_rel: PathBuf,
    pub destination_root: Option<AppDestinationRoot>,
    pub description: Option<String>,
    pub display_name: Option<String>,
    pub legacy_dest_annotation: Option<String>,
    pub transforms: Vec<String>,
    pub install_strategy: AppInstallStrategy,
    pub requires_admin: bool,
    pub restart_hint: Option<String>,
    pub generator: Option<AppGenerator>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppDestinationRoot {
    Path(String),
    DataDir(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppGenerator {
    pub script: PathBuf,
    pub runtime: ArtifactRuntime,
    pub env: Vec<EnvVarSpec>,
    pub when_env: String,
    pub auto: bool,
}

#[derive(Clone, Debug)]
pub struct PreparedAppFile {
    pub category: String,
    pub resource: String,
    pub source: String,
    pub destination: PathBuf,
    pub content: Vec<u8>,
    pub install_strategy: AppInstallStrategy,
    pub uses_env: bool,
    pub requires_admin: bool,
}

#[derive(Clone, Debug, Default)]
pub struct AppInstallRequest {
    pub files: Vec<PreparedAppFile>,
    pub dry_run: bool,
    pub force: bool,
}

#[derive(Clone, Debug)]
pub struct AppUninstallRequest {
    pub category: String,
    pub dry_run: bool,
    pub force: bool,
}

impl<H: FileSystemHost> CoreRuntime<H> {
    pub async fn install_prepared_app(
        &self,
        request: AppInstallRequest,
    ) -> Result<LifecycleResultV1> {
        let mut manifest = load_manifest(&self.host, &self.context.shine_dir).await?;
        let mut result = LifecycleResultV1::new(LifecycleOperation::Install, request.dry_run);
        for file in request.files {
            if file.requires_admin {
                bail!("prepared App install requires a privileged host capability");
            }
            if !matches!(file.install_strategy, AppInstallStrategy::Copy) {
                bail!("prepared App JSON merge requires the App JSON resource executor");
            }
            let previous = manifest.find_by_dest(&file.destination).cloned();
            let outcome = install_bytes_with_host(
                &self.host,
                &file.content,
                &file.destination,
                previous.is_some(),
                request.dry_run,
                request.force,
            )
            .await?;
            let (status, mut effects, backup) = match outcome {
                InstallOutcome::Installed { .. } => (
                    LifecycleStatus::Changed,
                    vec![LifecycleEffect::ResourceWritten],
                    previous.as_ref().and_then(|entry| entry.backup.clone()),
                ),
                InstallOutcome::BackedUpAndInstalled { backup, .. } => (
                    LifecycleStatus::Changed,
                    vec![
                        LifecycleEffect::BackupCreated,
                        LifecycleEffect::ResourceWritten,
                    ],
                    Some(backup),
                ),
                InstallOutcome::AlreadyManaged => (
                    LifecycleStatus::Unchanged,
                    Vec::new(),
                    previous.as_ref().and_then(|entry| entry.backup.clone()),
                ),
                InstallOutcome::DryRun => (
                    LifecycleStatus::Previewed,
                    vec![
                        LifecycleEffect::ResourceWritePreviewed,
                        LifecycleEffect::ReceiptWritePreviewed,
                    ],
                    previous.as_ref().and_then(|entry| entry.backup.clone()),
                ),
            };
            if !request.dry_run {
                let entry = AppEntry {
                    source: file.source,
                    destination: file.destination,
                    backup,
                    content_hash: hash_content(&file.content),
                    install_strategy: file.install_strategy,
                    uses_env: file.uses_env,
                    requires_admin: file.requires_admin,
                };
                let receipt_changed = previous.as_ref() != Some(&entry);
                manifest.upsert(entry);
                if receipt_changed {
                    effects.push(LifecycleEffect::ReceiptWritten);
                }
            }
            result.push(LifecycleOutcomeV1::new(
                format!("app/{}", file.category),
                Some(file.resource),
                status,
                effects,
            ));
        }
        if !request.dry_run {
            save_manifest(&self.host, &self.context.shine_dir, &manifest).await?;
        }
        Ok(result)
    }

    pub async fn uninstall_app(&self, request: AppUninstallRequest) -> Result<LifecycleResultV1> {
        let mut manifest = load_manifest(&self.host, &self.context.shine_dir).await?;
        let prefix = format!("app/{}/", request.category);
        let selected = manifest
            .entries
            .iter()
            .filter(|entry| entry.source.starts_with(&prefix))
            .cloned()
            .collect::<Vec<_>>();
        let mut result = LifecycleResultV1::new(LifecycleOperation::Uninstall, request.dry_run);
        for entry in selected {
            if entry.requires_admin {
                bail!("prepared App uninstall requires a privileged host capability");
            }
            let resource = entry.source.strip_prefix(&prefix).unwrap_or(&entry.source);
            let outcome =
                uninstall_entry_with_host(&self.host, &entry, request.dry_run, request.force)
                    .await?;
            let (status, effects, remove_receipt) = match outcome {
                UninstallOutcome::Removed | UninstallOutcome::ForceRemoved => (
                    LifecycleStatus::Changed,
                    vec![
                        LifecycleEffect::ResourceRemoved,
                        LifecycleEffect::ReceiptRemoved,
                    ],
                    true,
                ),
                UninstallOutcome::RestoredBackup { .. }
                | UninstallOutcome::ForceRestoredBackup { .. } => (
                    LifecycleStatus::Changed,
                    vec![
                        LifecycleEffect::BackupRestored,
                        LifecycleEffect::ReceiptRemoved,
                    ],
                    true,
                ),
                UninstallOutcome::NotFound => (
                    LifecycleStatus::Changed,
                    vec![LifecycleEffect::ReceiptRemoved],
                    true,
                ),
                UninstallOutcome::UserModified => (
                    LifecycleStatus::Preserved,
                    vec![LifecycleEffect::UserResourcePreserved],
                    false,
                ),
                UninstallOutcome::DryRun => (
                    LifecycleStatus::Previewed,
                    vec![
                        LifecycleEffect::ResourceRemovePreviewed,
                        LifecycleEffect::ReceiptRemovePreviewed,
                    ],
                    false,
                ),
            };
            if remove_receipt {
                manifest.remove_by_dest(&entry.destination);
            }
            result.push(LifecycleOutcomeV1::new(
                format!("app/{}", request.category),
                Some(resource.to_string()),
                status,
                effects,
            ));
        }
        if !request.dry_run {
            save_manifest(&self.host, &self.context.shine_dir, &manifest).await?;
        }
        Ok(result)
    }
}

async fn load_manifest(
    host: &impl FileSystemHost,
    shine_dir: &std::path::Path,
) -> Result<AppManifest> {
    let path = shine_dir.join("app-manifest.toml");
    let mut manifest = match host.read(&path).await {
        Ok(bytes) => toml::from_slice(&bytes).context("failed to parse app manifest")?,
        Err(error) if error.is_not_found() => AppManifest::default(),
        Err(error) => return Err(error.into_anyhow("failed to read app manifest")),
    };
    match manifest.schema_version {
        0 => manifest.schema_version = crate::install::manifest::APP_MANIFEST_SCHEMA_VERSION,
        crate::install::manifest::APP_MANIFEST_SCHEMA_VERSION => {}
        version => bail!(
            "app manifest schema version {version} is newer than this Shine supports ({})",
            crate::install::manifest::APP_MANIFEST_SCHEMA_VERSION
        ),
    }
    Ok(manifest)
}

async fn save_manifest(
    host: &impl FileSystemHost,
    shine_dir: &std::path::Path,
    manifest: &AppManifest,
) -> Result<()> {
    let content = toml::to_string_pretty(manifest).context("failed to serialize app manifest")?;
    host.write_atomic(&shine_dir.join("app-manifest.toml"), content.as_bytes())
        .await
        .map_err(|error| error.into_anyhow("failed to write app manifest"))
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use crate::runtime::{
        InMemoryHost, PresetSnapshot, PresetSourceKind, RuntimeContext, RuntimePlatform,
    };
    use std::collections::BTreeMap;
    use std::path::Path;

    fn runtime() -> CoreRuntime<InMemoryHost> {
        CoreRuntime::new(
            InMemoryHost::new(),
            RuntimeContext {
                home_dir: PathBuf::from("/home/test"),
                shine_dir: PathBuf::from("/home/test/.shine"),
                presets_dir: PathBuf::from("/home/test/.shine/presets"),
                bin_dir: PathBuf::from("/home/test/.shine/bin"),
                platform: RuntimePlatform::Linux,
                env: BTreeMap::new(),
            },
            PresetSnapshot::builder(PresetSourceKind::External).build(),
        )
    }

    fn install_request(content: &[u8]) -> AppInstallRequest {
        AppInstallRequest {
            files: vec![PreparedAppFile {
                category: "demo".into(),
                resource: "config".into(),
                source: "app/demo/config".into(),
                destination: PathBuf::from("/home/test/config"),
                content: content.to_vec(),
                install_strategy: AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
            ..AppInstallRequest::default()
        }
    }

    #[tokio::test]
    async fn app_executor_roundtrip_and_target_isolation_use_in_memory_host() {
        let runtime = runtime();
        let installed = runtime
            .install_prepared_app(install_request(b"one"))
            .await
            .unwrap();
        assert_eq!(installed.summary().changed, 1);
        let unchanged = runtime
            .install_prepared_app(install_request(b"one"))
            .await
            .unwrap();
        assert_eq!(unchanged.summary().unchanged, 1);

        runtime
            .host()
            .put_file("/home/test/other", b"other".to_vec());
        let removed = runtime
            .uninstall_app(AppUninstallRequest {
                category: "demo".into(),
                dry_run: false,
                force: false,
            })
            .await
            .unwrap();
        assert_eq!(removed.summary().changed, 1);
        assert!(
            runtime
                .host()
                .read(Path::new("/home/test/config"))
                .await
                .is_err()
        );
        assert_eq!(
            runtime
                .host()
                .read(Path::new("/home/test/other"))
                .await
                .unwrap(),
            b"other"
        );
    }
}
