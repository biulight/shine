//! Receipt-bound operation journal for declarative managed Sys resources.
mod action;
use action::SysAction;
mod resources;
use resources::{
    apply_sys_action, assess_sys_action, cleanup_sys_action, preflight_sys_action,
    recover_sys_action,
};
mod profile;
use profile::{apply_sys_profile_blocks, assess_sys_profile_blocks, recover_sys_profile_blocks};
mod split_dns;
use split_dns::{
    apply_split_dns_transition, assess_split_dns, recover_split_dns_transition, split_dns_request,
};
mod file_ops;
use file_ops::{
    ObservedFile, assess_created_file, assess_removed_file, assess_staged_file, file_hash,
    mode_matches, move_path, observe_file, remove_exact, remove_path, write_path,
};
mod journal_io;
use journal_io::{
    load_sys_operation_journal, read_missing_marker, remove_sys_operation_journal,
    save_sys_operation_journal,
};
mod observations;
use observations::add_action_observations;

use super::review_path::review_path;
use super::sys::{load_manifest_with_host, save_manifest_with_host};
use super::{
    CoreRuntime, FileKind, FileSystemHost, FileSystemObservationHost, PrivilegedFileSystemHost,
    SplitDnsHost, SplitDnsObservationHost, SplitDnsRequest, SysRunEntry, SysRunManifest,
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
        if SysAction::from_kind(&action.kind).is_err() {
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
        self.plan_sys_operation_recovery_from_journal(journal, journal_bytes)
            .await
    }

    pub(crate) async fn inspect_sys_operation_journal(
        &self,
    ) -> Result<Option<super::JournalInspection>> {
        let Some((journal, journal_bytes)) =
            load_sys_operation_journal(self.host(), &self.context().shine_dir).await?
        else {
            return Ok(None);
        };
        let total = journal.action_ir.actions.len() as u64;
        let (prepared_actions, applied_actions, receipt_committed_actions) = match journal.state {
            SysJournalStateV1::Prepared => (total, 0, 0),
            SysJournalStateV1::Applied => (0, total, 0),
            SysJournalStateV1::ReceiptCommitted => (0, 0, total),
        };
        Ok(Some(super::JournalInspection {
            operation_id: journal.action_ir.operation_id.clone(),
            prepared_actions,
            applied_actions,
            receipt_committed_actions,
            recovery_plan: self
                .plan_sys_operation_recovery_from_journal(journal, journal_bytes)
                .await?,
        }))
    }

    async fn plan_sys_operation_recovery_from_journal(
        &self,
        journal: SysOperationJournalV1,
        journal_bytes: Vec<u8>,
    ) -> Result<PlanV1> {
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
                // Journal resource labels may contain legacy physical destinations. Keep those
                // local; the Plan already carries exact scoped filesystem permissions.
                Some(match &action.kind {
                    ActionKindV1::ReconcileSysSplitDns { .. } => "split-dns",
                    ActionKindV1::ReconcileSysProfileBlocks { .. } => "profile-blocks",
                    _ => "managed-file",
                }),
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
        let _lifecycle_guard = self.acquire_sys_lifecycle_operation().await?;
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
