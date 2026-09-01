//! Receipt-bound operation journal for declarative managed Sys resources.

use super::sys::{load_manifest_with_host, save_manifest_with_host};
use super::{
    CoreRuntime, FileKind, FileSystemHost, FileSystemObservationHost, PrivilegedFileSystemHost,
    RuntimeContext, SplitDnsHost, SplitDnsObservationHost, SplitDnsRequest, SysRunEntry,
    SysRunManifest,
};
use crate::action::{ACTION_IR_SCHEMA_VERSION, ActionIrV1, ActionKindV1, SysSplitDnsStateV1};
use crate::install::hash_content;
use crate::plan::{
    FilesystemAccessV1, PLAN_APPROVAL_SCHEMA_VERSION, PermissionV1, PlanActionV1, PlanApprovalV1,
    PlanInputsV1, PlanOperationV1, PlanStepV1, PlanV1, SnapshotDigestV1,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SYS_OPERATION_JOURNAL_FILE: &str = "sys-operation-journal.toml";
const SYS_OPERATION_JOURNAL_SCHEMA_VERSION: u32 = 1;

pub struct SysOperationExecutionV1 {
    pub operation_id: String,
    privileged_operation: Option<super::PrivilegedOperationGuard>,
}

impl std::fmt::Debug for SysOperationExecutionV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SysOperationExecutionV1")
            .field("operation_id", &self.operation_id)
            .field(
                "holds_privileged_operation",
                &self.privileged_operation.is_some(),
            )
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SysRecoveryReportV1 {
    pub operation_id: String,
    pub rolled_back_actions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SysReceiptTransitionV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<Box<SysRunEntry>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired: Option<Box<SysRunEntry>>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum SysJournalStateV1 {
    Prepared,
    Applied,
    ReceiptCommitted,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SysOperationJournalV1 {
    schema_version: u32,
    action_ir: ActionIrV1,
    approval: PlanApprovalV1,
    receipt: SysReceiptTransitionV1,
    state: SysJournalStateV1,
}

impl SysOperationJournalV1 {
    fn new(
        action_ir: ActionIrV1,
        approval: PlanApprovalV1,
        receipt: SysReceiptTransitionV1,
    ) -> Self {
        Self {
            schema_version: SYS_OPERATION_JOURNAL_SCHEMA_VERSION,
            action_ir,
            approval,
            receipt,
            state: SysJournalStateV1::Prepared,
        }
    }

    fn validate(&self) -> Result<()> {
        if self.schema_version != SYS_OPERATION_JOURNAL_SCHEMA_VERSION {
            bail!(
                "Sys operation journal schema version {} is newer than this Shine supports ({SYS_OPERATION_JOURNAL_SCHEMA_VERSION})",
                self.schema_version
            );
        }
        self.action_ir.validate()?;
        if self.action_ir.schema_version != ACTION_IR_SCHEMA_VERSION
            || self.approval.schema_version != PLAN_APPROVAL_SCHEMA_VERSION
        {
            bail!("unsupported action IR or Plan approval in Sys operation journal");
        }
        let [action] = self.action_ir.actions.as_slice() else {
            bail!("a Sys operation journal must contain exactly one action");
        };
        if self.receipt.previous.is_none() && self.receipt.desired.is_none() {
            bail!("a Sys operation journal requires a receipt transition");
        }
        if self.receipt.previous == self.receipt.desired {
            bail!("a Sys operation journal requires distinct receipt states");
        }
        let matches_target = matches!(action.kind, ActionKindV1::ReconcileSysProfileBlocks { .. })
            && action.target == "sys/profile"
            || self
                .receipt
                .previous
                .as_deref()
                .or(self.receipt.desired.as_deref())
                .is_some_and(|entry| action.target == format!("sys/{}", entry.item_id));
        if !matches_target {
            bail!("Sys action target does not match its receipt transition");
        }
        if !matches!(
            action.kind,
            ActionKindV1::CreateManagedFile { .. }
                | ActionKindV1::CreateManagedFileWithBackup { .. }
                | ActionKindV1::UpdateManagedFile { .. }
                | ActionKindV1::RelocateManagedFile { .. }
                | ActionKindV1::RemoveManagedFile { .. }
                | ActionKindV1::RemoveManagedFileWithBackup { .. }
                | ActionKindV1::ReconcileSysSplitDns { .. }
                | ActionKindV1::ReconcileSysProfileBlocks { .. }
        ) {
            bail!("Sys operation journal contains a non-Sys declarative action");
        }
        Ok(())
    }
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(crate) async fn sys_operation_journal_bytes(&self) -> Result<Option<Vec<u8>>> {
        Ok(
            load_sys_operation_journal(self.host(), &self.context().shine_dir)
                .await?
                .map(|(_, bytes)| bytes),
        )
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemObservationHost + SplitDnsObservationHost,
{
    pub async fn plan_sys_operation_recovery(&self) -> Result<PlanV1> {
        let (journal, journal_bytes) =
            load_sys_operation_journal(self.host(), &self.context().shine_dir)
                .await?
                .context("no interrupted Sys operation is available for recovery")?;
        let manifest = load_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let receipt_state = receipt_boundary(&manifest, &journal.receipt);
        let action = &journal.action_ir.actions[0];
        let assessment = assess_sys_action(self.host(), &action.kind).await?;
        let blocked = receipt_state == ReceiptBoundary::Conflict
            || assessment == SysRecoveryAssessment::Blocked;

        let manifest_bytes = read_missing_marker(
            self.host(),
            &self.context().shine_dir.join("sys-manifest.toml"),
        )
        .await?;
        let mut state = SnapshotDigestV1::builder("state:sys-recovery");
        state.add_observation("operation", PlanOperationV1::SysRecovery.as_str())?;
        state.add_observation("journal", &journal_bytes)?;
        state.add_observation("sys-manifest", &manifest_bytes)?;
        add_action_observations(self.host(), &mut state, &action.kind).await?;

        let requirements = journal
            .action_ir
            .permission_requirements(|path| review_path(self.context(), path));
        let mut required = requirements.required;
        for (access, path) in [
            (
                FilesystemAccessV1::Write,
                self.context().shine_dir.join("sys-manifest.toml"),
            ),
            (
                FilesystemAccessV1::Remove,
                self.context().shine_dir.join(SYS_OPERATION_JOURNAL_FILE),
            ),
        ] {
            required.insert(PermissionV1::Filesystem {
                access,
                path: review_path(self.context(), &path),
            });
        }
        let steps = vec![
            PlanStepV1::new(
                &action.target,
                Some(&action.resource),
                if blocked {
                    PlanActionV1::Blocked
                } else if journal.state == SysJournalStateV1::ReceiptCommitted {
                    PlanActionV1::None
                } else {
                    PlanActionV1::Update
                },
            )
            .with_diagnostic_code(if receipt_state == ReceiptBoundary::Conflict {
                "sys_recovery_receipt_conflict"
            } else if assessment == SysRecoveryAssessment::Blocked {
                "sys_recovery_resource_changed"
            } else {
                "sys_recovery_transaction"
            }),
            PlanStepV1::new(
                "sys",
                Some("operation-journal"),
                if blocked {
                    PlanActionV1::Preserve
                } else {
                    PlanActionV1::Remove
                },
            ),
        ];
        Ok(PlanV1::new(
            PlanOperationV1::SysRecovery,
            PlanInputsV1 {
                preset: SnapshotDigestV1::builder("preset:sys-recovery").finish(),
                state: state.finish(),
            },
            steps,
            required.clone(),
            &required,
            requirements.uncomputable_codes,
        ))
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost,
{
    pub(crate) async fn execute_sys_action_approved(
        &self,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_ir: ActionIrV1,
        receipt: SysReceiptTransitionV1,
        desired_content: Option<&[u8]>,
    ) -> Result<SysOperationExecutionV1> {
        approval.validate(plan)?;
        action_ir.validate()?;
        let requirements =
            action_ir.permission_requirements(|path| review_path(self.context(), path));
        if !requirements.uncomputable_codes.is_empty()
            || requirements
                .required
                .iter()
                .any(|permission| !approval.approved_permissions.contains(permission))
        {
            bail!("Sys action permissions were not included in the approved security Plan");
        }
        let [action] = action_ir.actions.as_slice() else {
            bail!("the Sys transaction slice accepts exactly one action");
        };
        if !plan.steps.iter().any(|step| {
            step.target == action.target
                && matches!(
                    step.action,
                    PlanActionV1::Create | PlanActionV1::Update | PlanActionV1::Remove
                )
        }) {
            bail!("the Sys action was not described by the approved security Plan");
        }
        let action_kind = action.kind.clone();
        let operation_guard = self.host().acquire_privileged_operation().await?;
        if load_sys_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .is_some()
        {
            bail!("an interrupted Sys operation must be recovered before starting another one");
        }
        let manifest = load_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        if receipt_boundary(&manifest, &receipt) != ReceiptBoundary::Previous {
            bail!("Sys action requires its exact previous receipt boundary");
        }
        preflight_sys_action(self.host(), &action_kind, desired_content).await?;
        let mut journal = SysOperationJournalV1::new(action_ir, approval.clone(), receipt.clone());
        save_sys_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        apply_sys_action(self.host(), &action_kind, desired_content).await?;
        journal.state = SysJournalStateV1::Applied;
        save_sys_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        Ok(SysOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            privileged_operation: Some(operation_guard),
        })
    }

    pub(crate) async fn execute_sys_profile_blocks_approved(
        &self,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_ir: ActionIrV1,
        receipt: SysReceiptTransitionV1,
        desired_contents: &[Option<Vec<u8>>],
    ) -> Result<SysOperationExecutionV1> {
        approval.validate(plan)?;
        action_ir.validate()?;
        let requirements =
            action_ir.permission_requirements(|path| review_path(self.context(), path));
        if !requirements.uncomputable_codes.is_empty()
            || requirements
                .required
                .iter()
                .any(|permission| !approval.approved_permissions.contains(permission))
        {
            bail!("Sys profile action permissions were not included in the approved Plan");
        }
        let [action] = action_ir.actions.as_slice() else {
            bail!("the Sys profile transaction accepts exactly one action");
        };
        let ActionKindV1::ReconcileSysProfileBlocks { files, .. } = &action.kind else {
            bail!("the Sys profile transaction requires a profile-block action");
        };
        if files.len() != desired_contents.len()
            || !plan.steps.iter().any(|step| {
                step.target == action.target
                    && matches!(step.action, PlanActionV1::Create | PlanActionV1::Update)
            })
        {
            bail!("the Sys profile action was not described by the approved Plan");
        }
        let operation_guard = self.host().acquire_privileged_operation().await?;
        if load_sys_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .is_some()
        {
            bail!("an interrupted Sys operation must be recovered before starting another one");
        }
        let manifest = load_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        if receipt_boundary(&manifest, &receipt) != ReceiptBoundary::Previous
            || assess_sys_profile_blocks(self.host(), &action.kind).await?
                != SysRecoveryAssessment::Unchanged
        {
            bail!("Sys profile state changed after Plan approval");
        }
        for (file, content) in files.iter().zip(desired_contents) {
            if content.as_deref().map(hash_content)
                != file.desired.as_ref().map(|id| id.content_hash)
            {
                bail!("Sys profile content does not match its action identity");
            }
        }
        let action_kind = action.kind.clone();
        let mut journal = SysOperationJournalV1::new(action_ir, approval.clone(), receipt);
        save_sys_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        apply_sys_profile_blocks(self.host(), &action_kind, desired_contents).await?;
        journal.state = SysJournalStateV1::Applied;
        save_sys_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        Ok(SysOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            privileged_operation: Some(operation_guard),
        })
    }

    pub(crate) async fn commit_sys_operation(
        &self,
        execution: &SysOperationExecutionV1,
    ) -> Result<()> {
        let _guard = if execution.privileged_operation.is_some() {
            None
        } else {
            Some(self.host().acquire_privileged_operation().await?)
        };
        let (mut journal, _) = load_sys_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .context("no Sys operation journal is available to commit")?;
        if journal.action_ir.operation_id != execution.operation_id
            || journal.state != SysJournalStateV1::Applied
        {
            bail!("Sys operation journal is not at its applied commit boundary");
        }
        let manifest = load_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        if receipt_boundary(&manifest, &journal.receipt) != ReceiptBoundary::Desired {
            bail!("Sys operation cannot commit before its desired receipt boundary is durable");
        }
        journal.state = SysJournalStateV1::ReceiptCommitted;
        save_sys_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        cleanup_sys_action(self.host(), &journal.action_ir.actions[0].kind).await?;
        remove_sys_operation_journal(self.host(), &self.context().shine_dir).await
    }

    pub async fn recover_sys_operation_approved(
        &self,
        approval: &PlanApprovalV1,
    ) -> Result<SysRecoveryReportV1> {
        let plan = self.plan_sys_operation_recovery().await?;
        approval.validate(&plan)?;
        if !plan.is_ready() {
            bail!("Sys recovery Plan is blocked");
        }
        let _guard = self.host().acquire_privileged_operation().await?;
        let (journal, _) = load_sys_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .context("no interrupted Sys operation is available for recovery")?;
        let action = &journal.action_ir.actions[0];
        let manifest = load_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let boundary = receipt_boundary(&manifest, &journal.receipt);
        if boundary == ReceiptBoundary::Conflict {
            bail!("Sys receipt state changed after recovery approval");
        }
        let mut rolled_back_actions = Vec::new();
        if journal.state == SysJournalStateV1::ReceiptCommitted {
            if boundary != ReceiptBoundary::Desired {
                bail!("committed Sys recovery requires its desired receipt boundary");
            }
            cleanup_sys_action(self.host(), &action.kind).await?;
        } else {
            if boundary == ReceiptBoundary::Desired {
                let mut previous_manifest = manifest;
                apply_receipt_transition(&mut previous_manifest, &journal.receipt, false)?;
                save_manifest_with_host(self.host(), &self.context().shine_dir, &previous_manifest)
                    .await?;
            }
            recover_sys_action(self.host(), &action.kind).await?;
            rolled_back_actions.push(action.action_id.clone());
        }
        remove_sys_operation_journal(self.host(), &self.context().shine_dir).await?;
        Ok(SysRecoveryReportV1 {
            operation_id: journal.action_ir.operation_id,
            rolled_back_actions,
        })
    }
}

pub(crate) fn apply_receipt_transition(
    manifest: &mut SysRunManifest,
    transition: &SysReceiptTransitionV1,
    desired: bool,
) -> Result<()> {
    let target = if desired {
        transition.desired.as_deref()
    } else {
        transition.previous.as_deref()
    };
    let identity = transition
        .previous
        .as_deref()
        .or(transition.desired.as_deref())
        .context("Sys receipt transition identity")?;
    manifest
        .entries
        .retain(|entry| !(entry.os_id == identity.os_id && entry.item_id == identity.item_id));
    if let Some(entry) = target {
        manifest.upsert(entry.clone());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReceiptBoundary {
    Previous,
    Desired,
    Conflict,
}

fn receipt_boundary(
    manifest: &SysRunManifest,
    transition: &SysReceiptTransitionV1,
) -> ReceiptBoundary {
    let identity = transition
        .previous
        .as_deref()
        .or(transition.desired.as_deref());
    let current = identity.and_then(|identity| {
        manifest
            .entries
            .iter()
            .find(|entry| entry.os_id == identity.os_id && entry.item_id == identity.item_id)
    });
    if current == transition.previous.as_deref() {
        ReceiptBoundary::Previous
    } else if current == transition.desired.as_deref() {
        ReceiptBoundary::Desired
    } else {
        ReceiptBoundary::Conflict
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SysRecoveryAssessment {
    Unchanged,
    Rollback,
    Blocked,
}

async fn preflight_sys_action(
    host: &(impl FileSystemObservationHost + SplitDnsObservationHost),
    kind: &ActionKindV1,
    desired_content: Option<&[u8]>,
) -> Result<()> {
    let assessment = assess_sys_action(host, kind).await?;
    if assessment != SysRecoveryAssessment::Unchanged {
        bail!("Sys resource changed after Plan approval or transaction material is occupied");
    }
    match kind {
        ActionKindV1::CreateManagedFile { desired_hash, .. }
        | ActionKindV1::CreateManagedFileWithBackup { desired_hash, .. }
        | ActionKindV1::UpdateManagedFile { desired_hash, .. }
        | ActionKindV1::RelocateManagedFile { desired_hash, .. } => {
            let content = desired_content.context("managed Sys file action requires content")?;
            if hash_content(content) != *desired_hash {
                bail!("managed Sys file content does not match its action identity");
            }
        }
        _ if desired_content.is_some() => {
            bail!("non-file Sys action received unexpected managed content")
        }
        _ => {}
    }
    Ok(())
}

async fn assess_sys_action(
    host: &(impl FileSystemObservationHost + SplitDnsObservationHost),
    kind: &ActionKindV1,
) -> Result<SysRecoveryAssessment> {
    match kind {
        ActionKindV1::CreateManagedFile {
            destination,
            desired_hash,
            ..
        } => assess_created_file(host, destination, *desired_hash).await,
        ActionKindV1::CreateManagedFileWithBackup {
            destination,
            backup,
            original_hash,
            desired_hash,
            ..
        } => {
            let destination = observe_file(host, destination).await?;
            let backup = observe_file(host, backup).await?;
            Ok(match (destination, backup) {
                (ObservedFile::Regular(bytes, _), ObservedFile::Missing)
                    if hash_content(&bytes) == *original_hash =>
                {
                    SysRecoveryAssessment::Unchanged
                }
                (ObservedFile::Missing, ObservedFile::Regular(bytes, _))
                    if hash_content(&bytes) == *original_hash =>
                {
                    SysRecoveryAssessment::Rollback
                }
                (ObservedFile::Regular(current, _), ObservedFile::Regular(original, _))
                    if hash_content(&current) == *desired_hash
                        && hash_content(&original) == *original_hash =>
                {
                    SysRecoveryAssessment::Rollback
                }
                _ => SysRecoveryAssessment::Blocked,
            })
        }
        ActionKindV1::UpdateManagedFile {
            destination,
            rollback,
            original_hash,
            desired_hash,
            ..
        } => assess_staged_file(host, destination, rollback, *original_hash, *desired_hash).await,
        ActionKindV1::RelocateManagedFile {
            previous_destination,
            previous_backup,
            previous_rollback,
            desired_destination,
            previous_present,
            previous_hash,
            desired_hash,
            ..
        } => {
            let old = observe_file(host, previous_destination).await?;
            let rollback = observe_file(host, previous_rollback).await?;
            let desired = observe_file(host, desired_destination).await?;
            let backup = match previous_backup {
                Some(backup) => Some(observe_file(host, &backup.path).await?),
                None => None,
            };
            let pristine = (!previous_present
                || matches!(&old, ObservedFile::Regular(bytes, _) if hash_content(bytes) == *previous_hash))
                && matches!(rollback, ObservedFile::Missing)
                && matches!(desired, ObservedFile::Missing)
                && previous_backup.as_ref().is_none_or(|identity| {
                    matches!(&backup, Some(ObservedFile::Regular(bytes, _)) if hash_content(bytes) == identity.hash)
                });
            if pristine {
                return Ok(SysRecoveryAssessment::Unchanged);
            }
            let applied = (!previous_present
                || matches!(&rollback, ObservedFile::Regular(bytes, _) if hash_content(bytes) == *previous_hash))
                && matches!(&desired, ObservedFile::Regular(bytes, _) if hash_content(bytes) == *desired_hash)
                && previous_backup.as_ref().map_or(
                    matches!(old, ObservedFile::Missing),
                    |identity| matches!(&old, ObservedFile::Regular(bytes, _) if hash_content(bytes) == identity.hash)
                        && matches!(backup, Some(ObservedFile::Missing)),
                );
            Ok(if applied {
                SysRecoveryAssessment::Rollback
            } else {
                SysRecoveryAssessment::Blocked
            })
        }
        ActionKindV1::RemoveManagedFile {
            destination,
            rollback,
            original_hash,
            ..
        } => assess_removed_file(host, destination, rollback, *original_hash, None).await,
        ActionKindV1::RemoveManagedFileWithBackup {
            destination,
            backup,
            rollback,
            managed_hash,
            backup_hash,
            ..
        } => {
            assess_removed_file(
                host,
                destination,
                rollback,
                *managed_hash,
                Some((backup, *backup_hash)),
            )
            .await
        }
        ActionKindV1::ReconcileSysSplitDns { previous, desired } => {
            assess_split_dns(host, previous.as_ref(), desired.as_ref()).await
        }
        ActionKindV1::ReconcileSysProfileBlocks { .. } => {
            assess_sys_profile_blocks(host, kind).await
        }
        _ => bail!("unsupported action in Sys operation journal"),
    }
}

async fn apply_sys_action(
    host: &(impl FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost),
    kind: &ActionKindV1,
    desired_content: Option<&[u8]>,
) -> Result<()> {
    match kind {
        ActionKindV1::CreateManagedFile {
            destination,
            requires_admin,
            ..
        } => {
            write_path(
                host,
                destination,
                desired_content.unwrap_or_default(),
                *requires_admin,
            )
            .await
        }
        ActionKindV1::CreateManagedFileWithBackup {
            destination,
            backup,
            requires_admin,
            ..
        } => {
            move_path(host, destination, backup, *requires_admin).await?;
            write_path(
                host,
                destination,
                desired_content.unwrap_or_default(),
                *requires_admin,
            )
            .await
        }
        ActionKindV1::UpdateManagedFile {
            destination,
            rollback,
            requires_admin,
            ..
        } => {
            move_path(host, destination, rollback, *requires_admin).await?;
            write_path(
                host,
                destination,
                desired_content.unwrap_or_default(),
                *requires_admin,
            )
            .await
        }
        ActionKindV1::RelocateManagedFile {
            previous_destination,
            previous_backup,
            previous_rollback,
            desired_destination,
            previous_present,
            previous_requires_admin,
            desired_requires_admin,
            ..
        } => {
            if *previous_present {
                move_path(
                    host,
                    previous_destination,
                    previous_rollback,
                    *previous_requires_admin,
                )
                .await?;
            }
            if let Some(backup) = previous_backup {
                move_path(
                    host,
                    &backup.path,
                    previous_destination,
                    *previous_requires_admin,
                )
                .await?;
            }
            write_path(
                host,
                desired_destination,
                desired_content.unwrap_or_default(),
                *desired_requires_admin,
            )
            .await
        }
        ActionKindV1::RemoveManagedFile {
            destination,
            rollback,
            requires_admin,
            ..
        } => move_path(host, destination, rollback, *requires_admin).await,
        ActionKindV1::RemoveManagedFileWithBackup {
            destination,
            backup,
            rollback,
            requires_admin,
            ..
        } => {
            move_path(host, destination, rollback, *requires_admin).await?;
            move_path(host, backup, destination, *requires_admin).await
        }
        ActionKindV1::ReconcileSysSplitDns { previous, desired } => {
            apply_split_dns_transition(host, previous.as_ref(), desired.as_ref()).await
        }
        ActionKindV1::ReconcileSysProfileBlocks { .. } => {
            bail!("Sys profile block actions require the profile executor")
        }
        _ => bail!("unsupported Sys action"),
    }
}

async fn recover_sys_action(
    host: &(impl FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost),
    kind: &ActionKindV1,
) -> Result<()> {
    match kind {
        ActionKindV1::CreateManagedFile {
            destination,
            desired_hash,
            requires_admin,
        } => {
            if file_hash(host, destination).await? == Some(*desired_hash) {
                remove_path(host, destination, *requires_admin).await?;
            }
            Ok(())
        }
        ActionKindV1::CreateManagedFileWithBackup {
            destination,
            backup,
            original_hash,
            desired_hash,
            requires_admin,
        } => {
            if file_hash(host, destination).await? == Some(*desired_hash) {
                remove_path(host, destination, *requires_admin).await?;
            }
            if file_hash(host, backup).await? == Some(*original_hash) {
                move_path(host, backup, destination, *requires_admin).await?;
            }
            Ok(())
        }
        ActionKindV1::UpdateManagedFile {
            destination,
            rollback,
            original_hash,
            desired_hash,
            requires_admin,
            ..
        } => {
            if file_hash(host, destination).await? == Some(*desired_hash) {
                remove_path(host, destination, *requires_admin).await?;
            }
            if file_hash(host, rollback).await? == Some(*original_hash) {
                move_path(host, rollback, destination, *requires_admin).await?;
            }
            Ok(())
        }
        ActionKindV1::RelocateManagedFile {
            previous_destination,
            previous_backup,
            previous_rollback,
            desired_destination,
            previous_present,
            previous_hash,
            desired_hash,
            previous_requires_admin,
            desired_requires_admin,
            ..
        } => {
            if file_hash(host, desired_destination).await? == Some(*desired_hash) {
                remove_path(host, desired_destination, *desired_requires_admin).await?;
            }
            if let Some(backup) = previous_backup
                && file_hash(host, previous_destination).await? == Some(backup.hash)
            {
                move_path(
                    host,
                    previous_destination,
                    &backup.path,
                    *previous_requires_admin,
                )
                .await?;
            }
            if *previous_present
                && file_hash(host, previous_rollback).await? == Some(*previous_hash)
            {
                move_path(
                    host,
                    previous_rollback,
                    previous_destination,
                    *previous_requires_admin,
                )
                .await?;
            }
            Ok(())
        }
        ActionKindV1::RemoveManagedFile {
            destination,
            rollback,
            original_hash,
            requires_admin,
            ..
        } => {
            if file_hash(host, rollback).await? == Some(*original_hash) {
                move_path(host, rollback, destination, *requires_admin).await?;
            }
            Ok(())
        }
        ActionKindV1::RemoveManagedFileWithBackup {
            destination,
            backup,
            rollback,
            managed_hash,
            backup_hash,
            requires_admin,
            ..
        } => {
            if file_hash(host, destination).await? == Some(*backup_hash) {
                move_path(host, destination, backup, *requires_admin).await?;
            }
            if file_hash(host, rollback).await? == Some(*managed_hash) {
                move_path(host, rollback, destination, *requires_admin).await?;
            }
            Ok(())
        }
        ActionKindV1::ReconcileSysSplitDns { previous, desired } => {
            recover_split_dns_transition(host, previous.as_ref(), desired.as_ref()).await
        }
        ActionKindV1::ReconcileSysProfileBlocks { .. } => {
            recover_sys_profile_blocks(host, kind).await
        }
        _ => bail!("unsupported Sys recovery action"),
    }
}

async fn cleanup_sys_action(
    host: &(impl FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost),
    kind: &ActionKindV1,
) -> Result<()> {
    match kind {
        ActionKindV1::UpdateManagedFile {
            rollback,
            original_hash,
            requires_admin,
            ..
        }
        | ActionKindV1::RemoveManagedFile {
            rollback,
            original_hash,
            requires_admin,
            ..
        } => remove_exact(host, rollback, *original_hash, *requires_admin).await,
        ActionKindV1::RelocateManagedFile {
            previous_rollback,
            previous_present,
            previous_hash,
            previous_requires_admin,
            ..
        } if *previous_present => {
            remove_exact(
                host,
                previous_rollback,
                *previous_hash,
                *previous_requires_admin,
            )
            .await
        }
        ActionKindV1::RemoveManagedFileWithBackup {
            rollback,
            managed_hash,
            requires_admin,
            ..
        } => remove_exact(host, rollback, *managed_hash, *requires_admin).await,
        ActionKindV1::ReconcileSysProfileBlocks { files, .. } => {
            for file in files {
                if let Some(previous) = &file.previous {
                    remove_exact(host, &file.rollback, previous.content_hash, false).await?;
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

async fn assess_sys_profile_blocks(
    host: &impl FileSystemObservationHost,
    kind: &ActionKindV1,
) -> Result<SysRecoveryAssessment> {
    let ActionKindV1::ReconcileSysProfileBlocks { os_id, files } = kind else {
        bail!("expected Sys profile block action");
    };
    let mut needs_rollback = false;
    for file in files {
        let destination = observe_file(host, &file.destination).await?;
        let rollback = observe_file(host, &file.rollback).await?;
        let pristine_destination = match (&file.previous, &destination) {
            (None, ObservedFile::Missing) => true,
            (Some(identity), ObservedFile::Regular(bytes, mode)) => {
                hash_content(bytes) == identity.content_hash
                    && mode_matches(*mode, identity.unix_mode)
            }
            _ => false,
        };
        if pristine_destination && matches!(rollback, ObservedFile::Missing) {
            continue;
        }
        let rollback_matches = match (&file.previous, &rollback) {
            (None, ObservedFile::Missing) => true,
            (Some(identity), ObservedFile::Regular(bytes, mode)) => {
                hash_content(bytes) == identity.content_hash
                    && mode_matches(*mode, identity.unix_mode)
            }
            _ => false,
        };
        let destination_owned_hash = match &destination {
            ObservedFile::Missing => None,
            ObservedFile::Regular(bytes, _) => std::str::from_utf8(bytes)
                .ok()
                .and_then(|content| super::sys_profile::sys_owned_blocks_hash(content, os_id)),
            ObservedFile::Other => return Ok(SysRecoveryAssessment::Blocked),
        };
        if rollback_matches && destination_owned_hash == file.desired_owned_hash {
            needs_rollback = true;
        } else {
            return Ok(SysRecoveryAssessment::Blocked);
        }
    }
    Ok(if needs_rollback {
        SysRecoveryAssessment::Rollback
    } else {
        SysRecoveryAssessment::Unchanged
    })
}

async fn apply_sys_profile_blocks(
    host: &impl FileSystemHost,
    kind: &ActionKindV1,
    desired_contents: &[Option<Vec<u8>>],
) -> Result<()> {
    let ActionKindV1::ReconcileSysProfileBlocks { files, .. } = kind else {
        bail!("expected Sys profile block action");
    };
    for (file, desired) in files.iter().zip(desired_contents) {
        if file.previous.is_some() {
            host.rename(&file.destination, &file.rollback)
                .await
                .map_err(|error| error.into_anyhow("staging Sys profile block rollback"))?;
        }
        if let Some(desired) = desired {
            host.write_atomic(&file.destination, desired)
                .await
                .map_err(|error| error.into_anyhow("writing Sys profile blocks"))?;
            if let Some(mode) = file
                .previous
                .as_ref()
                .and_then(|identity| identity.unix_mode)
                .or(Some(0o644))
            {
                host.set_mode(&file.destination, mode)
                    .await
                    .map_err(|error| error.into_anyhow("restoring Sys profile mode"))?;
            }
        }
    }
    Ok(())
}

async fn recover_sys_profile_blocks(
    host: &(impl FileSystemHost + PrivilegedFileSystemHost),
    kind: &ActionKindV1,
) -> Result<()> {
    let ActionKindV1::ReconcileSysProfileBlocks { os_id, files } = kind else {
        bail!("expected Sys profile block action");
    };
    for file in files.iter().rev() {
        let current = match host.read(&file.destination).await {
            Ok(bytes) => String::from_utf8(bytes).context("Sys profile is not UTF-8")?,
            Err(error) if error.is_not_found() => String::new(),
            Err(error) => return Err(error.into_anyhow("reading interrupted Sys profile")),
        };
        let current_owned = super::sys_profile::sys_owned_blocks_hash(&current, os_id);
        if current_owned != file.desired_owned_hash {
            if file.previous_owned_hash == current_owned {
                continue;
            }
            bail!("Sys profile owned blocks changed after the interrupted operation");
        }
        let previous = match &file.previous {
            Some(identity) => {
                let bytes = host
                    .read(&file.rollback)
                    .await
                    .map_err(|error| error.into_anyhow("reading Sys profile rollback"))?;
                if hash_content(&bytes) != identity.content_hash {
                    bail!("Sys profile rollback material changed");
                }
                String::from_utf8(bytes).context("Sys profile rollback is not UTF-8")?
            }
            None => String::new(),
        };
        let restored = super::sys_profile::restore_sys_owned_blocks(&current, &previous, os_id);
        if restored.trim().is_empty() && file.previous.is_none() {
            match host.remove_file(&file.destination).await {
                Ok(()) => {}
                Err(error) if error.is_not_found() => {}
                Err(error) => return Err(error.into_anyhow("removing created Sys profile")),
            }
        } else {
            host.write_atomic(&file.destination, restored.as_bytes())
                .await
                .map_err(|error| error.into_anyhow("restoring Sys profile owned blocks"))?;
            if let Some(mode) = file
                .previous
                .as_ref()
                .and_then(|identity| identity.unix_mode)
            {
                host.set_mode(&file.destination, mode)
                    .await
                    .map_err(|error| error.into_anyhow("restoring Sys profile mode"))?;
            }
        }
        if let Some(previous) = &file.previous {
            remove_exact(host, &file.rollback, previous.content_hash, false).await?;
        }
    }
    Ok(())
}

fn mode_matches(observed: Option<u32>, expected: Option<u32>) -> bool {
    expected.is_none() || observed == expected
}

async fn assess_created_file(
    host: &impl FileSystemObservationHost,
    destination: &Path,
    desired_hash: u64,
) -> Result<SysRecoveryAssessment> {
    Ok(match observe_file(host, destination).await? {
        ObservedFile::Missing => SysRecoveryAssessment::Unchanged,
        ObservedFile::Regular(bytes, _) if hash_content(&bytes) == desired_hash => {
            SysRecoveryAssessment::Rollback
        }
        _ => SysRecoveryAssessment::Blocked,
    })
}

async fn assess_staged_file(
    host: &impl FileSystemObservationHost,
    destination: &Path,
    rollback: &Path,
    previous_hash: u64,
    desired_hash: u64,
) -> Result<SysRecoveryAssessment> {
    let destination = observe_file(host, destination).await?;
    let rollback = observe_file(host, rollback).await?;
    Ok(match (destination, rollback) {
        (ObservedFile::Regular(bytes, _), ObservedFile::Missing)
            if hash_content(&bytes) == previous_hash =>
        {
            SysRecoveryAssessment::Unchanged
        }
        (ObservedFile::Missing, ObservedFile::Regular(bytes, _))
            if hash_content(&bytes) == previous_hash =>
        {
            SysRecoveryAssessment::Rollback
        }
        (ObservedFile::Regular(current, _), ObservedFile::Regular(previous, _))
            if hash_content(&current) == desired_hash
                && hash_content(&previous) == previous_hash =>
        {
            SysRecoveryAssessment::Rollback
        }
        _ => SysRecoveryAssessment::Blocked,
    })
}

async fn assess_removed_file(
    host: &impl FileSystemObservationHost,
    destination_path: &Path,
    rollback_path: &Path,
    managed_hash: u64,
    backup: Option<(&PathBuf, u64)>,
) -> Result<SysRecoveryAssessment> {
    let destination = observe_file(host, destination_path).await?;
    let rollback = observe_file(host, rollback_path).await?;
    let backup_state = match backup {
        Some((path, _)) => Some(observe_file(host, path).await?),
        None => None,
    };
    let pristine = matches!(&destination, ObservedFile::Regular(bytes, _) if hash_content(bytes) == managed_hash)
        && matches!(rollback, ObservedFile::Missing)
        && backup.is_none_or(|(_, hash)| matches!(&backup_state, Some(ObservedFile::Regular(bytes, _)) if hash_content(bytes) == hash));
    if pristine {
        return Ok(SysRecoveryAssessment::Unchanged);
    }
    let applied = matches!(&rollback, ObservedFile::Regular(bytes, _) if hash_content(bytes) == managed_hash)
        && match backup {
            None => matches!(destination, ObservedFile::Missing),
            Some((_, hash)) => {
                matches!(&destination, ObservedFile::Regular(bytes, _) if hash_content(bytes) == hash)
                    && matches!(backup_state, Some(ObservedFile::Missing))
            }
        };
    Ok(if applied {
        SysRecoveryAssessment::Rollback
    } else {
        SysRecoveryAssessment::Blocked
    })
}

async fn assess_split_dns(
    host: &impl SplitDnsObservationHost,
    previous: Option<&SysSplitDnsStateV1>,
    desired: Option<&SysSplitDnsStateV1>,
) -> Result<SysRecoveryAssessment> {
    let previous_current = match previous {
        Some(state) => split_dns_matches(host, state).await?,
        None => false,
    };
    let desired_current = match desired {
        Some(state) => split_dns_matches(host, state).await?,
        None => false,
    };
    let resources_differ = previous.zip(desired).is_some_and(|(previous, desired)| {
        previous.resource != desired.resource || previous.os_id != desired.os_id
    });
    if resources_differ {
        let previous_exists = split_dns_exists(host, previous.expect("zipped above")).await?;
        let desired_exists = split_dns_exists(host, desired.expect("zipped above")).await?;
        return Ok(if previous_current && !desired_exists {
            SysRecoveryAssessment::Unchanged
        } else if !previous_exists && desired_current {
            SysRecoveryAssessment::Rollback
        } else {
            SysRecoveryAssessment::Blocked
        });
    }
    let state = previous.or(desired).context("split DNS transition state")?;
    let exists = split_dns_exists(host, state).await?;
    Ok(if previous.is_some() && previous_current {
        SysRecoveryAssessment::Unchanged
    } else if (desired.is_none() && !exists) || (desired.is_some() && desired_current) {
        SysRecoveryAssessment::Rollback
    } else if previous.is_none() && !exists {
        SysRecoveryAssessment::Unchanged
    } else {
        SysRecoveryAssessment::Blocked
    })
}

async fn apply_split_dns_transition(
    host: &impl SplitDnsHost,
    previous: Option<&SysSplitDnsStateV1>,
    desired: Option<&SysSplitDnsStateV1>,
) -> Result<()> {
    if let Some(desired) = desired {
        host.apply_split_dns(&split_dns_request(desired)).await?;
    }
    if let Some(previous) = previous
        && desired.is_none_or(|desired| {
            desired.resource != previous.resource || desired.os_id != previous.os_id
        })
    {
        host.remove_split_dns(&split_dns_request(previous)).await?;
    }
    Ok(())
}

async fn recover_split_dns_transition(
    host: &impl SplitDnsHost,
    previous: Option<&SysSplitDnsStateV1>,
    desired: Option<&SysSplitDnsStateV1>,
) -> Result<()> {
    if let Some(desired) = desired
        && split_dns_matches(host, desired).await?
        && previous.is_none_or(|previous| {
            previous.resource != desired.resource || previous.os_id != desired.os_id
        })
    {
        host.remove_split_dns(&split_dns_request(desired)).await?;
    }
    if let Some(previous) = previous {
        host.apply_split_dns(&split_dns_request(previous)).await?;
    }
    Ok(())
}

fn split_dns_request(state: &SysSplitDnsStateV1) -> SplitDnsRequest {
    let marker = format!("Managed by shine: split-dns:{}", state.item_id);
    let content = if state.os_id == "windows" {
        format!(
            "{marker}\n{}\n{}",
            state.resource.display(),
            state.servers.join(",")
        )
        .into_bytes()
    } else if state.os_id == "macos" {
        format!(
            "# {marker}\n{}\n",
            state
                .servers
                .iter()
                .map(|server| format!("nameserver {server}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
        .into_bytes()
    } else {
        format!(
            "# {marker}\n[Resolve]\nDNS={}\nDomains=~{}\n",
            state.servers.join(" "),
            state.domain
        )
        .into_bytes()
    };
    SplitDnsRequest {
        os_id: state.os_id.clone(),
        item_id: state.item_id.clone(),
        domain: state.domain.clone(),
        servers: state.servers.clone(),
        resource: state.resource.clone(),
        content,
    }
}

async fn split_dns_matches(
    host: &impl SplitDnsObservationHost,
    state: &SysSplitDnsStateV1,
) -> Result<bool> {
    let request = split_dns_request(state);
    let current = host.inspect_split_dns(&request).await?;
    Ok(current.exists
        && hash_content(&current.content) == state.content_hash
        && current.content == request.content)
}

async fn split_dns_exists(
    host: &impl SplitDnsObservationHost,
    state: &SysSplitDnsStateV1,
) -> Result<bool> {
    Ok(host
        .inspect_split_dns(&split_dns_request(state))
        .await?
        .exists)
}

#[derive(Debug)]
enum ObservedFile {
    Missing,
    Regular(Vec<u8>, Option<u32>),
    Other,
}

async fn observe_file(host: &impl FileSystemObservationHost, path: &Path) -> Result<ObservedFile> {
    match host.metadata(path).await {
        Ok(metadata) if metadata.kind == FileKind::File => {
            let bytes = host
                .read(path)
                .await
                .map_err(|error| error.into_anyhow("reading Sys transaction file"))?;
            Ok(ObservedFile::Regular(bytes, metadata.unix_mode))
        }
        Ok(_) => Ok(ObservedFile::Other),
        Err(error) if error.is_not_found() => Ok(ObservedFile::Missing),
        Err(error) => Err(error.into_anyhow("inspecting Sys transaction file")),
    }
}

async fn file_hash(host: &impl FileSystemHost, path: &Path) -> Result<Option<u64>> {
    match host.read(path).await {
        Ok(bytes) => Ok(Some(hash_content(&bytes))),
        Err(error) if error.is_not_found() => Ok(None),
        Err(error) => Err(error.into_anyhow("reading Sys transaction file")),
    }
}

async fn write_path(
    host: &(impl FileSystemHost + PrivilegedFileSystemHost),
    path: &Path,
    bytes: &[u8],
    privileged: bool,
) -> Result<()> {
    if privileged {
        host.write_privileged(path, bytes).await
    } else {
        host.write_atomic(path, bytes)
            .await
            .map_err(|error| error.into_anyhow("writing managed Sys file"))
    }
}

async fn move_path(
    host: &(impl FileSystemHost + PrivilegedFileSystemHost),
    from: &Path,
    to: &Path,
    privileged: bool,
) -> Result<()> {
    if privileged {
        host.move_privileged(from, to).await
    } else {
        host.rename(from, to)
            .await
            .map_err(|error| error.into_anyhow("moving managed Sys file"))
    }
}

async fn remove_path(
    host: &(impl FileSystemHost + PrivilegedFileSystemHost),
    path: &Path,
    privileged: bool,
) -> Result<()> {
    if privileged {
        host.remove_privileged(path).await
    } else {
        match host.remove_file(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.is_not_found() => Ok(()),
            Err(error) => Err(error.into_anyhow("removing managed Sys file")),
        }
    }
}

async fn remove_exact(
    host: &(impl FileSystemHost + PrivilegedFileSystemHost),
    path: &Path,
    expected_hash: u64,
    privileged: bool,
) -> Result<()> {
    match file_hash(host, path).await? {
        None => Ok(()),
        Some(hash) if hash == expected_hash => remove_path(host, path, privileged).await,
        Some(_) => bail!("Sys rollback material changed before cleanup; journal preserved"),
    }
}

async fn add_action_observations(
    host: &(impl FileSystemObservationHost + SplitDnsObservationHost),
    state: &mut crate::plan::SnapshotDigestBuilderV1,
    kind: &ActionKindV1,
) -> Result<()> {
    let mut paths = Vec::<PathBuf>::new();
    match kind {
        ActionKindV1::CreateManagedFile { destination, .. } => paths.push(destination.clone()),
        ActionKindV1::CreateManagedFileWithBackup {
            destination,
            backup,
            ..
        } => paths.extend([destination.clone(), backup.clone()]),
        ActionKindV1::UpdateManagedFile {
            destination,
            rollback,
            ..
        }
        | ActionKindV1::RemoveManagedFile {
            destination,
            rollback,
            ..
        } => paths.extend([destination.clone(), rollback.clone()]),
        ActionKindV1::RemoveManagedFileWithBackup {
            destination,
            backup,
            rollback,
            ..
        } => paths.extend([destination.clone(), backup.clone(), rollback.clone()]),
        ActionKindV1::RelocateManagedFile {
            previous_destination,
            previous_backup,
            previous_rollback,
            desired_destination,
            ..
        } => {
            paths.extend([
                previous_destination.clone(),
                previous_rollback.clone(),
                desired_destination.clone(),
            ]);
            if let Some(backup) = previous_backup {
                paths.push(backup.path.clone());
            }
        }
        ActionKindV1::ReconcileSysSplitDns { previous, desired } => {
            for (index, dns) in previous.iter().chain(desired.iter()).enumerate() {
                let request = split_dns_request(dns);
                let current = host.inspect_split_dns(&request).await?;
                state.add_observation(
                    format!("split-dns:{index}"),
                    if current.exists {
                        format!(
                            "{}:{}",
                            request.resource.display(),
                            hash_content(&current.content)
                        )
                    } else {
                        format!("{}:missing", request.resource.display())
                    },
                )?;
            }
        }
        ActionKindV1::ReconcileSysProfileBlocks { files, .. } => {
            for file in files {
                paths.extend([file.destination.clone(), file.rollback.clone()]);
            }
        }
        _ => {}
    }
    for (index, path) in paths.iter().enumerate() {
        let observation = match observe_file(host, path).await? {
            ObservedFile::Missing => "missing".to_string(),
            ObservedFile::Regular(bytes, mode) => {
                format!("file:{}:{mode:?}", hash_content(&bytes))
            }
            ObservedFile::Other => "other".to_string(),
        };
        state.add_observation(format!("path:{index}"), observation)?;
    }
    Ok(())
}

async fn read_missing_marker(
    host: &impl FileSystemObservationHost,
    path: &Path,
) -> Result<Vec<u8>> {
    match host.read(path).await {
        Ok(bytes) => Ok(bytes),
        Err(error) if error.is_not_found() => Ok(b"missing".to_vec()),
        Err(error) => Err(error.into_anyhow("reading Sys recovery state")),
    }
}

async fn load_sys_operation_journal(
    host: &impl FileSystemObservationHost,
    shine_dir: &Path,
) -> Result<Option<(SysOperationJournalV1, Vec<u8>)>> {
    let path = shine_dir.join(SYS_OPERATION_JOURNAL_FILE);
    let bytes = match host.read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.is_not_found() => return Ok(None),
        Err(error) => return Err(error.into_anyhow("reading Sys operation journal")),
    };
    let journal: SysOperationJournalV1 =
        toml::from_slice(&bytes).context("failed to parse Sys operation journal")?;
    journal.validate()?;
    Ok(Some((journal, bytes)))
}

async fn save_sys_operation_journal(
    host: &impl FileSystemHost,
    shine_dir: &Path,
    journal: &SysOperationJournalV1,
) -> Result<()> {
    journal.validate()?;
    let bytes = toml::to_string_pretty(journal).context("serializing Sys operation journal")?;
    host.write_atomic(
        &shine_dir.join(SYS_OPERATION_JOURNAL_FILE),
        bytes.as_bytes(),
    )
    .await
    .map_err(|error| error.into_anyhow("writing Sys operation journal"))
}

async fn remove_sys_operation_journal(host: &impl FileSystemHost, shine_dir: &Path) -> Result<()> {
    match host
        .remove_file(&shine_dir.join(SYS_OPERATION_JOURNAL_FILE))
        .await
    {
        Ok(()) => Ok(()),
        Err(error) if error.is_not_found() => Ok(()),
        Err(error) => Err(error.into_anyhow("removing Sys operation journal")),
    }
}

fn review_path(context: &RuntimeContext, path: &Path) -> String {
    for (base, root) in [
        ("shine", &context.shine_dir),
        ("data-dir", &context.data_dir),
        ("home", &context.home_dir),
    ] {
        if let Ok(relative) = path.strip_prefix(root) {
            let value = if relative.as_os_str().is_empty() {
                ".".to_string()
            } else {
                logical_path(relative)
            };
            return format!("{base}:{value}");
        }
    }
    format!("absolute:{}", logical_path(path))
}

fn logical_path(path: &Path) -> String {
    path.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
