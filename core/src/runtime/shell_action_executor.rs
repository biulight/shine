mod action;
mod apply;
mod commit;
mod journal_io;
mod receipt_changes;
mod receipts;
pub(crate) use receipt_changes::{ShellReceiptRemoval, ShellReceiptTransition};
mod recovery_apply;
mod recovery_plan;
use action::ShellAction;
use journal_io::*;
use receipts::*;

// Transactional mutation and explicit recovery for Shell lifecycle resources.

use super::launcher::{
    PreparedLauncherResource, apply_prepared_launcher_resource, prepare_launcher_resources,
};
use super::review_path::review_path;
use super::shell::{
    ShellManifest, ShellManifestEntry, load_shell_manifest_with_host,
    shell_link_spec_from_manifest_entry,
};
use super::{
    CoreRuntime, FileKind, FileSystemHost, FileSystemObservationHost, LinkSpec,
    PrivilegedFileSystemHost,
};
use crate::action::{
    ACTION_IR_SCHEMA_VERSION, ActionIrV1, ActionKindV1, DeclarativeActionV1,
    ShellCacheFileRemovalV1, ShellCacheFileReplacementV1, ShellCacheRemovalSpecV1,
    ShellCacheReplacementSpecV1, ShellFileIdentityV1, ShellLauncherReceiptV1,
    ShellLauncherRemovalResourceV1, ShellLauncherResourceV1, ShellLauncherUpdateResourceV1,
    ShellProfileFileOwnershipV1, ShellProfileFileV1, ShellProfileReconciliationSpecV1,
    ShellReceiptRemovalV1, ShellReceiptTransitionV1, ShellRenderedFileRemovalSpecV1,
    ShellRenderedFileReplacementSpecV1, ShellSnapshotRemovalSpecV1, ShellSnapshotReplacementSpecV1,
    ShellTreeFileV1, managed_file_rollback_path, shell_snapshot_rollback_path,
    shell_snapshot_stage_path,
};
use crate::install::hash_content;
use crate::plan::{
    FilesystemAccessV1, PLAN_APPROVAL_SCHEMA_VERSION, PermissionSetV1, PermissionV1, PlanActionV1,
    PlanApprovalV1, PlanInputsV1, PlanOperationV1, PlanStepV1, PlanV1, SnapshotDigestV1,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub const SHELL_OPERATION_JOURNAL_FILE: &str = "shell-operation-journal.toml";
const SHELL_OPERATION_JOURNAL_SCHEMA_VERSION: u32 = 1;

pub(crate) struct ShellLauncherCreation<'a> {
    pub target: String,
    pub spec: &'a LinkSpec,
    pub receipt: ShellManifestEntry,
}

pub(crate) struct ShellLauncherUpdate<'a> {
    pub target: String,
    pub previous_receipt: ShellManifestEntry,
    pub desired_spec: &'a LinkSpec,
    pub desired_receipt: ShellManifestEntry,
}

pub(crate) struct ShellLauncherRemoval {
    pub target: String,
    pub previous_receipt: ShellManifestEntry,
}

pub(crate) struct ShellLegacyLauncherRemoval {
    pub target: String,
    pub resources: Vec<PreparedLauncherResource>,
}

#[derive(Clone)]
pub(crate) struct ShellSnapshotFile {
    pub relative_path: PathBuf,
    pub bytes: Vec<u8>,
    pub executable: bool,
}

pub(crate) struct ShellSnapshotReplacement {
    pub target: String,
    pub destination: PathBuf,
    pub files: Vec<ShellSnapshotFile>,
    pub receipt_transitions: Vec<ShellReceiptTransition>,
}

pub(crate) struct ShellRenderedFileReplacement {
    pub target: String,
    pub destination: PathBuf,
    pub bytes: Vec<u8>,
    pub unix_mode: Option<u32>,
    pub receipt_transitions: Vec<ShellReceiptTransition>,
}

pub(crate) struct ShellRenderedFileRemoval {
    pub target: String,
    pub destination: PathBuf,
    pub previous: ShellFileIdentityV1,
    pub previous_receipts: Vec<ShellReceiptRemoval>,
}

pub(crate) struct ShellCacheReplacementFile {
    pub destination: PathBuf,
    pub bytes: Vec<u8>,
    pub unix_mode: Option<u32>,
}

pub(crate) struct ShellCacheReplacement {
    pub target: String,
    pub files: Vec<ShellCacheReplacementFile>,
    pub receipt_transitions: Vec<ShellReceiptTransition>,
}

pub(crate) struct ShellCacheRemoval {
    pub target: String,
    pub files: Vec<(PathBuf, ShellFileIdentityV1)>,
    pub previous_receipts: Vec<ShellReceiptRemoval>,
}

pub(crate) struct ShellSnapshotRemoval {
    pub target: String,
    pub destination: PathBuf,
    pub previous_files: Vec<ShellTreeFileV1>,
    pub previous_receipts: Vec<ShellReceiptRemoval>,
}

pub(crate) struct ShellProfileReconciliation {
    pub target: String,
    pub files: Vec<ShellProfilePreparedFile>,
    pub receipt_transitions: Vec<ShellReceiptTransition>,
    pub receipt_removals: Vec<ShellReceiptRemoval>,
    pub legacy_targets: Vec<String>,
}

pub(crate) struct ShellProfilePreparedFile {
    pub destination: PathBuf,
    pub desired: Option<Vec<u8>>,
    pub unix_mode: Option<u32>,
    pub ownership: ShellProfileFileOwnershipV1,
    pub previous_block_hash: Option<u64>,
    pub desired_block_hash: Option<u64>,
}

pub(crate) struct ShellSharedReplacements<'a> {
    pub caches: &'a [ShellCacheReplacement],
    pub snapshots: &'a [ShellSnapshotReplacement],
    pub rendered_files: &'a [ShellRenderedFileReplacement],
    pub rendered_removals: &'a [ShellRenderedFileRemoval],
    pub cache_removals: &'a [ShellCacheRemoval],
    pub snapshot_removals: &'a [ShellSnapshotRemoval],
    pub profiles: &'a [ShellProfileReconciliation],
}

enum PreparedShellAction {
    Cache {
        files: Vec<PreparedShellCacheFile>,
    },
    Snapshot {
        destination: PathBuf,
        stage: PathBuf,
        rollback: PathBuf,
        previous_present: bool,
        files: Vec<ShellSnapshotFile>,
    },
    RenderedFile {
        destination: PathBuf,
        rollback: PathBuf,
        previous_present: bool,
        bytes: Vec<u8>,
        unix_mode: Option<u32>,
    },
    RemoveRenderedFile {
        destination: PathBuf,
        rollback: PathBuf,
    },
    RemoveCache {
        files: Vec<(PathBuf, PathBuf)>,
    },
    RemoveSnapshot {
        destination: PathBuf,
        rollback: PathBuf,
    },
    Profile {
        files: Vec<PreparedShellProfileFile>,
    },
    Create(Vec<PreparedLauncherResource>),
    Update(Vec<PreparedShellLauncherUpdateResource>),
    Remove(Vec<ShellLauncherRemovalResourceV1>),
}

struct PreparedShellCacheFile {
    destination: PathBuf,
    rollback: PathBuf,
    previous_present: bool,
    bytes: Vec<u8>,
    unix_mode: Option<u32>,
}

struct PreparedShellLauncherUpdateResource {
    previous: ShellLauncherResourceV1,
    desired: PreparedLauncherResource,
    rollback: std::path::PathBuf,
}

struct PreparedShellProfileFile {
    contract: ShellProfileFileV1,
    desired_bytes: Option<Vec<u8>>,
}

pub struct ShellOperationExecutionV1 {
    operation_id: String,
    _operation_guard: super::PrivilegedOperationGuard,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShellRecoveryReportV1 {
    pub operation_id: String,
    pub rolled_back_actions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ShellOperationJournalV1 {
    schema_version: u32,
    action_ir: ActionIrV1,
    approval: PlanApprovalV1,
    applied: Vec<String>,
    #[serde(default)]
    receipt_committed: Vec<String>,
}

impl ShellOperationJournalV1 {
    fn new(action_ir: ActionIrV1, approval: PlanApprovalV1) -> Self {
        Self {
            schema_version: SHELL_OPERATION_JOURNAL_SCHEMA_VERSION,
            action_ir,
            approval,
            applied: Vec::new(),
            receipt_committed: Vec::new(),
        }
    }

    fn validate(&self) -> Result<()> {
        if self.schema_version != SHELL_OPERATION_JOURNAL_SCHEMA_VERSION {
            bail!(
                "shell operation journal schema version {} is newer than this Shine supports ({SHELL_OPERATION_JOURNAL_SCHEMA_VERSION})",
                self.schema_version
            );
        }
        self.action_ir.validate()?;
        if self.action_ir.schema_version != ACTION_IR_SCHEMA_VERSION
            || self.approval.schema_version != PLAN_APPROVAL_SCHEMA_VERSION
        {
            bail!("unsupported action or approval schema in Shell operation journal");
        }
        for action in &self.action_ir.actions {
            ShellAction::from_kind(&action.kind)?;
        }
        if self.applied.iter().collect::<BTreeSet<_>>().len() != self.applied.len()
            || self.applied.iter().any(|action_id| {
                !self
                    .action_ir
                    .actions
                    .iter()
                    .any(|action| &action.action_id == action_id)
            })
        {
            bail!("Shell operation journal action state does not match its action IR");
        }
        if self.receipt_committed.iter().collect::<BTreeSet<_>>().len()
            != self.receipt_committed.len()
            || self.receipt_committed.iter().any(|action_id| {
                !self.applied.contains(action_id)
                    || !self.action_ir.actions.iter().any(|action| {
                        &action.action_id == action_id
                            && ShellAction::from_kind(&action.kind)
                                .is_ok_and(|kind| kind.requires_receipt_commit_marker())
                    })
            })
        {
            bail!("Shell operation journal receipt state does not match its action IR");
        }
        Ok(())
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(crate) async fn shell_operation_journal_bytes(&self) -> Result<Option<Vec<u8>>> {
        Ok(
            load_shell_operation_journal(self.host(), &self.context().shine_dir)
                .await?
                .map(|(_, bytes)| bytes),
        )
    }

    pub async fn plan_shell_operation_recovery(&self) -> Result<PlanV1> {
        let (journal, journal_bytes) =
            load_shell_operation_journal(self.host(), &self.context().shine_dir)
                .await?
                .context("no interrupted Shell operation is available for recovery")?;
        self.plan_shell_operation_recovery_from_journal(journal, journal_bytes)
            .await
    }

    pub(crate) async fn inspect_shell_operation_journal(
        &self,
    ) -> Result<Option<super::JournalInspection>> {
        let Some((journal, journal_bytes)) =
            load_shell_operation_journal(self.host(), &self.context().shine_dir).await?
        else {
            return Ok(None);
        };
        let mut prepared_actions = 0;
        let mut applied_actions = 0;
        let mut receipt_committed_actions = 0;
        for action in &journal.action_ir.actions {
            if journal.receipt_committed.contains(&action.action_id) {
                receipt_committed_actions += 1;
            } else if journal.applied.contains(&action.action_id) {
                applied_actions += 1;
            } else {
                prepared_actions += 1;
            }
        }
        Ok(Some(super::JournalInspection {
            operation_id: journal.action_ir.operation_id.clone(),
            prepared_actions,
            applied_actions,
            receipt_committed_actions,
            recovery_plan: self
                .plan_shell_operation_recovery_from_journal(journal, journal_bytes)
                .await?,
        }))
    }
}

fn resource_contract(resource: &PreparedLauncherResource) -> ShellLauncherResourceV1 {
    match resource {
        PreparedLauncherResource::Symlink {
            destination,
            target,
        } => ShellLauncherResourceV1::Symlink {
            destination: destination.clone(),
            target: target.clone(),
        },
        PreparedLauncherResource::File {
            destination,
            bytes,
            unix_mode,
        } => ShellLauncherResourceV1::File {
            destination: destination.clone(),
            desired_hash: hash_content(bytes),
            unix_mode: *unix_mode,
        },
    }
}

fn receipt_contract(entry: &ShellManifestEntry) -> ShellLauncherReceiptV1 {
    ShellLauncherReceiptV1 {
        launcher_format: entry.launcher_format.clone(),
        launcher_config_dir: entry.launcher_config_dir.clone(),

        category: entry.category.clone(),
        command: entry.command.clone(),
        mode: match entry.mode {
            super::ExternalShellMode::Snapshot => "snapshot",
            super::ExternalShellMode::Live => "live",
        }
        .to_string(),
        source_path: entry.source_path.clone(),
        rendered_path: entry.rendered_path.clone(),
        runtime: entry.runtime.clone(),
        bun_dependencies: entry.bun_dependencies.clone(),
        dependency_hash: entry.dependency_hash,
        transforms: entry.transforms.clone(),
        env: entry.env.clone(),
        needs_source: entry.needs_source,
        content_hash: entry.content_hash,
    }
}

fn shell_tree_contract(files: &[ShellSnapshotFile]) -> Vec<ShellTreeFileV1> {
    let mut contracts = files
        .iter()
        .map(|file| ShellTreeFileV1 {
            relative_path: file.relative_path.clone(),
            content_hash: hash_content(&file.bytes),
            executable: cfg!(unix).then_some(file.executable),
        })
        .collect::<Vec<_>>();
    contracts.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    contracts
}

async fn collect_shell_tree(
    host: &impl FileSystemObservationHost,
    root: &Path,
) -> Result<Option<Vec<ShellTreeFileV1>>> {
    let metadata = match host.metadata(root).await {
        Ok(metadata) => metadata,
        Err(error) if error.is_not_found() => return Ok(None),
        Err(error) => return Err(error.into_anyhow("observing Shell snapshot root")),
    };
    if metadata.kind != FileKind::Directory {
        bail!("Shell snapshot root is not a directory: {}", root.display());
    }
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for path in host
            .read_dir(&directory)
            .await
            .map_err(|error| error.into_anyhow("reading Shell snapshot tree"))?
        {
            let metadata = host
                .metadata(&path)
                .await
                .map_err(|error| error.into_anyhow("observing Shell snapshot entry"))?;
            match metadata.kind {
                FileKind::Directory => pending.push(path),
                FileKind::File => {
                    let bytes = host
                        .read(&path)
                        .await
                        .map_err(|error| error.into_anyhow("reading Shell snapshot entry"))?;
                    files.push(ShellTreeFileV1 {
                        relative_path: path
                            .strip_prefix(root)
                            .context("Shell snapshot entry escaped its root")?
                            .to_path_buf(),
                        content_hash: hash_content(&bytes),
                        executable: if cfg!(unix) {
                            metadata.unix_mode.map(|mode| mode & 0o111 != 0)
                        } else {
                            None
                        },
                    });
                }
                FileKind::Symlink => {
                    bail!(
                        "Shell snapshot contains an unsupported symlink: {}",
                        path.display()
                    )
                }
            }
        }
    }
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(Some(files))
}

pub(crate) async fn collect_shell_tree_for_action(
    host: &impl FileSystemObservationHost,
    root: &Path,
) -> Result<Option<Vec<ShellTreeFileV1>>> {
    collect_shell_tree(host, root).await
}

fn manifest_entry_from_receipt(receipt: &ShellLauncherReceiptV1) -> Result<ShellManifestEntry> {
    let delivery = receipt.validated_configuration()?;
    let mode = match delivery.mode {
        crate::action::shell_receipt::ReceiptMode::Snapshot => super::ExternalShellMode::Snapshot,
        crate::action::shell_receipt::ReceiptMode::Live => super::ExternalShellMode::Live,
    };
    Ok(ShellManifestEntry {
        launcher_format: receipt.launcher_format.clone(),
        launcher_config_dir: receipt.launcher_config_dir.clone(),

        category: receipt.category.clone(),
        command: receipt.command.clone(),
        mode,
        source_path: receipt.source_path.clone(),
        rendered_path: receipt.rendered_path.clone(),
        runtime: receipt.runtime.clone(),
        bun_dependencies: receipt.bun_dependencies.clone(),
        dependency_hash: receipt.dependency_hash,
        transforms: receipt.transforms.clone(),
        env: receipt.env.clone(),
        needs_source: receipt.needs_source,
        content_hash: receipt.content_hash,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReceiptState {
    Missing,
    Matching,
    Conflict,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ShellFileObservation {
    Missing,
    Regular(ShellFileIdentityV1),
    Other,
}

impl ShellFileObservation {
    fn matches(&self, expected: &ShellFileIdentityV1) -> bool {
        matches!(self, Self::Regular(actual)
            if actual.content_hash == expected.content_hash
                && shell_unix_modes_match(actual.unix_mode, expected.unix_mode))
    }
}

fn shell_unix_modes_match(actual: Option<u32>, expected: Option<u32>) -> bool {
    match (actual, expected) {
        (Some(actual), Some(expected)) => actual & 0o7777 == expected & 0o7777,
        (None, None) => true,
        // Windows has no Unix mode contract. InMemoryHost still records a synthetic mode
        // for files written there, while a real Windows host reports None.
        (Some(_), None) if cfg!(windows) => true,
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellTreeObservation {
    Missing,
    Exact,
    SafePartial,
    Changed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellSnapshotRecoveryAssessment {
    Stable,
    RemoveStage,
    RestoreMoved,
    RestoreReplaced,
    RemoveCreated,
    CleanupRollback,
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellRenderedFileRecoveryAssessment {
    Stable,
    RestoreMoved,
    RestoreReplaced,
    RemoveCreated,
    CleanupRollback,
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellRenderedFileRemovalRecoveryAssessment {
    Stable,
    RestoreMoved,
    CleanupRollback,
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellTreeRemovalRecoveryAssessment {
    Stable,
    RestoreMoved,
    CleanupRollback,
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellProfileFileRecoveryAssessment {
    Stable,
    RestoreWholeMoved,
    RestoreWholeReplaced,
    RemoveWholeCreated,
    RestoreSentinel,
    CleanupRollback,
    Blocked,
}

async fn observe_shell_file(
    host: &impl FileSystemObservationHost,
    path: &Path,
) -> Result<ShellFileObservation> {
    let metadata = match host.metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.is_not_found() => return Ok(ShellFileObservation::Missing),
        Err(error) => return Err(error.into_anyhow("observing Shell rendered file")),
    };
    if metadata.kind != FileKind::File {
        return Ok(ShellFileObservation::Other);
    }
    let bytes = host
        .read(path)
        .await
        .map_err(|error| error.into_anyhow("reading Shell rendered file"))?;
    Ok(ShellFileObservation::Regular(ShellFileIdentityV1 {
        content_hash: hash_content(&bytes),
        unix_mode: metadata.unix_mode,
    }))
}

fn shell_profile_block_hash(bytes: &[u8]) -> Result<Option<u64>> {
    let content = std::str::from_utf8(bytes).context("Shell profile is not UTF-8")?;
    Ok(super::profile::shell_sentinel_block(content).map(|block| hash_content(block.as_bytes())))
}

async fn observe_shell_profile_block_hash(
    host: &impl FileSystemObservationHost,
    path: &Path,
) -> Result<Option<u64>> {
    match host.read(path).await {
        Ok(bytes) => shell_profile_block_hash(&bytes),
        Err(error) if error.is_not_found() => Ok(None),
        Err(error) => Err(error.into_anyhow("reading Shell profile sentinel")),
    }
}

async fn assess_shell_profile_file_recovery(
    host: &impl FileSystemObservationHost,
    file: &ShellProfileFileV1,
    committed: bool,
) -> Result<ShellProfileFileRecoveryAssessment> {
    if file.ownership == ShellProfileFileOwnershipV1::WholeFile {
        return match (&file.previous, &file.desired) {
            (previous, Some(desired)) => Ok(
                match assess_shell_rendered_file_recovery(
                    host,
                    &file.destination,
                    &file.rollback,
                    previous.as_ref(),
                    desired,
                    committed,
                )
                .await?
                {
                    ShellRenderedFileRecoveryAssessment::Stable => {
                        ShellProfileFileRecoveryAssessment::Stable
                    }
                    ShellRenderedFileRecoveryAssessment::RestoreMoved => {
                        ShellProfileFileRecoveryAssessment::RestoreWholeMoved
                    }
                    ShellRenderedFileRecoveryAssessment::RestoreReplaced => {
                        ShellProfileFileRecoveryAssessment::RestoreWholeReplaced
                    }
                    ShellRenderedFileRecoveryAssessment::RemoveCreated => {
                        ShellProfileFileRecoveryAssessment::RemoveWholeCreated
                    }
                    ShellRenderedFileRecoveryAssessment::CleanupRollback => {
                        ShellProfileFileRecoveryAssessment::CleanupRollback
                    }
                    ShellRenderedFileRecoveryAssessment::Blocked => {
                        ShellProfileFileRecoveryAssessment::Blocked
                    }
                },
            ),
            (Some(previous), None) => Ok(
                match assess_shell_rendered_file_removal_recovery(
                    host,
                    &file.destination,
                    &file.rollback,
                    previous,
                    committed,
                )
                .await?
                {
                    ShellRenderedFileRemovalRecoveryAssessment::Stable => {
                        ShellProfileFileRecoveryAssessment::Stable
                    }
                    ShellRenderedFileRemovalRecoveryAssessment::RestoreMoved => {
                        ShellProfileFileRecoveryAssessment::RestoreWholeMoved
                    }
                    ShellRenderedFileRemovalRecoveryAssessment::CleanupRollback => {
                        ShellProfileFileRecoveryAssessment::CleanupRollback
                    }
                    ShellRenderedFileRemovalRecoveryAssessment::Blocked => {
                        ShellProfileFileRecoveryAssessment::Blocked
                    }
                },
            ),
            (None, None) => Ok(ShellProfileFileRecoveryAssessment::Blocked),
        };
    }

    let destination = observe_shell_file(host, &file.destination).await?;
    let rollback = observe_shell_file(host, &file.rollback).await?;
    let current_block = observe_shell_profile_block_hash(host, &file.destination).await?;
    if committed {
        if current_block != file.desired_block_hash {
            return Ok(ShellProfileFileRecoveryAssessment::Blocked);
        }
        return match &file.previous {
            Some(previous) if rollback.matches(previous) => {
                Ok(ShellProfileFileRecoveryAssessment::CleanupRollback)
            }
            Some(_) if rollback == ShellFileObservation::Missing => {
                Ok(ShellProfileFileRecoveryAssessment::Stable)
            }
            None if rollback == ShellFileObservation::Missing => {
                Ok(ShellProfileFileRecoveryAssessment::Stable)
            }
            _ => Ok(ShellProfileFileRecoveryAssessment::Blocked),
        };
    }
    if file
        .previous
        .as_ref()
        .is_some_and(|previous| destination.matches(previous))
        && rollback == ShellFileObservation::Missing
    {
        return Ok(ShellProfileFileRecoveryAssessment::Stable);
    }
    if file.previous.is_none()
        && destination == ShellFileObservation::Missing
        && rollback == ShellFileObservation::Missing
    {
        return Ok(ShellProfileFileRecoveryAssessment::Stable);
    }
    let rollback_safe = match &file.previous {
        Some(previous) => rollback.matches(previous),
        None => rollback == ShellFileObservation::Missing,
    };
    if rollback_safe && current_block == file.desired_block_hash {
        Ok(ShellProfileFileRecoveryAssessment::RestoreSentinel)
    } else {
        Ok(ShellProfileFileRecoveryAssessment::Blocked)
    }
}

async fn restore_shell_profile_sentinel(
    host: &impl FileSystemHost,
    file: &ShellProfileFileV1,
) -> Result<()> {
    let current_mode = match host.metadata(&file.destination).await {
        Ok(metadata) => metadata.unix_mode,
        Err(error) if error.is_not_found() => {
            file.previous.as_ref().and_then(|file| file.unix_mode)
        }
        Err(error) => return Err(error.into_anyhow("reading Shell profile mode")),
    };
    let current = match host.read(&file.destination).await {
        Ok(bytes) => String::from_utf8(bytes).context("Shell profile is not UTF-8")?,
        Err(error) if error.is_not_found() => String::new(),
        Err(error) => return Err(error.into_anyhow("reading current Shell profile")),
    };
    let cleaned = super::profile::remove_shell_sentinel(&current);
    let previous_block = if file.previous.is_some() {
        let bytes = host
            .read(&file.rollback)
            .await
            .map_err(|error| error.into_anyhow("reading Shell profile rollback"))?;
        let content = String::from_utf8(bytes).context("Shell profile rollback is not UTF-8")?;
        super::profile::shell_sentinel_block(&content).map(str::to_string)
    } else {
        None
    };
    let restored = previous_block.map_or(cleaned.clone(), |block| {
        format!("{cleaned}\n{}\n", block.trim_end_matches('\n'))
    });
    if file.previous.is_none() && restored.is_empty() {
        match host.remove_file(&file.destination).await {
            Ok(()) => {}
            Err(error) if error.is_not_found() => {}
            Err(error) => return Err(error.into_anyhow("removing created Shell profile")),
        }
    } else {
        host.write_atomic(&file.destination, restored.as_bytes())
            .await
            .map_err(|error| error.into_anyhow("restoring Shell profile sentinel"))?;
        if let Some(mode) = current_mode {
            host.set_mode(&file.destination, mode)
                .await
                .map_err(|error| error.into_anyhow("restoring Shell profile mode"))?;
        }
    }
    if file.previous.is_some() {
        host.remove_file(&file.rollback)
            .await
            .map_err(|error| error.into_anyhow("cleaning Shell profile rollback"))?;
    }
    Ok(())
}

async fn assess_shell_rendered_file_recovery(
    host: &impl FileSystemObservationHost,
    destination: &Path,
    rollback: &Path,
    previous: Option<&ShellFileIdentityV1>,
    desired: &ShellFileIdentityV1,
    committed: bool,
) -> Result<ShellRenderedFileRecoveryAssessment> {
    let destination = observe_shell_file(host, destination).await?;
    let rollback = observe_shell_file(host, rollback).await?;
    if committed {
        if !destination.matches(desired) {
            return Ok(ShellRenderedFileRecoveryAssessment::Blocked);
        }
        return match previous {
            Some(previous) if rollback.matches(previous) => {
                Ok(ShellRenderedFileRecoveryAssessment::CleanupRollback)
            }
            Some(_) | None if rollback == ShellFileObservation::Missing => {
                Ok(ShellRenderedFileRecoveryAssessment::Stable)
            }
            Some(_) | None => Ok(ShellRenderedFileRecoveryAssessment::Blocked),
        };
    }
    match previous {
        Some(previous) => {
            if destination.matches(previous) && rollback == ShellFileObservation::Missing {
                Ok(ShellRenderedFileRecoveryAssessment::Stable)
            } else if destination == ShellFileObservation::Missing && rollback.matches(previous) {
                Ok(ShellRenderedFileRecoveryAssessment::RestoreMoved)
            } else if destination.matches(desired) && rollback.matches(previous) {
                Ok(ShellRenderedFileRecoveryAssessment::RestoreReplaced)
            } else {
                Ok(ShellRenderedFileRecoveryAssessment::Blocked)
            }
        }
        None => {
            if rollback != ShellFileObservation::Missing {
                Ok(ShellRenderedFileRecoveryAssessment::Blocked)
            } else if destination == ShellFileObservation::Missing {
                Ok(ShellRenderedFileRecoveryAssessment::Stable)
            } else if destination.matches(desired) {
                Ok(ShellRenderedFileRecoveryAssessment::RemoveCreated)
            } else {
                Ok(ShellRenderedFileRecoveryAssessment::Blocked)
            }
        }
    }
}

async fn assess_shell_rendered_file_removal_recovery(
    host: &impl FileSystemObservationHost,
    destination: &Path,
    rollback: &Path,
    previous: &ShellFileIdentityV1,
    committed: bool,
) -> Result<ShellRenderedFileRemovalRecoveryAssessment> {
    let destination = observe_shell_file(host, destination).await?;
    let rollback = observe_shell_file(host, rollback).await?;
    if committed {
        if destination != ShellFileObservation::Missing {
            return Ok(ShellRenderedFileRemovalRecoveryAssessment::Blocked);
        }
        return if rollback.matches(previous) {
            Ok(ShellRenderedFileRemovalRecoveryAssessment::CleanupRollback)
        } else if rollback == ShellFileObservation::Missing {
            Ok(ShellRenderedFileRemovalRecoveryAssessment::Stable)
        } else {
            Ok(ShellRenderedFileRemovalRecoveryAssessment::Blocked)
        };
    }
    if destination.matches(previous) && rollback == ShellFileObservation::Missing {
        Ok(ShellRenderedFileRemovalRecoveryAssessment::Stable)
    } else if destination == ShellFileObservation::Missing && rollback.matches(previous) {
        Ok(ShellRenderedFileRemovalRecoveryAssessment::RestoreMoved)
    } else {
        Ok(ShellRenderedFileRemovalRecoveryAssessment::Blocked)
    }
}

async fn assess_shell_tree_removal_recovery(
    host: &impl FileSystemObservationHost,
    destination: &Path,
    rollback: &Path,
    previous: &[ShellTreeFileV1],
    committed: bool,
) -> Result<ShellTreeRemovalRecoveryAssessment> {
    let destination = observe_shell_tree(host, destination, previous, false).await?;
    let rollback = observe_shell_tree(host, rollback, previous, false).await?;
    if committed {
        if destination != ShellTreeObservation::Missing {
            return Ok(ShellTreeRemovalRecoveryAssessment::Blocked);
        }
        return if rollback == ShellTreeObservation::Exact {
            Ok(ShellTreeRemovalRecoveryAssessment::CleanupRollback)
        } else if rollback == ShellTreeObservation::Missing {
            Ok(ShellTreeRemovalRecoveryAssessment::Stable)
        } else {
            Ok(ShellTreeRemovalRecoveryAssessment::Blocked)
        };
    }
    if destination == ShellTreeObservation::Exact && rollback == ShellTreeObservation::Missing {
        Ok(ShellTreeRemovalRecoveryAssessment::Stable)
    } else if destination == ShellTreeObservation::Missing
        && rollback == ShellTreeObservation::Exact
    {
        Ok(ShellTreeRemovalRecoveryAssessment::RestoreMoved)
    } else {
        Ok(ShellTreeRemovalRecoveryAssessment::Blocked)
    }
}

fn rendered_file_recovery_permissions<'a>(
    assessment: ShellRenderedFileRecoveryAssessment,
    destination: &'a Path,
    rollback: &'a Path,
) -> Vec<(FilesystemAccessV1, &'a Path)> {
    match assessment {
        ShellRenderedFileRecoveryAssessment::RestoreMoved => vec![
            (FilesystemAccessV1::Write, destination),
            (FilesystemAccessV1::Remove, rollback),
        ],
        ShellRenderedFileRecoveryAssessment::RestoreReplaced => vec![
            (FilesystemAccessV1::Remove, destination),
            (FilesystemAccessV1::Write, destination),
            (FilesystemAccessV1::Remove, rollback),
        ],
        ShellRenderedFileRecoveryAssessment::RemoveCreated => {
            vec![(FilesystemAccessV1::Remove, destination)]
        }
        ShellRenderedFileRecoveryAssessment::CleanupRollback => {
            vec![(FilesystemAccessV1::Remove, rollback)]
        }
        ShellRenderedFileRecoveryAssessment::Stable
        | ShellRenderedFileRecoveryAssessment::Blocked => Vec::new(),
    }
}

fn rendered_file_removal_recovery_permissions<'a>(
    assessment: ShellRenderedFileRemovalRecoveryAssessment,
    destination: &'a Path,
    rollback: &'a Path,
) -> Vec<(FilesystemAccessV1, &'a Path)> {
    match assessment {
        ShellRenderedFileRemovalRecoveryAssessment::RestoreMoved => vec![
            (FilesystemAccessV1::Write, destination),
            (FilesystemAccessV1::Remove, rollback),
        ],
        ShellRenderedFileRemovalRecoveryAssessment::CleanupRollback => {
            vec![(FilesystemAccessV1::Remove, rollback)]
        }
        ShellRenderedFileRemovalRecoveryAssessment::Stable
        | ShellRenderedFileRemovalRecoveryAssessment::Blocked => Vec::new(),
    }
}

fn shell_tree_removal_recovery_permissions<'a>(
    assessment: ShellTreeRemovalRecoveryAssessment,
    destination: &'a Path,
    rollback: &'a Path,
) -> Vec<(FilesystemAccessV1, &'a Path)> {
    match assessment {
        ShellTreeRemovalRecoveryAssessment::RestoreMoved => vec![
            (FilesystemAccessV1::Write, destination),
            (FilesystemAccessV1::Remove, rollback),
        ],
        ShellTreeRemovalRecoveryAssessment::CleanupRollback => {
            vec![(FilesystemAccessV1::Remove, rollback)]
        }
        ShellTreeRemovalRecoveryAssessment::Stable
        | ShellTreeRemovalRecoveryAssessment::Blocked => Vec::new(),
    }
}

fn shell_profile_recovery_permissions(
    assessment: ShellProfileFileRecoveryAssessment,
    file: &ShellProfileFileV1,
) -> Vec<(FilesystemAccessV1, &Path)> {
    match assessment {
        ShellProfileFileRecoveryAssessment::RestoreWholeMoved => vec![
            (FilesystemAccessV1::Write, &file.destination),
            (FilesystemAccessV1::Remove, &file.rollback),
        ],
        ShellProfileFileRecoveryAssessment::RestoreWholeReplaced => vec![
            (FilesystemAccessV1::Remove, &file.destination),
            (FilesystemAccessV1::Write, &file.destination),
            (FilesystemAccessV1::Remove, &file.rollback),
        ],
        ShellProfileFileRecoveryAssessment::RemoveWholeCreated => {
            vec![(FilesystemAccessV1::Remove, &file.destination)]
        }
        ShellProfileFileRecoveryAssessment::RestoreSentinel => vec![
            (FilesystemAccessV1::Write, &file.destination),
            (FilesystemAccessV1::Remove, &file.destination),
            (FilesystemAccessV1::Remove, &file.rollback),
        ],
        ShellProfileFileRecoveryAssessment::CleanupRollback => {
            vec![(FilesystemAccessV1::Remove, &file.rollback)]
        }
        ShellProfileFileRecoveryAssessment::Stable
        | ShellProfileFileRecoveryAssessment::Blocked => Vec::new(),
    }
}

async fn observe_shell_tree(
    host: &impl FileSystemObservationHost,
    root: &Path,
    expected: &[ShellTreeFileV1],
    allow_safe_partial: bool,
) -> Result<ShellTreeObservation> {
    let Some(actual) = collect_shell_tree(host, root).await? else {
        return Ok(ShellTreeObservation::Missing);
    };
    // Old journals omit executable intent; retain their content-only recovery contract.
    let matches = |actual: &ShellTreeFileV1, expected: &ShellTreeFileV1| {
        actual.relative_path == expected.relative_path
            && actual.content_hash == expected.content_hash
            && expected
                .executable
                .is_none_or(|value| actual.executable == Some(value))
    };
    if actual.len() == expected.len() && actual.iter().zip(expected).all(|(a, e)| matches(a, e)) {
        return Ok(ShellTreeObservation::Exact);
    }
    if allow_safe_partial {
        let expected = expected
            .iter()
            .map(|file| (&file.relative_path, file))
            .collect::<BTreeMap<_, _>>();
        if actual.iter().all(|file| {
            expected
                .get(&file.relative_path)
                .is_some_and(|expected| matches(file, expected))
        }) {
            return Ok(ShellTreeObservation::SafePartial);
        }
    }
    Ok(ShellTreeObservation::Changed)
}

#[allow(clippy::too_many_arguments)]
async fn assess_shell_snapshot_recovery(
    host: &impl FileSystemObservationHost,
    destination: &Path,
    stage: &Path,
    rollback: &Path,
    previous_present: bool,
    previous_files: &[ShellTreeFileV1],
    desired_files: &[ShellTreeFileV1],
    committed: bool,
) -> Result<ShellSnapshotRecoveryAssessment> {
    let destination_previous = observe_shell_tree(host, destination, previous_files, false).await?;
    let destination_desired = observe_shell_tree(host, destination, desired_files, false).await?;
    let stage_desired = observe_shell_tree(host, stage, desired_files, true).await?;
    let rollback_previous = observe_shell_tree(host, rollback, previous_files, false).await?;
    if committed {
        if destination_desired != ShellTreeObservation::Exact
            || stage_desired != ShellTreeObservation::Missing
        {
            return Ok(ShellSnapshotRecoveryAssessment::Blocked);
        }
        return if previous_present {
            match rollback_previous {
                ShellTreeObservation::Exact => Ok(ShellSnapshotRecoveryAssessment::CleanupRollback),
                ShellTreeObservation::Missing => Ok(ShellSnapshotRecoveryAssessment::Stable),
                ShellTreeObservation::SafePartial | ShellTreeObservation::Changed => {
                    Ok(ShellSnapshotRecoveryAssessment::Blocked)
                }
            }
        } else if rollback_previous == ShellTreeObservation::Missing {
            Ok(ShellSnapshotRecoveryAssessment::Stable)
        } else {
            Ok(ShellSnapshotRecoveryAssessment::Blocked)
        };
    }

    let safe_stage = matches!(
        stage_desired,
        ShellTreeObservation::Missing
            | ShellTreeObservation::Exact
            | ShellTreeObservation::SafePartial
    );
    if !safe_stage {
        return Ok(ShellSnapshotRecoveryAssessment::Blocked);
    }
    if previous_present {
        if destination_previous == ShellTreeObservation::Exact
            && rollback_previous == ShellTreeObservation::Missing
        {
            return Ok(if stage_desired == ShellTreeObservation::Missing {
                ShellSnapshotRecoveryAssessment::Stable
            } else {
                ShellSnapshotRecoveryAssessment::RemoveStage
            });
        }
        if destination_previous == ShellTreeObservation::Missing
            && rollback_previous == ShellTreeObservation::Exact
        {
            return Ok(ShellSnapshotRecoveryAssessment::RestoreMoved);
        }
        if destination_desired == ShellTreeObservation::Exact
            && rollback_previous == ShellTreeObservation::Exact
            && stage_desired == ShellTreeObservation::Missing
        {
            return Ok(ShellSnapshotRecoveryAssessment::RestoreReplaced);
        }
    } else if rollback_previous == ShellTreeObservation::Missing {
        if destination_desired == ShellTreeObservation::Missing {
            return Ok(if stage_desired == ShellTreeObservation::Missing {
                ShellSnapshotRecoveryAssessment::Stable
            } else {
                ShellSnapshotRecoveryAssessment::RemoveStage
            });
        }
        if destination_desired == ShellTreeObservation::Exact
            && stage_desired == ShellTreeObservation::Missing
        {
            return Ok(ShellSnapshotRecoveryAssessment::RemoveCreated);
        }
    }
    Ok(ShellSnapshotRecoveryAssessment::Blocked)
}

fn snapshot_recovery_permissions<'a>(
    assessment: ShellSnapshotRecoveryAssessment,
    destination: &'a Path,
    stage: &'a Path,
    rollback: &'a Path,
) -> Vec<(FilesystemAccessV1, &'a Path)> {
    match assessment {
        ShellSnapshotRecoveryAssessment::Stable | ShellSnapshotRecoveryAssessment::Blocked => {
            Vec::new()
        }
        ShellSnapshotRecoveryAssessment::RemoveStage => {
            vec![(FilesystemAccessV1::Remove, stage)]
        }
        ShellSnapshotRecoveryAssessment::RestoreMoved => vec![
            (FilesystemAccessV1::Remove, stage),
            (FilesystemAccessV1::Write, destination),
            (FilesystemAccessV1::Remove, rollback),
        ],
        ShellSnapshotRecoveryAssessment::RestoreReplaced => vec![
            (FilesystemAccessV1::Remove, destination),
            (FilesystemAccessV1::Write, destination),
            (FilesystemAccessV1::Remove, rollback),
        ],
        ShellSnapshotRecoveryAssessment::RemoveCreated => {
            vec![(FilesystemAccessV1::Remove, destination)]
        }
        ShellSnapshotRecoveryAssessment::CleanupRollback => {
            vec![(FilesystemAccessV1::Remove, rollback)]
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LauncherObservation {
    Missing,
    Exact,
    Changed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LauncherUpdateObservation {
    Stable,
    RestoreMoved,
    RestoreReplaced,
    CleanupRollback,
    Changed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LauncherRemovalObservation {
    Stable,
    RestoreMoved,
    CleanupRollback,
    Changed,
}

impl LauncherRemovalObservation {
    fn identity(self) -> &'static [u8] {
        match self {
            Self::Stable => b"stable",
            Self::RestoreMoved => b"restore-moved",
            Self::CleanupRollback => b"cleanup-rollback",
            Self::Changed => b"changed",
        }
    }

    fn needs_recovery(self) -> bool {
        matches!(self, Self::RestoreMoved | Self::CleanupRollback)
    }

    fn required_permissions(
        self,
        resource: &ShellLauncherRemovalResourceV1,
    ) -> Vec<(FilesystemAccessV1, &Path)> {
        match self {
            Self::Stable | Self::Changed => Vec::new(),
            Self::RestoreMoved => vec![
                (FilesystemAccessV1::Write, resource.previous.destination()),
                (FilesystemAccessV1::Remove, resource.rollback.as_path()),
            ],
            Self::CleanupRollback => {
                vec![(FilesystemAccessV1::Remove, resource.rollback.as_path())]
            }
        }
    }
}

impl LauncherUpdateObservation {
    fn identity(self) -> &'static [u8] {
        match self {
            Self::Stable => b"stable",
            Self::RestoreMoved => b"restore-moved",
            Self::RestoreReplaced => b"restore-replaced",
            Self::CleanupRollback => b"cleanup-rollback",
            Self::Changed => b"changed",
        }
    }

    fn needs_recovery(self) -> bool {
        matches!(
            self,
            Self::RestoreMoved | Self::RestoreReplaced | Self::CleanupRollback
        )
    }

    fn required_permissions(
        self,
        resource: &ShellLauncherUpdateResourceV1,
    ) -> Vec<(FilesystemAccessV1, &Path)> {
        match self {
            Self::Stable | Self::Changed => Vec::new(),
            Self::RestoreMoved => vec![
                (FilesystemAccessV1::Write, resource.previous.destination()),
                (FilesystemAccessV1::Remove, resource.rollback.as_path()),
            ],
            Self::RestoreReplaced => vec![
                (FilesystemAccessV1::Remove, resource.desired.destination()),
                (FilesystemAccessV1::Write, resource.previous.destination()),
                (FilesystemAccessV1::Remove, resource.rollback.as_path()),
            ],
            Self::CleanupRollback => {
                vec![(FilesystemAccessV1::Remove, resource.rollback.as_path())]
            }
        }
    }
}

impl LauncherObservation {
    fn identity(self) -> &'static [u8] {
        match self {
            Self::Missing => b"missing",
            Self::Exact => b"exact",
            Self::Changed => b"changed",
        }
    }
}

async fn observe_launcher_resource(
    host: &impl FileSystemObservationHost,
    resource: &ShellLauncherResourceV1,
) -> Result<LauncherObservation> {
    let metadata = match host.metadata(resource.destination()).await {
        Ok(metadata) => metadata,
        Err(error) if error.is_not_found() => return Ok(LauncherObservation::Missing),
        Err(error) => return Err(error.into_anyhow("observing Shell launcher recovery path")),
    };
    let exact = match resource {
        ShellLauncherResourceV1::Symlink { target, .. } => {
            metadata.kind == FileKind::Symlink
                && host
                    .read_link(resource.destination())
                    .await
                    .is_ok_and(|current| current == *target)
        }
        ShellLauncherResourceV1::File {
            desired_hash,
            unix_mode,
            ..
        } => {
            metadata.kind == FileKind::File
                && host
                    .read(resource.destination())
                    .await
                    .is_ok_and(|bytes| hash_content(&bytes) == *desired_hash)
                && unix_mode.is_none_or(|expected| {
                    metadata
                        .unix_mode
                        .is_some_and(|mode| mode & 0o777 == expected)
                })
        }
    };
    Ok(if exact {
        LauncherObservation::Exact
    } else {
        LauncherObservation::Changed
    })
}

async fn observe_launcher_resource_at(
    host: &impl FileSystemObservationHost,
    resource: &ShellLauncherResourceV1,
    destination: &Path,
) -> Result<LauncherObservation> {
    let resource = match resource {
        ShellLauncherResourceV1::Symlink { target, .. } => ShellLauncherResourceV1::Symlink {
            destination: destination.to_path_buf(),
            target: target.clone(),
        },
        ShellLauncherResourceV1::File {
            desired_hash,
            unix_mode,
            ..
        } => ShellLauncherResourceV1::File {
            destination: destination.to_path_buf(),
            desired_hash: *desired_hash,
            unix_mode: *unix_mode,
        },
    };
    observe_launcher_resource(host, &resource).await
}

async fn observe_launcher_update_resource(
    host: &impl FileSystemObservationHost,
    resource: &ShellLauncherUpdateResourceV1,
    committed: bool,
) -> Result<LauncherUpdateObservation> {
    let previous = observe_launcher_resource(host, &resource.previous).await?;
    let desired = observe_launcher_resource(host, &resource.desired).await?;
    let rollback =
        observe_launcher_resource_at(host, &resource.previous, &resource.rollback).await?;
    if committed {
        return Ok(match rollback {
            LauncherObservation::Missing => LauncherUpdateObservation::Stable,
            LauncherObservation::Exact if desired == LauncherObservation::Exact => {
                LauncherUpdateObservation::CleanupRollback
            }
            LauncherObservation::Exact | LauncherObservation::Changed => {
                LauncherUpdateObservation::Changed
            }
        });
    }
    Ok(match (previous, desired, rollback) {
        (LauncherObservation::Exact, _, LauncherObservation::Missing) => {
            LauncherUpdateObservation::Stable
        }
        (
            LauncherObservation::Missing,
            LauncherObservation::Missing,
            LauncherObservation::Exact,
        ) => LauncherUpdateObservation::RestoreMoved,
        (_, LauncherObservation::Exact, LauncherObservation::Exact) => {
            LauncherUpdateObservation::RestoreReplaced
        }
        _ => LauncherUpdateObservation::Changed,
    })
}

async fn observe_launcher_removal_resource(
    host: &impl FileSystemObservationHost,
    resource: &ShellLauncherRemovalResourceV1,
    committed: bool,
) -> Result<LauncherRemovalObservation> {
    let previous = observe_launcher_resource(host, &resource.previous).await?;
    let rollback =
        observe_launcher_resource_at(host, &resource.previous, &resource.rollback).await?;
    Ok(if committed {
        match (previous, rollback) {
            (LauncherObservation::Missing, LauncherObservation::Missing) => {
                LauncherRemovalObservation::Stable
            }
            (LauncherObservation::Missing, LauncherObservation::Exact) => {
                LauncherRemovalObservation::CleanupRollback
            }
            _ => LauncherRemovalObservation::Changed,
        }
    } else {
        match (previous, rollback) {
            (LauncherObservation::Exact, LauncherObservation::Missing) => {
                LauncherRemovalObservation::Stable
            }
            (LauncherObservation::Missing, LauncherObservation::Exact) => {
                LauncherRemovalObservation::RestoreMoved
            }
            _ => LauncherRemovalObservation::Changed,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::InMemoryHost;

    #[test]
    fn manifest_action_and_recovery_share_receipt_delivery_rules_and_wire_shapes() {
        let base = ShellManifestEntry {
            launcher_format: None,
            launcher_config_dir: None,
            category: "demo".into(),
            command: "run".into(),
            mode: super::super::ExternalShellMode::Snapshot,
            source_path: "source.ps1".into(),
            rendered_path: "rendered.ps1".into(),
            runtime: "native".into(),
            bun_dependencies: None,
            dependency_hash: None,
            transforms: Vec::new(),
            env: Vec::new(),
            needs_source: false,
            content_hash: 1,
        };
        let mut cases = vec![(base.clone(), true)];
        let mut legacy_bun = base.clone();
        legacy_bun.runtime = "bun".into();
        legacy_bun.source_path = "source.ts".into();
        cases.push((legacy_bun, true));
        let mut entry = base.clone();
        entry.bun_dependencies = Some("locked".into());
        cases.push((entry, true));
        for live in [false, true] {
            let mut entry = base.clone();
            entry.launcher_format = Some("native-cmd-v2".into());
            if live {
                entry.mode = super::super::ExternalShellMode::Live;
            }
            cases.push((entry, true));
        }
        let mut live = base.clone();
        live.mode = super::super::ExternalShellMode::Live;
        live.runtime = "bun".into();
        live.source_path = "source.ts".into();
        live.transforms = vec!["template".into()];
        live.launcher_format = Some("live-bun-v2".into());
        live.launcher_config_dir = Some(std::env::temp_dir().join("shine-receipt-config"));
        cases.push((live.clone(), true));
        for change in 0..8 {
            let mut invalid = live.clone();
            match change {
                0 => invalid.runtime = "native".into(),
                1 => invalid.mode = super::super::ExternalShellMode::Snapshot,
                2 => invalid.transforms.clear(),
                3 => invalid.needs_source = true,
                4 => invalid.launcher_config_dir = Some("relative".into()),
                5 => invalid.launcher_config_dir = Some(std::env::temp_dir().join("../escape")),
                6 => invalid.launcher_format = Some("live-bun-v3".into()),
                _ => invalid.bun_dependencies = Some("unknown".into()),
            }
            cases.push((invalid, false));
        }
        for entry in ["unknown", "bun"] {
            let mut invalid = base.clone();
            invalid.runtime = entry.into();
            invalid.launcher_format = Some("native-cmd-v2".into());
            cases.push((invalid, false));
        }
        for (entry, expected) in cases {
            let receipt = receipt_contract(&entry);
            let before = serde_json::to_value(&receipt).unwrap();
            assert_eq!(
                shell_link_spec_from_manifest_entry(&entry).is_ok(),
                expected,
                "{entry:?}"
            );
            assert_eq!(
                receipt.validated_configuration().is_ok(),
                expected,
                "{receipt:?}"
            );
            let action = DeclarativeActionV1::create_shell_launcher(
                "launcher",
                "shell/demo/run",
                "launcher",
                receipt.clone(),
                vec![ShellLauncherResourceV1::File {
                    destination: "bin/run".into(),
                    desired_hash: 1,
                    unix_mode: None,
                }],
            );
            assert_eq!(
                ActionIrV1::new("receipt-test", vec![action])
                    .validate()
                    .is_ok(),
                expected
            );
            let restored = manifest_entry_from_receipt(&receipt);
            assert_eq!(restored.is_ok(), expected);
            if let Ok(restored) = restored {
                assert_eq!(restored, entry);
                assert_eq!(
                    serde_json::to_value(receipt_contract(&restored)).unwrap(),
                    before
                );
                let encoded = toml::to_string(&entry).unwrap();
                let decoded: ShellManifestEntry = toml::from_str(&encoded).unwrap();
                assert_eq!(toml::to_string(&decoded).unwrap(), encoded);
            }
        }
        let mut unsupported = receipt_contract(&base);
        unsupported.mode = "other".into();
        assert!(manifest_entry_from_receipt(&unsupported).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn sentinel_recovery_preserves_private_mode_and_unrelated_edits() {
        use crate::runtime::RealHost;
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("shine-profile-recovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let destination = dir.join("profile");
        let rollback = dir.join("rollback");
        let previous = b"original user content\n# >>> shine >>>\nsource old\n# <<< shine <<<\n";
        let current = b"new user edit\n# >>> shine >>>\nsource new\n# <<< shine <<<\n";
        for path in [&destination, &rollback] {
            std::fs::write(
                path,
                if path == &destination {
                    current.as_slice()
                } else {
                    previous.as_slice()
                },
            )
            .unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let file = ShellProfileFileV1 {
            destination: destination.clone(),
            rollback: rollback.clone(),
            ownership: ShellProfileFileOwnershipV1::SentinelBlock,
            previous: Some(ShellFileIdentityV1 {
                content_hash: hash_content(previous),
                unix_mode: Some(0o600),
            }),
            desired: None,
            previous_block_hash: None,
            desired_block_hash: None,
        };
        restore_shell_profile_sentinel(&RealHost, &file)
            .await
            .unwrap();
        let restored = std::fs::read_to_string(&destination).unwrap();
        assert!(restored.contains("new user edit"));
        assert!(restored.contains("source old"));
        assert!(!restored.contains("source new"));
        assert_eq!(
            std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(!rollback.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn shell_file_identity_uses_native_mode_contract() {
        let observation = ShellFileObservation::Regular(ShellFileIdentityV1 {
            content_hash: hash_content(b"cache"),
            unix_mode: Some(0o100644),
        });
        let without_mode = ShellFileIdentityV1 {
            content_hash: hash_content(b"cache"),
            unix_mode: None,
        };
        assert_eq!(observation.matches(&without_mode), cfg!(windows));
        assert!(!observation.matches(&ShellFileIdentityV1 {
            content_hash: hash_content(b"changed"),
            ..without_mode
        }));
        assert!(observation.matches(&ShellFileIdentityV1 {
            content_hash: hash_content(b"cache"),
            unix_mode: Some(0o644),
        }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn snapshot_recovery_binds_executable_intent_but_reads_legacy_journals() {
        let host = InMemoryHost::new();
        let root = std::env::temp_dir().join("shine-shell-mode-recovery");
        host.put_file_with_mode(root.join("helper"), b"helper".to_vec(), 0o100755);
        let expected = vec![ShellTreeFileV1 {
            relative_path: "helper".into(),
            content_hash: hash_content(b"helper"),
            executable: Some(true),
        }];
        assert_eq!(
            observe_shell_tree(&host, &root, &expected, false)
                .await
                .unwrap(),
            ShellTreeObservation::Exact
        );
        host.set_mode(&root.join("helper"), 0o644).await.unwrap();
        for partial in [false, true] {
            assert_eq!(
                observe_shell_tree(&host, &root, &expected, partial)
                    .await
                    .unwrap(),
                ShellTreeObservation::Changed
            );
        }
        let legacy_json = serde_json::json!({ "relative_path": "helper", "content_hash": hash_content(b"helper") });
        let legacy: ShellTreeFileV1 = serde_json::from_value(legacy_json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&legacy).unwrap(), legacy_json);
        assert_eq!(
            observe_shell_tree(&host, &root, &[legacy], false)
                .await
                .unwrap(),
            ShellTreeObservation::Exact
        );
    }

    #[tokio::test]
    async fn snapshot_recovery_blocks_changed_rollback_tree() {
        let host = InMemoryHost::new();
        let root = std::env::temp_dir().join("shine-shell-snapshot-recovery-test/demo");
        let stage = shell_snapshot_stage_path(&root);
        let rollback = shell_snapshot_rollback_path(&root);
        host.put_file(root.join("demo.sh"), b"desired".to_vec());
        host.put_file(rollback.join("demo.sh"), b"user-changed".to_vec());
        let previous_files = vec![ShellTreeFileV1 {
            relative_path: PathBuf::from("demo.sh"),
            content_hash: hash_content(b"previous"),
            executable: None,
        }];
        let desired_files = vec![ShellTreeFileV1 {
            relative_path: PathBuf::from("demo.sh"),
            content_hash: hash_content(b"desired"),
            executable: None,
        }];

        assert_eq!(
            assess_shell_snapshot_recovery(
                &host,
                &root,
                &stage,
                &rollback,
                true,
                &previous_files,
                &desired_files,
                false,
            )
            .await
            .unwrap(),
            ShellSnapshotRecoveryAssessment::Blocked
        );
    }

    #[tokio::test]
    async fn rendered_file_recovery_blocks_changed_rollback() {
        let host = InMemoryHost::new();
        let destination = std::env::temp_dir().join("shine-shell-rendered-recovery-test/demo.sh");
        let rollback = managed_file_rollback_path(&destination);
        host.put_file(&destination, b"desired".to_vec());
        host.put_file(&rollback, b"user-changed".to_vec());
        let previous = ShellFileIdentityV1 {
            content_hash: hash_content(b"previous"),
            unix_mode: None,
        };
        let desired = ShellFileIdentityV1 {
            content_hash: hash_content(b"desired"),
            unix_mode: None,
        };

        assert_eq!(
            assess_shell_rendered_file_recovery(
                &host,
                &destination,
                &rollback,
                Some(&previous),
                &desired,
                false,
            )
            .await
            .unwrap(),
            ShellRenderedFileRecoveryAssessment::Blocked
        );
    }
}
