use crate::install::file_ops::{
    InstallOutcome, UninstallOutcome, install_bytes_with_host, uninstall_entry_with_host,
};
use crate::install::{AppEntry, AppInstallStrategy, hash_content};
use crate::lifecycle::LifecycleEffect;
use crate::lifecycle::{
    LifecycleOperation, LifecycleOutcomeV1, LifecycleResultV1, LifecycleStatus,
};
use crate::runtime::{CoreRuntime, FileSystemHost};
use anyhow::Context;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

pub const RECEIPT_VERSION: u32 = 1;
pub const SYS_MANIFEST_FILE: &str = "sys-manifest.toml";
pub const SYS_MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SysDriverKind {
    #[default]
    Script,
    SplitDns,
    ManagedFile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SysItemStatus {
    Installed,
    AlreadyInstalled,
    Skipped,
    Updated,
    NeedsAction,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "driver", rename_all = "kebab-case")]
pub enum SystemReceipt {
    Script { version: u32 },
    SplitDns(SplitDnsReceipt),
    ManagedFile(ManagedFileReceipt),
}

impl SystemReceipt {
    pub fn script() -> Self {
        Self::Script {
            version: RECEIPT_VERSION,
        }
    }

    pub fn driver(&self) -> SysDriverKind {
        match self {
            Self::Script { .. } => SysDriverKind::Script,
            Self::SplitDns(_) => SysDriverKind::SplitDns,
            Self::ManagedFile(_) => SysDriverKind::ManagedFile,
        }
    }

    pub fn requires_admin(&self) -> bool {
        match self {
            Self::Script { .. } => false,
            Self::SplitDns(_) => true,
            Self::ManagedFile(receipt) => receipt.privileged,
        }
    }

    pub fn ensure_supported(&self) -> Result<()> {
        let version = match self {
            Self::Script { version } => *version,
            Self::SplitDns(receipt) => receipt.version,
            Self::ManagedFile(receipt) => receipt.version,
        };
        if version != RECEIPT_VERSION {
            bail!("unsupported system resource receipt version {version}");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SplitDnsReceipt {
    pub version: u32,
    pub os_id: String,
    pub item_id: String,
    pub domain: String,
    pub servers: Vec<String>,
    pub resource: String,
    pub content_hash: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedFileReceipt {
    pub version: u32,
    pub destination: PathBuf,
    pub backup: Option<PathBuf>,
    pub content_hash: u64,
    pub privileged: bool,
    pub restart_hint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourcePlan {
    pub description: String,
    pub requires_admin: bool,
    pub restart_hint: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ResourceOutcome {
    pub changed: bool,
    pub effects: Vec<LifecycleEffect>,
    pub detail: String,
    pub receipt: Option<SystemReceipt>,
    pub restart_hint: Option<String>,
}

#[derive(Debug)]
pub struct ResourceConflict {
    message: String,
}

#[derive(Clone, Debug)]
pub struct ManagedFileRequest {
    pub os_id: String,
    pub item_id: String,
    pub label: String,
    pub destination: PathBuf,
    pub content: Vec<u8>,
    pub privileged: bool,
    pub restart_hint: Option<String>,
    pub dry_run: bool,
}

#[derive(Clone, Debug)]
pub struct ManagedFileRemoveRequest {
    pub os_id: String,
    pub item_id: String,
    pub dry_run: bool,
}

impl ResourceConflict {
    pub fn user_modified(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ResourceConflict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ResourceConflict {}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SysRunManifest {
    #[serde(default = "legacy_manifest_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub entries: Vec<SysRunEntry>,
}

fn legacy_manifest_schema_version() -> u32 {
    0
}

impl Default for SysRunManifest {
    fn default() -> Self {
        Self {
            schema_version: SYS_MANIFEST_SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SysRunEntry {
    pub os_id: String,
    pub item_id: String,
    pub label: String,
    pub status: SysItemStatus,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    pub updated_at: String,
    #[serde(default)]
    pub managed: bool,
    #[serde(default = "default_profile_enabled")]
    pub profile_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<SystemReceipt>,
}

fn default_profile_enabled() -> bool {
    true
}

impl SysRunManifest {
    pub async fn load(shine_dir: &Path) -> Result<Self> {
        let mut manifest: Self = crate::persist::load_toml_or_default(
            &shine_dir.join(SYS_MANIFEST_FILE),
            "sys manifest",
        )
        .await?;
        match manifest.schema_version {
            0 => manifest.schema_version = SYS_MANIFEST_SCHEMA_VERSION,
            SYS_MANIFEST_SCHEMA_VERSION => {}
            version => bail!(
                "sys manifest schema version {version} is newer than this Shine supports ({SYS_MANIFEST_SCHEMA_VERSION})"
            ),
        }
        Ok(manifest)
    }

    pub async fn save(&self, shine_dir: &Path) -> Result<()> {
        if self.schema_version != SYS_MANIFEST_SCHEMA_VERSION {
            bail!(
                "cannot write sys manifest schema version {}; expected {SYS_MANIFEST_SCHEMA_VERSION}",
                self.schema_version
            );
        }
        crate::persist::save_toml_atomic(self, &shine_dir.join(SYS_MANIFEST_FILE), "sys manifest")
            .await
    }

    pub fn upsert(&mut self, entry: SysRunEntry) {
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|existing| existing.os_id == entry.os_id && existing.item_id == entry.item_id)
        {
            *existing = entry;
        } else {
            self.entries.push(entry);
        }
    }
}

impl<H: FileSystemHost> CoreRuntime<H> {
    pub async fn apply_managed_sys_file(
        &self,
        request: ManagedFileRequest,
    ) -> Result<LifecycleResultV1> {
        if request.privileged {
            bail!("managed Sys file requires a privileged host capability");
        }
        let mut manifest = load_manifest_with_host(&self.host, &self.context.shine_dir).await?;
        let previous = manifest
            .entries
            .iter()
            .find(|entry| entry.os_id == request.os_id && entry.item_id == request.item_id);
        let previous_receipt = previous.and_then(|entry| match &entry.receipt {
            Some(SystemReceipt::ManagedFile(receipt)) => Some(receipt),
            _ => None,
        });
        if let Some(receipt) = previous_receipt
            && receipt.destination != request.destination
        {
            let entry = app_entry_from_receipt(receipt);
            if matches!(
                uninstall_entry_with_host(&self.host, &entry, request.dry_run, false).await?,
                UninstallOutcome::UserModified
            ) {
                let mut result =
                    LifecycleResultV1::new(LifecycleOperation::Upgrade, request.dry_run);
                result.push(
                    LifecycleOutcomeV1::new(
                        format!("sys/{}", request.item_id),
                        None::<String>,
                        LifecycleStatus::Conflict,
                        [LifecycleEffect::UserResourcePreserved],
                    )
                    .with_diagnostic_code("sys_user_modified"),
                );
                return Ok(result);
            }
        }
        let is_managed =
            previous_receipt.is_some_and(|receipt| receipt.destination == request.destination);
        let outcome = install_bytes_with_host(
            &self.host,
            &request.content,
            &request.destination,
            is_managed,
            request.dry_run,
            true,
        )
        .await?;
        let (status, mut effects, backup) = match outcome {
            InstallOutcome::Installed { .. } => (
                LifecycleStatus::Changed,
                vec![LifecycleEffect::ResourceWritten],
                previous_receipt.and_then(|receipt| receipt.backup.clone()),
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
                previous_receipt.and_then(|receipt| receipt.backup.clone()),
            ),
            InstallOutcome::DryRun => (
                LifecycleStatus::Previewed,
                vec![
                    LifecycleEffect::ResourceWritePreviewed,
                    LifecycleEffect::ReceiptWritePreviewed,
                ],
                None,
            ),
        };
        if !request.dry_run {
            manifest.upsert(SysRunEntry {
                os_id: request.os_id,
                item_id: request.item_id.clone(),
                label: request.label,
                status: if status == LifecycleStatus::Unchanged {
                    SysItemStatus::AlreadyInstalled
                } else {
                    SysItemStatus::Updated
                },
                detail: request.destination.display().to_string(),
                updated_at: String::new(),
                managed: true,
                profile_enabled: true,
                receipt: Some(SystemReceipt::ManagedFile(ManagedFileReceipt {
                    version: RECEIPT_VERSION,
                    destination: request.destination,
                    backup,
                    content_hash: hash_content(&request.content),
                    privileged: request.privileged,
                    restart_hint: request.restart_hint,
                })),
            });
            effects.push(LifecycleEffect::ReceiptWritten);
            save_manifest_with_host(&self.host, &self.context.shine_dir, &manifest).await?;
        }
        let mut result = LifecycleResultV1::new(LifecycleOperation::Upgrade, request.dry_run);
        result.push(LifecycleOutcomeV1::new(
            format!("sys/{}", request.item_id),
            None::<String>,
            status,
            effects,
        ));
        Ok(result)
    }

    pub async fn remove_managed_sys_file(
        &self,
        request: ManagedFileRemoveRequest,
    ) -> Result<LifecycleResultV1> {
        let mut manifest = load_manifest_with_host(&self.host, &self.context.shine_dir).await?;
        let position = manifest
            .entries
            .iter()
            .position(|entry| entry.os_id == request.os_id && entry.item_id == request.item_id);
        let mut result = LifecycleResultV1::new(LifecycleOperation::Uninstall, request.dry_run);
        let Some(position) = position else {
            return Ok(result);
        };
        let entry = manifest.entries[position].clone();
        let Some(SystemReceipt::ManagedFile(receipt)) = entry.receipt else {
            return Ok(result);
        };
        if receipt.privileged {
            bail!("managed Sys file requires a privileged host capability");
        }
        let outcome = uninstall_entry_with_host(
            &self.host,
            &app_entry_from_receipt(&receipt),
            request.dry_run,
            false,
        )
        .await?;
        let (status, effects, remove_receipt) = match outcome {
            UninstallOutcome::UserModified => (
                LifecycleStatus::Conflict,
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
            UninstallOutcome::RestoredBackup { .. }
            | UninstallOutcome::ForceRestoredBackup { .. } => (
                LifecycleStatus::Changed,
                vec![
                    LifecycleEffect::BackupRestored,
                    LifecycleEffect::ReceiptRemoved,
                ],
                true,
            ),
            UninstallOutcome::Removed | UninstallOutcome::ForceRemoved => (
                LifecycleStatus::Changed,
                vec![
                    LifecycleEffect::ResourceRemoved,
                    LifecycleEffect::ReceiptRemoved,
                ],
                true,
            ),
            UninstallOutcome::NotFound => (
                LifecycleStatus::Changed,
                vec![LifecycleEffect::ReceiptRemoved],
                true,
            ),
        };
        if remove_receipt {
            manifest.entries.remove(position);
            save_manifest_with_host(&self.host, &self.context.shine_dir, &manifest).await?;
        }
        result.push(LifecycleOutcomeV1::new(
            format!("sys/{}", request.item_id),
            None::<String>,
            status,
            effects,
        ));
        Ok(result)
    }
}

fn app_entry_from_receipt(receipt: &ManagedFileReceipt) -> AppEntry {
    AppEntry {
        source: "sys/managed-file".into(),
        destination: receipt.destination.clone(),
        backup: receipt.backup.clone(),
        content_hash: receipt.content_hash,
        install_strategy: AppInstallStrategy::Copy,
        uses_env: false,
        requires_admin: receipt.privileged,
    }
}

async fn load_manifest_with_host(
    host: &impl FileSystemHost,
    shine_dir: &Path,
) -> Result<SysRunManifest> {
    let path = shine_dir.join(SYS_MANIFEST_FILE);
    let mut manifest = match host.read(&path).await {
        Ok(bytes) => toml::from_slice(&bytes).context("failed to parse sys manifest")?,
        Err(error) if error.is_not_found() => SysRunManifest::default(),
        Err(error) => return Err(error.into_anyhow("failed to read sys manifest")),
    };
    match manifest.schema_version {
        0 => manifest.schema_version = SYS_MANIFEST_SCHEMA_VERSION,
        SYS_MANIFEST_SCHEMA_VERSION => {}
        version => bail!(
            "sys manifest schema version {version} is newer than this Shine supports ({SYS_MANIFEST_SCHEMA_VERSION})"
        ),
    }
    Ok(manifest)
}

async fn save_manifest_with_host(
    host: &impl FileSystemHost,
    shine_dir: &Path,
    manifest: &SysRunManifest,
) -> Result<()> {
    let content = toml::to_string_pretty(manifest).context("failed to serialize sys manifest")?;
    host.write_atomic(&shine_dir.join(SYS_MANIFEST_FILE), content.as_bytes())
        .await
        .map_err(|error| error.into_anyhow("failed to write sys manifest"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{
        InMemoryHost, PresetSnapshot, PresetSourceKind, RuntimeContext, RuntimePlatform,
    };
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn manifest_and_receipt_versions_are_owned_by_core() {
        let root =
            std::env::temp_dir().join(format!("shine-sys-manifest-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let path = root.join(SYS_MANIFEST_FILE);
        tokio::fs::write(&path, "entries = []\n").await.unwrap();
        let legacy = SysRunManifest::load(&root).await.unwrap();
        assert_eq!(legacy.schema_version, SYS_MANIFEST_SCHEMA_VERSION);
        legacy.save(&root).await.unwrap();

        let receipt = SystemReceipt::ManagedFile(ManagedFileReceipt {
            version: RECEIPT_VERSION,
            destination: PathBuf::from("/managed"),
            backup: None,
            content_hash: 1,
            privileged: false,
            restart_hint: None,
        });
        receipt.ensure_supported().unwrap();
        assert_eq!(receipt.driver(), SysDriverKind::ManagedFile);
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn managed_file_roundtrip_uses_virtual_host_and_receipt() {
        let runtime = CoreRuntime::new(
            InMemoryHost::new(),
            RuntimeContext {
                home_dir: PathBuf::from("/home/test"),
                shine_dir: PathBuf::from("/home/test/.shine"),
                presets_dir: PathBuf::from("/presets"),
                bin_dir: PathBuf::from("/bin"),
                platform: RuntimePlatform::Linux,
                env: BTreeMap::new(),
            },
            PresetSnapshot::builder(PresetSourceKind::External).build(),
        );
        let applied = runtime
            .apply_managed_sys_file(ManagedFileRequest {
                os_id: "linux".into(),
                item_id: "managed".into(),
                label: "Managed".into(),
                destination: PathBuf::from("/etc/example"),
                content: b"desired".to_vec(),
                privileged: false,
                restart_hint: None,
                dry_run: false,
            })
            .await
            .unwrap();
        assert_eq!(applied.summary().changed, 1);
        let removed = runtime
            .remove_managed_sys_file(ManagedFileRemoveRequest {
                os_id: "linux".into(),
                item_id: "managed".into(),
                dry_run: false,
            })
            .await
            .unwrap();
        assert_eq!(removed.summary().changed, 1);
        assert!(
            runtime
                .host()
                .read(Path::new("/etc/example"))
                .await
                .is_err()
        );
    }
}
