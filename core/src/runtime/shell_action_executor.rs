//! Transactional mutation and explicit recovery for Shell lifecycle resources.

use super::launcher::{
    PreparedLauncherResource, apply_prepared_launcher_resource, prepare_launcher_resources,
};
use super::shell::{
    ShellManifest, ShellManifestEntry, load_shell_manifest_with_host,
    shell_link_spec_from_manifest_entry,
};
use super::{
    CoreRuntime, FileKind, FileSystemHost, FileSystemObservationHost, LinkSpec,
    PrivilegedFileSystemHost, RuntimeContext,
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

pub(crate) struct ShellSnapshotReplacement {
    pub target: String,
    pub destination: PathBuf,
    pub files: Vec<(PathBuf, Vec<u8>)>,
    pub receipt_transitions: Vec<(String, Option<ShellManifestEntry>, ShellManifestEntry)>,
}

pub(crate) struct ShellRenderedFileReplacement {
    pub target: String,
    pub destination: PathBuf,
    pub bytes: Vec<u8>,
    pub unix_mode: Option<u32>,
    pub receipt_transitions: Vec<(String, Option<ShellManifestEntry>, ShellManifestEntry)>,
}

pub(crate) struct ShellRenderedFileRemoval {
    pub target: String,
    pub destination: PathBuf,
    pub previous: ShellFileIdentityV1,
    pub previous_receipts: Vec<(String, ShellManifestEntry)>,
}

pub(crate) struct ShellCacheReplacementFile {
    pub destination: PathBuf,
    pub bytes: Vec<u8>,
    pub unix_mode: Option<u32>,
}

pub(crate) struct ShellCacheReplacement {
    pub target: String,
    pub files: Vec<ShellCacheReplacementFile>,
    pub receipt_transitions: Vec<(String, Option<ShellManifestEntry>, ShellManifestEntry)>,
}

pub(crate) struct ShellCacheRemoval {
    pub target: String,
    pub files: Vec<(PathBuf, ShellFileIdentityV1)>,
    pub previous_receipts: Vec<(String, ShellManifestEntry)>,
}

pub(crate) struct ShellSnapshotRemoval {
    pub target: String,
    pub destination: PathBuf,
    pub previous_files: Vec<ShellTreeFileV1>,
    pub previous_receipts: Vec<(String, ShellManifestEntry)>,
}

pub(crate) struct ShellProfileReconciliation {
    pub target: String,
    pub files: Vec<ShellProfilePreparedFile>,
    pub receipt_transitions: Vec<(String, Option<ShellManifestEntry>, ShellManifestEntry)>,
    pub receipt_removals: Vec<(String, ShellManifestEntry)>,
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
        files: Vec<(PathBuf, Vec<u8>)>,
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
    destination: PathBuf,
    rollback: PathBuf,
    desired: Option<Vec<u8>>,
    unix_mode: Option<u32>,
    ownership: ShellProfileFileOwnershipV1,
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
        if self.action_ir.actions.iter().any(|action| {
            !matches!(
                action.kind,
                ActionKindV1::CreateShellLauncher { .. }
                    | ActionKindV1::UpdateShellLauncher { .. }
                    | ActionKindV1::RemoveShellLauncher { .. }
                    | ActionKindV1::RemoveLegacyShellLauncher { .. }
                    | ActionKindV1::ReplaceShellSnapshot { .. }
                    | ActionKindV1::ReplaceShellCache { .. }
                    | ActionKindV1::RemoveShellCache { .. }
                    | ActionKindV1::RemoveShellSnapshot { .. }
                    | ActionKindV1::ReconcileShellProfile { .. }
                    | ActionKindV1::ReplaceShellRenderedFile { .. }
                    | ActionKindV1::RemoveShellRenderedFile { .. }
            )
        }) {
            bail!("Shell operation journal contains an unsupported action");
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
                            && matches!(
                                action.kind,
                                ActionKindV1::RemoveShellLauncher { .. }
                                    | ActionKindV1::RemoveLegacyShellLauncher { .. }
                                    | ActionKindV1::ReplaceShellSnapshot { .. }
                                    | ActionKindV1::ReplaceShellCache { .. }
                                    | ActionKindV1::RemoveShellCache { .. }
                                    | ActionKindV1::RemoveShellSnapshot { .. }
                                    | ActionKindV1::ReconcileShellProfile { .. }
                                    | ActionKindV1::ReplaceShellRenderedFile { .. }
                                    | ActionKindV1::RemoveShellRenderedFile { .. }
                            )
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

    async fn plan_shell_operation_recovery_from_journal(
        &self,
        journal: ShellOperationJournalV1,
        journal_bytes: Vec<u8>,
    ) -> Result<PlanV1> {
        let mut manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let mut shared_receipts_to_restore = BTreeSet::new();
        let mut projected_previous_receipt_targets = BTreeSet::new();
        let mut shared_receipt_conflicts = BTreeSet::new();
        for action in &journal.action_ir.actions {
            match &action.kind {
                ActionKindV1::RemoveShellRenderedFile { receipts, .. }
                | ActionKindV1::RemoveShellCache { receipts, .. }
                | ActionKindV1::RemoveShellSnapshot { receipts, .. } => {
                    if journal.receipt_committed.contains(&action.action_id) {
                        if !shell_removal_receipts_match_missing(&manifest, receipts) {
                            shared_receipt_conflicts.insert(action.action_id.clone());
                        }
                    } else if shell_removal_receipts_match_previous(&manifest, receipts) {
                    } else if shell_removal_receipts_match_missing(&manifest, receipts) {
                        restore_shell_removed_receipts(&mut manifest, receipts)?;
                        projected_previous_receipt_targets
                            .extend(receipts.iter().map(|receipt| receipt.target.clone()));
                        shared_receipts_to_restore.insert(action.action_id.clone());
                    } else {
                        shared_receipt_conflicts.insert(action.action_id.clone());
                    }
                }
                ActionKindV1::ReconcileShellProfile {
                    receipt_transitions,
                    receipt_removals,
                    legacy_targets,
                    ..
                } => {
                    if journal.receipt_committed.contains(&action.action_id) {
                        if !shell_profile_receipts_match_desired(
                            &manifest,
                            receipt_transitions,
                            receipt_removals,
                            legacy_targets,
                        ) {
                            shared_receipt_conflicts.insert(action.action_id.clone());
                        }
                    } else if shell_profile_receipts_match_previous(
                        &manifest,
                        receipt_transitions,
                        receipt_removals,
                        legacy_targets,
                    ) {
                    } else if shell_profile_receipts_match_desired(
                        &manifest,
                        receipt_transitions,
                        receipt_removals,
                        legacy_targets,
                    ) {
                        restore_shell_previous_receipts(&mut manifest, receipt_transitions)?;
                        restore_shell_removed_receipts(&mut manifest, receipt_removals)?;
                        projected_previous_receipt_targets.extend(
                            receipt_transitions
                                .iter()
                                .map(|receipt| receipt.target.clone())
                                .chain(
                                    receipt_removals
                                        .iter()
                                        .map(|receipt| receipt.target.clone()),
                                ),
                        );
                        shared_receipts_to_restore.insert(action.action_id.clone());
                    } else {
                        shared_receipt_conflicts.insert(action.action_id.clone());
                    }
                }
                _ => {
                    let Some(receipts) = shell_shared_receipt_transitions(&action.kind) else {
                        continue;
                    };
                    if journal.receipt_committed.contains(&action.action_id) {
                        if !shell_receipts_match_desired(&manifest, receipts) {
                            shared_receipt_conflicts.insert(action.action_id.clone());
                        }
                    } else if shell_receipts_match_previous(&manifest, receipts) {
                    } else if shell_receipts_match_desired(&manifest, receipts) {
                        restore_shell_previous_receipts(&mut manifest, receipts)?;
                        projected_previous_receipt_targets
                            .extend(receipts.iter().map(|receipt| receipt.target.clone()));
                        shared_receipts_to_restore.insert(action.action_id.clone());
                    } else {
                        shared_receipt_conflicts.insert(action.action_id.clone());
                    }
                }
            }
        }
        let manifest_bytes = match self
            .host()
            .read(&self.context().shine_dir.join("shell-manifest.toml"))
            .await
        {
            Ok(bytes) => bytes,
            Err(error) if error.is_not_found() => b"missing".to_vec(),
            Err(error) => return Err(error.into_anyhow("reading Shell recovery manifest")),
        };
        let mut state = SnapshotDigestV1::builder("state:shell-recovery");
        state.add_observation("operation", PlanOperationV1::ShellRecovery.as_str())?;
        state.add_observation("journal", &journal_bytes)?;
        state.add_observation("shell-manifest", &manifest_bytes)?;
        let mut required = PermissionSetV1::new([PermissionV1::Filesystem {
            access: FilesystemAccessV1::Remove,
            path: review_path(
                self.context(),
                &self.context().shine_dir.join(SHELL_OPERATION_JOURNAL_FILE),
            ),
        }]);
        let mut steps = Vec::new();
        let mut blocked = false;
        for action in &journal.action_ir.actions {
            let (plan_action, code, action_blocked) = match &action.kind {
                ActionKindV1::CreateShellLauncher { receipt, resources } => {
                    let receipt_state = matching_shell_receipt(&manifest, &action.target, receipt);
                    if receipt_state == ReceiptState::Conflict {
                        (
                            PlanActionV1::Blocked,
                            "shell_recovery_receipt_conflict",
                            true,
                        )
                    } else {
                        let mut action_changed = false;
                        let mut action_blocked = false;
                        for (index, resource) in resources.iter().enumerate() {
                            let observation =
                                observe_launcher_resource(self.host(), resource).await?;
                            state.add_observation(
                                format!("launcher:{}:{index}", action.action_id),
                                observation.identity(),
                            )?;
                            if receipt_state == ReceiptState::Matching {
                                continue;
                            }
                            match observation {
                                LauncherObservation::Missing => {}
                                LauncherObservation::Exact => {
                                    action_changed = true;
                                    required.insert(PermissionV1::Filesystem {
                                        access: FilesystemAccessV1::Remove,
                                        path: review_path(self.context(), resource.destination()),
                                    });
                                }
                                LauncherObservation::Changed => action_blocked = true,
                            }
                        }
                        if action_blocked {
                            (
                                PlanActionV1::Blocked,
                                "shell_recovery_launcher_changed",
                                true,
                            )
                        } else if receipt_state == ReceiptState::Matching {
                            (
                                PlanActionV1::None,
                                "shell_recovery_receipt_already_committed",
                                false,
                            )
                        } else if action_changed {
                            (
                                PlanActionV1::Remove,
                                "shell_recovery_remove_created_launcher",
                                false,
                            )
                        } else {
                            (PlanActionV1::None, "shell_recovery_launcher_absent", false)
                        }
                    }
                }
                ActionKindV1::UpdateShellLauncher {
                    previous_receipt,
                    desired_receipt,
                    resources,
                } => {
                    let previous_state =
                        matching_shell_receipt(&manifest, &action.target, previous_receipt);
                    let desired_state =
                        matching_shell_receipt(&manifest, &action.target, desired_receipt);
                    let committed = desired_state == ReceiptState::Matching;
                    if !committed && previous_state != ReceiptState::Matching {
                        (
                            PlanActionV1::Blocked,
                            "shell_recovery_receipt_conflict",
                            true,
                        )
                    } else {
                        let mut action_changed = false;
                        let mut action_blocked = false;
                        for (index, resource) in resources.iter().enumerate() {
                            let observation =
                                observe_launcher_update_resource(self.host(), resource, committed)
                                    .await?;
                            state.add_observation(
                                format!("launcher-update:{}:{index}", action.action_id),
                                observation.identity(),
                            )?;
                            action_changed |= observation.needs_recovery();
                            action_blocked |= observation == LauncherUpdateObservation::Changed;
                            for (access, path) in observation.required_permissions(resource) {
                                required.insert(PermissionV1::Filesystem {
                                    access,
                                    path: review_path(self.context(), path),
                                });
                            }
                        }
                        if action_blocked {
                            (
                                PlanActionV1::Blocked,
                                "shell_recovery_launcher_update_changed",
                                true,
                            )
                        } else if action_changed {
                            (
                                if committed {
                                    PlanActionV1::Remove
                                } else {
                                    PlanActionV1::Update
                                },
                                if committed {
                                    "shell_recovery_cleanup_launcher_rollback"
                                } else {
                                    "shell_recovery_restore_previous_launcher"
                                },
                                false,
                            )
                        } else {
                            (
                                PlanActionV1::None,
                                if committed {
                                    "shell_recovery_launcher_update_committed"
                                } else {
                                    "shell_recovery_launcher_update_not_started"
                                },
                                false,
                            )
                        }
                    }
                }
                ActionKindV1::RemoveShellLauncher {
                    previous_receipt,
                    resources,
                } => {
                    let receipt_state =
                        matching_shell_receipt(&manifest, &action.target, previous_receipt);
                    let committed = journal.receipt_committed.contains(&action.action_id);
                    if receipt_state == ReceiptState::Conflict
                        || (committed && receipt_state != ReceiptState::Missing)
                    {
                        (
                            PlanActionV1::Blocked,
                            "shell_recovery_receipt_conflict",
                            true,
                        )
                    } else {
                        let restore_receipt = !committed
                            && (receipt_state == ReceiptState::Missing
                                || projected_previous_receipt_targets.contains(&action.target));
                        let mut action_changed = restore_receipt;
                        let mut action_blocked = false;
                        if restore_receipt {
                            required.insert(PermissionV1::Filesystem {
                                access: FilesystemAccessV1::Write,
                                path: review_path(
                                    self.context(),
                                    &self.context().shine_dir.join("shell-manifest.toml"),
                                ),
                            });
                        }
                        for (index, resource) in resources.iter().enumerate() {
                            let observation =
                                observe_launcher_removal_resource(self.host(), resource, committed)
                                    .await?;
                            state.add_observation(
                                format!("launcher-remove:{}:{index}", action.action_id),
                                observation.identity(),
                            )?;
                            action_changed |= observation.needs_recovery();
                            action_blocked |= observation == LauncherRemovalObservation::Changed;
                            for (access, path) in observation.required_permissions(resource) {
                                required.insert(PermissionV1::Filesystem {
                                    access,
                                    path: review_path(self.context(), path),
                                });
                            }
                        }
                        if action_blocked {
                            (
                                PlanActionV1::Blocked,
                                "shell_recovery_launcher_removal_changed",
                                true,
                            )
                        } else if action_changed {
                            (
                                if committed {
                                    PlanActionV1::Remove
                                } else {
                                    PlanActionV1::Update
                                },
                                if committed {
                                    "shell_recovery_cleanup_removed_launcher"
                                } else if restore_receipt {
                                    "shell_recovery_restore_removed_launcher_receipt"
                                } else {
                                    "shell_recovery_restore_removed_launcher"
                                },
                                false,
                            )
                        } else {
                            (
                                PlanActionV1::None,
                                if committed {
                                    "shell_recovery_launcher_removal_committed"
                                } else {
                                    "shell_recovery_launcher_removal_not_started"
                                },
                                false,
                            )
                        }
                    }
                }
                ActionKindV1::RemoveLegacyShellLauncher { resources } => {
                    let committed = journal.receipt_committed.contains(&action.action_id);
                    let mut action_changed = false;
                    let mut action_blocked = false;
                    for (index, resource) in resources.iter().enumerate() {
                        let observation =
                            observe_launcher_removal_resource(self.host(), resource, committed)
                                .await?;
                        state.add_observation(
                            format!("legacy-launcher-remove:{}:{index}", action.action_id),
                            observation.identity(),
                        )?;
                        action_changed |= observation.needs_recovery();
                        action_blocked |= observation == LauncherRemovalObservation::Changed;
                        for (access, path) in observation.required_permissions(resource) {
                            required.insert(PermissionV1::Filesystem {
                                access,
                                path: review_path(self.context(), path),
                            });
                        }
                    }
                    if action_blocked {
                        (
                            PlanActionV1::Blocked,
                            "shell_recovery_legacy_launcher_removal_changed",
                            true,
                        )
                    } else if action_changed {
                        (
                            if committed {
                                PlanActionV1::Remove
                            } else {
                                PlanActionV1::Update
                            },
                            if committed {
                                "shell_recovery_cleanup_removed_legacy_launcher"
                            } else {
                                "shell_recovery_restore_removed_legacy_launcher"
                            },
                            false,
                        )
                    } else {
                        (
                            PlanActionV1::None,
                            if committed {
                                "shell_recovery_legacy_launcher_removal_committed"
                            } else {
                                "shell_recovery_legacy_launcher_removal_not_started"
                            },
                            false,
                        )
                    }
                }
                ActionKindV1::ReplaceShellSnapshot {
                    destination,
                    stage,
                    rollback,
                    previous_present,
                    previous_files,
                    desired_files,
                    receipts: _,
                } => {
                    let committed = journal.receipt_committed.contains(&action.action_id);
                    let receipt_conflict = shared_receipt_conflicts.contains(&action.action_id);
                    let restore_receipts = shared_receipts_to_restore.contains(&action.action_id);
                    for (label, path) in [
                        ("destination", destination),
                        ("stage", stage),
                        ("rollback", rollback),
                    ] {
                        let observation = collect_shell_tree(self.host(), path).await?;
                        state.add_observation(
                            format!("snapshot:{}:{label}", action.action_id),
                            serde_json::to_vec(&observation)
                                .context("serializing Shell snapshot recovery observation")?,
                        )?;
                    }
                    let assessment = if receipt_conflict {
                        ShellSnapshotRecoveryAssessment::Blocked
                    } else {
                        assess_shell_snapshot_recovery(
                            self.host(),
                            destination,
                            stage,
                            rollback,
                            *previous_present,
                            previous_files,
                            desired_files,
                            committed,
                        )
                        .await?
                    };
                    if restore_receipts {
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Write,
                            path: review_path(
                                self.context(),
                                &self.context().shine_dir.join("shell-manifest.toml"),
                            ),
                        });
                    }
                    for (access, path) in
                        snapshot_recovery_permissions(assessment, destination, stage, rollback)
                    {
                        required.insert(PermissionV1::Filesystem {
                            access,
                            path: review_path(self.context(), path),
                        });
                    }
                    match assessment {
                        ShellSnapshotRecoveryAssessment::Blocked => (
                            PlanActionV1::Blocked,
                            "shell_recovery_snapshot_changed",
                            true,
                        ),
                        ShellSnapshotRecoveryAssessment::Stable => (
                            if restore_receipts {
                                PlanActionV1::Update
                            } else {
                                PlanActionV1::None
                            },
                            if committed {
                                "shell_recovery_snapshot_committed"
                            } else if restore_receipts {
                                "shell_recovery_restore_snapshot_receipts"
                            } else {
                                "shell_recovery_snapshot_not_started"
                            },
                            false,
                        ),
                        ShellSnapshotRecoveryAssessment::CleanupRollback => (
                            PlanActionV1::Remove,
                            "shell_recovery_cleanup_snapshot_rollback",
                            false,
                        ),
                        ShellSnapshotRecoveryAssessment::RemoveStage
                        | ShellSnapshotRecoveryAssessment::RestoreMoved
                        | ShellSnapshotRecoveryAssessment::RestoreReplaced
                        | ShellSnapshotRecoveryAssessment::RemoveCreated => (
                            PlanActionV1::Update,
                            "shell_recovery_restore_previous_snapshot",
                            false,
                        ),
                    }
                }
                ActionKindV1::ReplaceShellCache { files, receipts: _ } => {
                    let committed = journal.receipt_committed.contains(&action.action_id);
                    let receipt_conflict = shared_receipt_conflicts.contains(&action.action_id);
                    let restore_receipts = shared_receipts_to_restore.contains(&action.action_id);
                    let mut any_blocked = receipt_conflict;
                    let mut any_cleanup = false;
                    let mut any_restore = false;
                    for (index, file) in files.iter().enumerate() {
                        for (label, path) in [
                            ("destination", &file.destination),
                            ("rollback", &file.rollback),
                        ] {
                            let observation = observe_shell_file(self.host(), path).await?;
                            state.add_observation(
                                format!("cache:{}:{index}:{label}", action.action_id),
                                serde_json::to_vec(&observation)
                                    .context("serializing Shell cache recovery observation")?,
                            )?;
                        }
                        let assessment = if receipt_conflict {
                            ShellRenderedFileRecoveryAssessment::Blocked
                        } else {
                            assess_shell_rendered_file_recovery(
                                self.host(),
                                &file.destination,
                                &file.rollback,
                                file.previous.as_ref(),
                                &file.desired,
                                committed,
                            )
                            .await?
                        };
                        any_blocked |= assessment == ShellRenderedFileRecoveryAssessment::Blocked;
                        any_cleanup |=
                            assessment == ShellRenderedFileRecoveryAssessment::CleanupRollback;
                        any_restore |= matches!(
                            assessment,
                            ShellRenderedFileRecoveryAssessment::RestoreMoved
                                | ShellRenderedFileRecoveryAssessment::RestoreReplaced
                                | ShellRenderedFileRecoveryAssessment::RemoveCreated
                        );
                        for (access, path) in rendered_file_recovery_permissions(
                            assessment,
                            &file.destination,
                            &file.rollback,
                        ) {
                            required.insert(PermissionV1::Filesystem {
                                access,
                                path: review_path(self.context(), path),
                            });
                        }
                    }
                    if restore_receipts {
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Write,
                            path: review_path(
                                self.context(),
                                &self.context().shine_dir.join("shell-manifest.toml"),
                            ),
                        });
                    }
                    if any_blocked {
                        (PlanActionV1::Blocked, "shell_recovery_cache_changed", true)
                    } else if any_restore {
                        (
                            PlanActionV1::Update,
                            "shell_recovery_restore_previous_cache",
                            false,
                        )
                    } else if any_cleanup {
                        (
                            PlanActionV1::Remove,
                            "shell_recovery_cleanup_cache_rollback",
                            false,
                        )
                    } else if restore_receipts {
                        (
                            PlanActionV1::Update,
                            "shell_recovery_restore_cache_receipts",
                            false,
                        )
                    } else {
                        (
                            PlanActionV1::None,
                            if committed {
                                "shell_recovery_cache_committed"
                            } else {
                                "shell_recovery_cache_not_started"
                            },
                            false,
                        )
                    }
                }
                ActionKindV1::RemoveShellCache { files, receipts: _ } => {
                    let committed = journal.receipt_committed.contains(&action.action_id);
                    let receipt_conflict = shared_receipt_conflicts.contains(&action.action_id);
                    let restore_receipts = shared_receipts_to_restore.contains(&action.action_id);
                    let mut any_blocked = receipt_conflict;
                    let mut any_restore = false;
                    let mut any_cleanup = false;
                    for (index, file) in files.iter().enumerate() {
                        for (label, path) in [
                            ("destination", &file.destination),
                            ("rollback", &file.rollback),
                        ] {
                            let observation = observe_shell_file(self.host(), path).await?;
                            state.add_observation(
                                format!("cache-remove:{}:{index}:{label}", action.action_id),
                                serde_json::to_vec(&observation).context(
                                    "serializing Shell cache removal recovery observation",
                                )?,
                            )?;
                        }
                        let assessment = if receipt_conflict {
                            ShellRenderedFileRemovalRecoveryAssessment::Blocked
                        } else {
                            assess_shell_rendered_file_removal_recovery(
                                self.host(),
                                &file.destination,
                                &file.rollback,
                                &file.previous,
                                committed,
                            )
                            .await?
                        };
                        any_blocked |=
                            assessment == ShellRenderedFileRemovalRecoveryAssessment::Blocked;
                        any_restore |=
                            assessment == ShellRenderedFileRemovalRecoveryAssessment::RestoreMoved;
                        any_cleanup |= assessment
                            == ShellRenderedFileRemovalRecoveryAssessment::CleanupRollback;
                        for (access, path) in rendered_file_removal_recovery_permissions(
                            assessment,
                            &file.destination,
                            &file.rollback,
                        ) {
                            required.insert(PermissionV1::Filesystem {
                                access,
                                path: review_path(self.context(), path),
                            });
                        }
                    }
                    if restore_receipts {
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Write,
                            path: review_path(
                                self.context(),
                                &self.context().shine_dir.join("shell-manifest.toml"),
                            ),
                        });
                    }
                    if any_blocked {
                        (
                            PlanActionV1::Blocked,
                            "shell_recovery_cache_removal_changed",
                            true,
                        )
                    } else if any_restore {
                        (
                            PlanActionV1::Update,
                            "shell_recovery_restore_removed_cache",
                            false,
                        )
                    } else if any_cleanup {
                        (
                            PlanActionV1::Remove,
                            "shell_recovery_cleanup_removed_cache",
                            false,
                        )
                    } else if restore_receipts {
                        (
                            PlanActionV1::Update,
                            "shell_recovery_restore_cache_receipts",
                            false,
                        )
                    } else {
                        (
                            PlanActionV1::None,
                            if committed {
                                "shell_recovery_cache_removal_committed"
                            } else {
                                "shell_recovery_cache_removal_not_started"
                            },
                            false,
                        )
                    }
                }
                ActionKindV1::RemoveShellSnapshot {
                    destination,
                    rollback,
                    previous_files,
                    receipts: _,
                } => {
                    let committed = journal.receipt_committed.contains(&action.action_id);
                    let receipt_conflict = shared_receipt_conflicts.contains(&action.action_id);
                    let restore_receipts = shared_receipts_to_restore.contains(&action.action_id);
                    for (label, path) in [("destination", destination), ("rollback", rollback)] {
                        let observation = collect_shell_tree(self.host(), path).await?;
                        state.add_observation(
                            format!("snapshot-remove:{}:{label}", action.action_id),
                            serde_json::to_vec(&observation).context(
                                "serializing Shell snapshot removal recovery observation",
                            )?,
                        )?;
                    }
                    let assessment = if receipt_conflict {
                        ShellTreeRemovalRecoveryAssessment::Blocked
                    } else {
                        assess_shell_tree_removal_recovery(
                            self.host(),
                            destination,
                            rollback,
                            previous_files,
                            committed,
                        )
                        .await?
                    };
                    if restore_receipts {
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Write,
                            path: review_path(
                                self.context(),
                                &self.context().shine_dir.join("shell-manifest.toml"),
                            ),
                        });
                    }
                    for (access, path) in
                        shell_tree_removal_recovery_permissions(assessment, destination, rollback)
                    {
                        required.insert(PermissionV1::Filesystem {
                            access,
                            path: review_path(self.context(), path),
                        });
                    }
                    match assessment {
                        ShellTreeRemovalRecoveryAssessment::Blocked => (
                            PlanActionV1::Blocked,
                            "shell_recovery_snapshot_removal_changed",
                            true,
                        ),
                        ShellTreeRemovalRecoveryAssessment::RestoreMoved => (
                            PlanActionV1::Update,
                            "shell_recovery_restore_removed_snapshot",
                            false,
                        ),
                        ShellTreeRemovalRecoveryAssessment::CleanupRollback => (
                            PlanActionV1::Remove,
                            "shell_recovery_cleanup_removed_snapshot",
                            false,
                        ),
                        ShellTreeRemovalRecoveryAssessment::Stable => (
                            if restore_receipts {
                                PlanActionV1::Update
                            } else {
                                PlanActionV1::None
                            },
                            if committed {
                                "shell_recovery_snapshot_removal_committed"
                            } else if restore_receipts {
                                "shell_recovery_restore_snapshot_receipts"
                            } else {
                                "shell_recovery_snapshot_removal_not_started"
                            },
                            false,
                        ),
                    }
                }
                ActionKindV1::ReconcileShellProfile { files, .. } => {
                    let committed = journal.receipt_committed.contains(&action.action_id);
                    let receipt_conflict = shared_receipt_conflicts.contains(&action.action_id);
                    let restore_receipts = shared_receipts_to_restore.contains(&action.action_id);
                    let mut any_blocked = receipt_conflict;
                    let mut any_restore = false;
                    let mut any_cleanup = false;
                    for (index, file) in files.iter().enumerate() {
                        for (label, path) in [
                            ("destination", &file.destination),
                            ("rollback", &file.rollback),
                        ] {
                            let observation = observe_shell_file(self.host(), path).await?;
                            state.add_observation(
                                format!("profile:{}:{index}:{label}", action.action_id),
                                serde_json::to_vec(&observation)
                                    .context("serializing Shell profile recovery observation")?,
                            )?;
                        }
                        let assessment = if receipt_conflict {
                            ShellProfileFileRecoveryAssessment::Blocked
                        } else {
                            assess_shell_profile_file_recovery(self.host(), file, committed).await?
                        };
                        any_blocked |= assessment == ShellProfileFileRecoveryAssessment::Blocked;
                        any_restore |= matches!(
                            assessment,
                            ShellProfileFileRecoveryAssessment::RestoreWholeMoved
                                | ShellProfileFileRecoveryAssessment::RestoreWholeReplaced
                                | ShellProfileFileRecoveryAssessment::RemoveWholeCreated
                                | ShellProfileFileRecoveryAssessment::RestoreSentinel
                        );
                        any_cleanup |=
                            assessment == ShellProfileFileRecoveryAssessment::CleanupRollback;
                        for (access, path) in shell_profile_recovery_permissions(assessment, file) {
                            required.insert(PermissionV1::Filesystem {
                                access,
                                path: review_path(self.context(), path),
                            });
                        }
                    }
                    if restore_receipts {
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Write,
                            path: review_path(
                                self.context(),
                                &self.context().shine_dir.join("shell-manifest.toml"),
                            ),
                        });
                    }
                    if any_blocked {
                        (
                            PlanActionV1::Blocked,
                            "shell_recovery_profile_changed",
                            true,
                        )
                    } else if any_restore {
                        (
                            PlanActionV1::Update,
                            "shell_recovery_restore_profile",
                            false,
                        )
                    } else if any_cleanup {
                        (
                            PlanActionV1::Remove,
                            "shell_recovery_cleanup_profile_rollback",
                            false,
                        )
                    } else if restore_receipts {
                        (
                            PlanActionV1::Update,
                            "shell_recovery_restore_profile_receipts",
                            false,
                        )
                    } else {
                        (
                            PlanActionV1::None,
                            if committed {
                                "shell_recovery_profile_committed"
                            } else {
                                "shell_recovery_profile_not_started"
                            },
                            false,
                        )
                    }
                }
                ActionKindV1::ReplaceShellRenderedFile {
                    destination,
                    rollback,
                    previous,
                    desired,
                    receipts: _,
                } => {
                    let committed = journal.receipt_committed.contains(&action.action_id);
                    let receipt_conflict = shared_receipt_conflicts.contains(&action.action_id);
                    let restore_receipts = shared_receipts_to_restore.contains(&action.action_id);
                    for (label, path) in [("destination", destination), ("rollback", rollback)] {
                        let observation = observe_shell_file(self.host(), path).await?;
                        state.add_observation(
                            format!("rendered:{}:{label}", action.action_id),
                            serde_json::to_vec(&observation)
                                .context("serializing Shell rendered-file recovery observation")?,
                        )?;
                    }
                    let assessment = if receipt_conflict {
                        ShellRenderedFileRecoveryAssessment::Blocked
                    } else {
                        assess_shell_rendered_file_recovery(
                            self.host(),
                            destination,
                            rollback,
                            previous.as_ref(),
                            desired,
                            committed,
                        )
                        .await?
                    };
                    if restore_receipts {
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Write,
                            path: review_path(
                                self.context(),
                                &self.context().shine_dir.join("shell-manifest.toml"),
                            ),
                        });
                    }
                    for (access, path) in
                        rendered_file_recovery_permissions(assessment, destination, rollback)
                    {
                        required.insert(PermissionV1::Filesystem {
                            access,
                            path: review_path(self.context(), path),
                        });
                    }
                    match assessment {
                        ShellRenderedFileRecoveryAssessment::Blocked => (
                            PlanActionV1::Blocked,
                            "shell_recovery_rendered_file_changed",
                            true,
                        ),
                        ShellRenderedFileRecoveryAssessment::Stable => (
                            if restore_receipts {
                                PlanActionV1::Update
                            } else {
                                PlanActionV1::None
                            },
                            if committed {
                                "shell_recovery_rendered_file_committed"
                            } else if restore_receipts {
                                "shell_recovery_restore_rendered_receipts"
                            } else {
                                "shell_recovery_rendered_file_not_started"
                            },
                            false,
                        ),
                        ShellRenderedFileRecoveryAssessment::CleanupRollback => (
                            PlanActionV1::Remove,
                            "shell_recovery_cleanup_rendered_rollback",
                            false,
                        ),
                        ShellRenderedFileRecoveryAssessment::RestoreMoved
                        | ShellRenderedFileRecoveryAssessment::RestoreReplaced
                        | ShellRenderedFileRecoveryAssessment::RemoveCreated => (
                            PlanActionV1::Update,
                            "shell_recovery_restore_previous_rendered_file",
                            false,
                        ),
                    }
                }
                ActionKindV1::RemoveShellRenderedFile {
                    destination,
                    rollback,
                    previous,
                    receipts: _,
                } => {
                    let committed = journal.receipt_committed.contains(&action.action_id);
                    let receipt_conflict = shared_receipt_conflicts.contains(&action.action_id);
                    let restore_receipts = shared_receipts_to_restore.contains(&action.action_id);
                    for (label, path) in [("destination", destination), ("rollback", rollback)] {
                        let observation = observe_shell_file(self.host(), path).await?;
                        state.add_observation(
                            format!("rendered-remove:{}:{label}", action.action_id),
                            serde_json::to_vec(&observation).context(
                                "serializing Shell rendered-file removal recovery observation",
                            )?,
                        )?;
                    }
                    let assessment = if receipt_conflict {
                        ShellRenderedFileRemovalRecoveryAssessment::Blocked
                    } else {
                        assess_shell_rendered_file_removal_recovery(
                            self.host(),
                            destination,
                            rollback,
                            previous,
                            committed,
                        )
                        .await?
                    };
                    if restore_receipts {
                        required.insert(PermissionV1::Filesystem {
                            access: FilesystemAccessV1::Write,
                            path: review_path(
                                self.context(),
                                &self.context().shine_dir.join("shell-manifest.toml"),
                            ),
                        });
                    }
                    for (access, path) in rendered_file_removal_recovery_permissions(
                        assessment,
                        destination,
                        rollback,
                    ) {
                        required.insert(PermissionV1::Filesystem {
                            access,
                            path: review_path(self.context(), path),
                        });
                    }
                    match assessment {
                        ShellRenderedFileRemovalRecoveryAssessment::Blocked => (
                            PlanActionV1::Blocked,
                            "shell_recovery_rendered_file_removal_changed",
                            true,
                        ),
                        ShellRenderedFileRemovalRecoveryAssessment::Stable => (
                            if restore_receipts {
                                PlanActionV1::Update
                            } else {
                                PlanActionV1::None
                            },
                            if committed {
                                "shell_recovery_rendered_file_removal_committed"
                            } else if restore_receipts {
                                "shell_recovery_restore_removed_rendered_receipts"
                            } else {
                                "shell_recovery_rendered_file_removal_not_started"
                            },
                            false,
                        ),
                        ShellRenderedFileRemovalRecoveryAssessment::RestoreMoved => (
                            PlanActionV1::Update,
                            "shell_recovery_restore_removed_rendered_file",
                            false,
                        ),
                        ShellRenderedFileRemovalRecoveryAssessment::CleanupRollback => (
                            PlanActionV1::Remove,
                            "shell_recovery_cleanup_removed_rendered_rollback",
                            false,
                        ),
                    }
                }
                _ => unreachable!("validated Shell journal action kind"),
            };
            blocked |= action_blocked;
            steps.push(
                PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                    .with_diagnostic_code(code),
            );
        }
        steps.push(
            PlanStepV1::new(
                "shell",
                Some("operation-journal"),
                if blocked {
                    PlanActionV1::Preserve
                } else {
                    PlanActionV1::Remove
                },
            )
            .with_diagnostic_code(if blocked {
                "shell_recovery_journal_preserved"
            } else {
                "shell_recovery_clear_journal"
            }),
        );
        let preset = self.presets().digest_v1()?;
        Ok(PlanV1::new(
            PlanOperationV1::ShellRecovery,
            PlanInputsV1 {
                preset,
                state: state.finish(),
            },
            steps,
            required.clone(),
            &required,
            std::iter::empty::<String>(),
        ))
    }
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    pub(crate) async fn reconcile_shell_launchers_approved(
        &self,
        shared: ShellSharedReplacements<'_>,
        creations: &[ShellLauncherCreation<'_>],
        updates: &[ShellLauncherUpdate<'_>],
        removals: &[ShellLauncherRemoval],
        legacy_removals: &[ShellLegacyLauncherRemoval],
        approval: &PlanApprovalV1,
    ) -> Result<Option<ShellOperationExecutionV1>> {
        if shared.caches.is_empty()
            && shared.snapshots.is_empty()
            && shared.rendered_files.is_empty()
            && shared.rendered_removals.is_empty()
            && shared.cache_removals.is_empty()
            && shared.snapshot_removals.is_empty()
            && shared.profiles.is_empty()
            && creations.is_empty()
            && updates.is_empty()
            && removals.is_empty()
            && legacy_removals.is_empty()
        {
            return Ok(None);
        }
        let operation_guard = self.host().acquire_privileged_operation().await?;
        if load_shell_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .is_some()
        {
            bail!("an interrupted Shell operation requires explicit recovery");
        }
        let mut prepared = Vec::new();
        let mut actions = Vec::new();
        for cache in shared.caches {
            let mut action_files = Vec::new();
            let mut prepared_files = Vec::new();
            for file in &cache.files {
                let rollback = managed_file_rollback_path(&file.destination);
                match self.host().metadata(&rollback).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Shell cache rollback path is occupied: {}",
                        rollback.display()
                    ),
                    Err(error) => {
                        return Err(error.into_anyhow("revalidating Shell cache rollback path"));
                    }
                }
                let previous = observe_shell_file(self.host(), &file.destination).await?;
                let previous_identity = match previous {
                    ShellFileObservation::Missing => None,
                    ShellFileObservation::Regular(identity) => Some(identity),
                    ShellFileObservation::Other => {
                        bail!(
                            "Shell cache destination is not a regular file: {}",
                            file.destination.display()
                        )
                    }
                };
                let desired = ShellFileIdentityV1 {
                    content_hash: hash_content(&file.bytes),
                    unix_mode: file.unix_mode,
                };
                if previous_identity.as_ref() == Some(&desired) {
                    bail!("Shell cache replacement contains no changed file");
                }
                action_files.push(ShellCacheFileReplacementV1 {
                    destination: file.destination.clone(),
                    rollback: rollback.clone(),
                    previous: previous_identity.clone(),
                    desired,
                });
                prepared_files.push(PreparedShellCacheFile {
                    destination: file.destination.clone(),
                    rollback,
                    previous_present: previous_identity.is_some(),
                    bytes: file.bytes.clone(),
                    unix_mode: file.unix_mode,
                });
            }
            let receipts = cache
                .receipt_transitions
                .iter()
                .map(|(target, previous, desired)| ShellReceiptTransitionV1 {
                    target: target.clone(),
                    previous: previous.as_ref().map(receipt_contract).map(Box::new),
                    desired: Box::new(receipt_contract(desired)),
                })
                .collect();
            actions.push(DeclarativeActionV1::replace_shell_cache(
                format!("replace-cache:{}", cache.target),
                &cache.target,
                "preset-cache",
                ShellCacheReplacementSpecV1 {
                    files: action_files,
                    receipts,
                },
            ));
            prepared.push(PreparedShellAction::Cache {
                files: prepared_files,
            });
        }
        for snapshot in shared.snapshots {
            let stage = shell_snapshot_stage_path(&snapshot.destination);
            let rollback = shell_snapshot_rollback_path(&snapshot.destination);
            for (path, label) in [(&stage, "stage"), (&rollback, "rollback")] {
                match self.host().metadata(path).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Shell snapshot {label} path is occupied: {}",
                        path.display()
                    ),
                    Err(error) => {
                        return Err(
                            error.into_anyhow("revalidating Shell snapshot transaction path")
                        );
                    }
                }
            }
            let previous = collect_shell_tree(self.host(), &snapshot.destination).await?;
            let desired_files = shell_tree_contract(&snapshot.files);
            let receipts = snapshot
                .receipt_transitions
                .iter()
                .map(|(target, previous, desired)| ShellReceiptTransitionV1 {
                    target: target.clone(),
                    previous: previous.as_ref().map(receipt_contract).map(Box::new),
                    desired: Box::new(receipt_contract(desired)),
                })
                .collect();
            actions.push(DeclarativeActionV1::replace_shell_snapshot(
                format!("replace-snapshot:{}", snapshot.target),
                &snapshot.target,
                "shared-snapshot",
                ShellSnapshotReplacementSpecV1 {
                    destination: snapshot.destination.clone(),
                    previous_present: previous.is_some(),
                    previous_files: previous.unwrap_or_default(),
                    desired_files,
                    receipts,
                },
            ));
            prepared.push(PreparedShellAction::Snapshot {
                destination: snapshot.destination.clone(),
                stage,
                rollback,
                previous_present: self.host().metadata(&snapshot.destination).await.is_ok(),
                files: snapshot.files.clone(),
            });
        }
        for rendered in shared.rendered_files {
            let rollback = managed_file_rollback_path(&rendered.destination);
            match self.host().metadata(&rollback).await {
                Err(error) if error.is_not_found() => {}
                Ok(_) => bail!(
                    "Shell rendered-file rollback path is occupied: {}",
                    rollback.display()
                ),
                Err(error) => {
                    return Err(error.into_anyhow("revalidating Shell rendered-file rollback path"));
                }
            }
            let previous = observe_shell_file(self.host(), &rendered.destination).await?;
            let previous_identity = match previous {
                ShellFileObservation::Missing => None,
                ShellFileObservation::Regular(identity) => Some(identity),
                ShellFileObservation::Other => {
                    bail!(
                        "Shell rendered-file destination is not a regular file: {}",
                        rendered.destination.display()
                    )
                }
            };
            let desired = ShellFileIdentityV1 {
                content_hash: hash_content(&rendered.bytes),
                unix_mode: rendered.unix_mode,
            };
            if previous_identity.as_ref() == Some(&desired) {
                bail!("Shell rendered-file replacement contains no changed file");
            }
            let receipts = rendered
                .receipt_transitions
                .iter()
                .map(|(target, previous, desired)| ShellReceiptTransitionV1 {
                    target: target.clone(),
                    previous: previous.as_ref().map(receipt_contract).map(Box::new),
                    desired: Box::new(receipt_contract(desired)),
                })
                .collect();
            actions.push(DeclarativeActionV1::replace_shell_rendered_file(
                format!("replace-rendered:{}", rendered.target),
                &rendered.target,
                "rendered-output",
                ShellRenderedFileReplacementSpecV1 {
                    destination: rendered.destination.clone(),
                    previous: previous_identity.clone(),
                    desired,
                    receipts,
                },
            ));
            prepared.push(PreparedShellAction::RenderedFile {
                destination: rendered.destination.clone(),
                rollback,
                previous_present: previous_identity.is_some(),
                bytes: rendered.bytes.clone(),
                unix_mode: rendered.unix_mode,
            });
        }
        for removal in shared.rendered_removals {
            let rollback = managed_file_rollback_path(&removal.destination);
            match self.host().metadata(&rollback).await {
                Err(error) if error.is_not_found() => {}
                Ok(_) => bail!(
                    "Shell rendered-file rollback path is occupied: {}",
                    rollback.display()
                ),
                Err(error) => {
                    return Err(
                        error.into_anyhow("revalidating Shell rendered-file removal rollback path")
                    );
                }
            }
            if !observe_shell_file(self.host(), &removal.destination)
                .await?
                .matches(&removal.previous)
            {
                bail!("Shell rendered file changed after Plan approval");
            }
            let manifest =
                load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
            let receipts = removal
                .previous_receipts
                .iter()
                .map(|(target, entry)| ShellReceiptRemovalV1 {
                    target: target.clone(),
                    previous: Box::new(receipt_contract(entry)),
                })
                .collect::<Vec<_>>();
            if !shell_removal_receipts_match_previous(&manifest, &receipts) {
                bail!("Shell rendered-file receipts changed after Plan approval");
            }
            actions.push(DeclarativeActionV1::remove_shell_rendered_file(
                format!("remove-rendered:{}", removal.target),
                &removal.target,
                "rendered-output",
                ShellRenderedFileRemovalSpecV1 {
                    destination: removal.destination.clone(),
                    previous: removal.previous.clone(),
                    receipts,
                },
            ));
            prepared.push(PreparedShellAction::RemoveRenderedFile {
                destination: removal.destination.clone(),
                rollback,
            });
        }
        for removal in shared.cache_removals {
            let mut files = Vec::new();
            let mut prepared_files = Vec::new();
            for (destination, previous) in &removal.files {
                let rollback = managed_file_rollback_path(destination);
                match self.host().metadata(&rollback).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Shell cache removal rollback path is occupied: {}",
                        rollback.display()
                    ),
                    Err(error) => {
                        return Err(
                            error.into_anyhow("revalidating Shell cache removal rollback path")
                        );
                    }
                }
                if !observe_shell_file(self.host(), destination)
                    .await?
                    .matches(previous)
                {
                    bail!("Shell cache file changed after Plan approval");
                }
                files.push(ShellCacheFileRemovalV1 {
                    destination: destination.clone(),
                    rollback: rollback.clone(),
                    previous: previous.clone(),
                });
                prepared_files.push((destination.clone(), rollback));
            }
            let receipts = removal
                .previous_receipts
                .iter()
                .map(|(target, entry)| ShellReceiptRemovalV1 {
                    target: target.clone(),
                    previous: Box::new(receipt_contract(entry)),
                })
                .collect::<Vec<_>>();
            let manifest =
                load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
            if !shell_removal_receipts_match_previous(&manifest, &receipts) {
                bail!("Shell cache receipts changed after Plan approval");
            }
            actions.push(DeclarativeActionV1::remove_shell_cache(
                format!("remove-cache:{}", removal.target),
                &removal.target,
                "preset-cache",
                ShellCacheRemovalSpecV1 { files, receipts },
            ));
            prepared.push(PreparedShellAction::RemoveCache {
                files: prepared_files,
            });
        }
        for removal in shared.snapshot_removals {
            let rollback = shell_snapshot_rollback_path(&removal.destination);
            match self.host().metadata(&rollback).await {
                Err(error) if error.is_not_found() => {}
                Ok(_) => bail!(
                    "Shell snapshot removal rollback path is occupied: {}",
                    rollback.display()
                ),
                Err(error) => {
                    return Err(
                        error.into_anyhow("revalidating Shell snapshot removal rollback path")
                    );
                }
            }
            if observe_shell_tree(
                self.host(),
                &removal.destination,
                &removal.previous_files,
                false,
            )
            .await?
                != ShellTreeObservation::Exact
            {
                bail!("Shell snapshot changed after Plan approval");
            }
            let receipts = removal
                .previous_receipts
                .iter()
                .map(|(target, entry)| ShellReceiptRemovalV1 {
                    target: target.clone(),
                    previous: Box::new(receipt_contract(entry)),
                })
                .collect::<Vec<_>>();
            let manifest =
                load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
            if !shell_removal_receipts_match_previous(&manifest, &receipts) {
                bail!("Shell snapshot receipts changed after Plan approval");
            }
            actions.push(DeclarativeActionV1::remove_shell_snapshot(
                format!("remove-snapshot:{}", removal.target),
                &removal.target,
                "shared-snapshot",
                ShellSnapshotRemovalSpecV1 {
                    destination: removal.destination.clone(),
                    previous_files: removal.previous_files.clone(),
                    receipts,
                },
            ));
            prepared.push(PreparedShellAction::RemoveSnapshot {
                destination: removal.destination.clone(),
                rollback,
            });
        }
        for profile in shared.profiles {
            let mut action_files = Vec::new();
            let mut prepared_files = Vec::new();
            for file in &profile.files {
                let rollback = managed_file_rollback_path(&file.destination);
                match self.host().metadata(&rollback).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Shell profile rollback path is occupied: {}",
                        rollback.display()
                    ),
                    Err(error) => {
                        return Err(error.into_anyhow("revalidating Shell profile rollback path"));
                    }
                }
                let previous = match observe_shell_file(self.host(), &file.destination).await? {
                    ShellFileObservation::Missing => None,
                    ShellFileObservation::Regular(identity) => Some(identity),
                    ShellFileObservation::Other => {
                        bail!(
                            "Shell profile destination is not a regular file: {}",
                            file.destination.display()
                        )
                    }
                };
                let desired = file.desired.as_ref().map(|bytes| ShellFileIdentityV1 {
                    content_hash: hash_content(bytes),
                    unix_mode: file.unix_mode,
                });
                if previous == desired {
                    bail!("Shell profile action contains no file change");
                }
                if file.ownership == ShellProfileFileOwnershipV1::SentinelBlock {
                    let previous_block_hash =
                        observe_shell_profile_block_hash(self.host(), &file.destination).await?;
                    let desired_block_hash = match file.desired.as_deref() {
                        Some(bytes) => shell_profile_block_hash(bytes)?,
                        None => None,
                    };
                    if previous_block_hash != file.previous_block_hash
                        || desired_block_hash != file.desired_block_hash
                    {
                        bail!("Shell profile sentinel changed after Plan approval");
                    }
                }
                action_files.push(ShellProfileFileV1 {
                    destination: file.destination.clone(),
                    rollback: rollback.clone(),
                    ownership: file.ownership,
                    previous,
                    desired,
                    previous_block_hash: file.previous_block_hash,
                    desired_block_hash: file.desired_block_hash,
                });
                prepared_files.push(PreparedShellProfileFile {
                    destination: file.destination.clone(),
                    rollback,
                    desired: file.desired.clone(),
                    unix_mode: file.unix_mode,
                    ownership: file.ownership,
                });
            }
            let receipt_transitions = profile
                .receipt_transitions
                .iter()
                .map(|(target, previous, desired)| ShellReceiptTransitionV1 {
                    target: target.clone(),
                    previous: previous.as_ref().map(receipt_contract).map(Box::new),
                    desired: Box::new(receipt_contract(desired)),
                })
                .collect::<Vec<_>>();
            let receipt_removals = profile
                .receipt_removals
                .iter()
                .map(|(target, previous)| ShellReceiptRemovalV1 {
                    target: target.clone(),
                    previous: Box::new(receipt_contract(previous)),
                })
                .collect::<Vec<_>>();
            let manifest =
                load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
            if (!receipt_transitions.is_empty()
                && !shell_receipts_match_previous(&manifest, &receipt_transitions))
                || (!receipt_removals.is_empty()
                    && !shell_removal_receipts_match_previous(&manifest, &receipt_removals))
            {
                bail!("Shell profile receipts changed after Plan approval");
            }
            actions.push(DeclarativeActionV1::reconcile_shell_profile(
                format!("reconcile-profile:{}", profile.target),
                &profile.target,
                "profile",
                ShellProfileReconciliationSpecV1 {
                    files: action_files,
                    receipt_transitions,
                    receipt_removals,
                    legacy_targets: profile.legacy_targets.clone(),
                },
            ));
            prepared.push(PreparedShellAction::Profile {
                files: prepared_files,
            });
        }
        for creation in creations {
            let resources = prepare_launcher_resources(&self.context().bin_dir, creation.spec);
            for resource in &resources {
                match self.host().metadata(resource.destination()).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Shell launcher path changed after Plan approval: {}",
                        resource.destination().display()
                    ),
                    Err(error) => {
                        return Err(error.into_anyhow("revalidating Shell launcher creation"));
                    }
                }
            }
            actions.push(DeclarativeActionV1::create_shell_launcher(
                format!("create-launcher:{}", creation.target),
                &creation.target,
                "launcher",
                receipt_contract(&creation.receipt),
                resources.iter().map(resource_contract).collect(),
            ));
            prepared.push(PreparedShellAction::Create(resources));
        }
        for update in updates {
            let previous_spec = shell_link_spec_from_manifest_entry(&update.previous_receipt)?;
            let previous_resources =
                prepare_launcher_resources(&self.context().bin_dir, &previous_spec);
            let desired_resources =
                prepare_launcher_resources(&self.context().bin_dir, update.desired_spec);
            if previous_resources.len() != desired_resources.len()
                || previous_resources
                    .iter()
                    .zip(&desired_resources)
                    .any(|(previous, desired)| previous.destination() != desired.destination())
            {
                bail!(
                    "Shell launcher resource shape changed outside the supported update boundary"
                );
            }
            let mut prepared_resources = Vec::new();
            let mut action_resources = Vec::new();
            for (previous, desired) in previous_resources.iter().zip(desired_resources) {
                let previous = resource_contract(previous);
                let desired_contract = resource_contract(&desired);
                if previous == desired_contract {
                    continue;
                }
                if observe_launcher_resource(self.host(), &previous).await?
                    != LauncherObservation::Exact
                {
                    bail!(
                        "Shell launcher changed after Plan approval: {}",
                        previous.destination().display()
                    );
                }
                let rollback = managed_file_rollback_path(previous.destination());
                match self.host().metadata(&rollback).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Shell launcher rollback path is occupied: {}",
                        rollback.display()
                    ),
                    Err(error) => {
                        return Err(error.into_anyhow("revalidating Shell launcher rollback path"));
                    }
                }
                action_resources.push(ShellLauncherUpdateResourceV1 {
                    previous: previous.clone(),
                    desired: desired_contract,
                    rollback: rollback.clone(),
                });
                prepared_resources.push(PreparedShellLauncherUpdateResource {
                    previous,
                    desired,
                    rollback,
                });
            }
            if action_resources.is_empty() {
                bail!("Shell launcher update contains no changed launcher resource");
            }
            actions.push(DeclarativeActionV1::update_shell_launcher(
                format!("update-launcher:{}", update.target),
                &update.target,
                "launcher",
                receipt_contract(&update.previous_receipt),
                receipt_contract(&update.desired_receipt),
                action_resources,
            ));
            prepared.push(PreparedShellAction::Update(prepared_resources));
        }
        for removal in removals {
            let previous_spec = shell_link_spec_from_manifest_entry(&removal.previous_receipt)?;
            let previous_resources =
                prepare_launcher_resources(&self.context().bin_dir, &previous_spec);
            let mut action_resources = Vec::new();
            for previous in &previous_resources {
                let previous = resource_contract(previous);
                if observe_launcher_resource(self.host(), &previous).await?
                    != LauncherObservation::Exact
                {
                    bail!(
                        "Shell launcher changed after Plan approval: {}",
                        previous.destination().display()
                    );
                }
                let rollback = managed_file_rollback_path(previous.destination());
                match self.host().metadata(&rollback).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Shell launcher rollback path is occupied: {}",
                        rollback.display()
                    ),
                    Err(error) => {
                        return Err(error.into_anyhow("revalidating Shell launcher rollback path"));
                    }
                }
                action_resources.push(ShellLauncherRemovalResourceV1 { previous, rollback });
            }
            actions.push(DeclarativeActionV1::remove_shell_launcher(
                format!("remove-launcher:{}", removal.target),
                &removal.target,
                "launcher",
                receipt_contract(&removal.previous_receipt),
                action_resources.clone(),
            ));
            prepared.push(PreparedShellAction::Remove(action_resources));
        }
        for removal in legacy_removals {
            let mut action_resources = Vec::new();
            for previous in &removal.resources {
                let previous = resource_contract(previous);
                if observe_launcher_resource(self.host(), &previous).await?
                    != LauncherObservation::Exact
                {
                    bail!(
                        "Legacy Shell launcher changed after Plan approval: {}",
                        previous.destination().display()
                    );
                }
                let rollback = managed_file_rollback_path(previous.destination());
                match self.host().metadata(&rollback).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "Legacy Shell launcher rollback path is occupied: {}",
                        rollback.display()
                    ),
                    Err(error) => {
                        return Err(
                            error.into_anyhow("revalidating legacy Shell launcher rollback path")
                        );
                    }
                }
                action_resources.push(ShellLauncherRemovalResourceV1 { previous, rollback });
            }
            actions.push(DeclarativeActionV1::remove_legacy_shell_launcher(
                format!("remove-legacy-launcher:{}", removal.target),
                &removal.target,
                "launcher",
                action_resources.clone(),
            ));
            prepared.push(PreparedShellAction::Remove(action_resources));
        }
        let action_ir = ActionIrV1::new(
            format!(
                "shell-launcher-reconcile:{}",
                approval.plan_fingerprint.as_hex()
            ),
            actions,
        );
        action_ir.validate()?;
        let requirements =
            action_ir.permission_requirements(|path| review_path(self.context(), path));
        if !requirements.uncomputable_codes.is_empty()
            || requirements
                .required
                .iter()
                .any(|permission| !approval.approved_permissions.contains(permission))
        {
            bail!("Shell launcher action permissions exceed the approved security Plan");
        }
        let journal_path = review_path(
            self.context(),
            &self.context().shine_dir.join(SHELL_OPERATION_JOURNAL_FILE),
        );
        for access in [FilesystemAccessV1::Write, FilesystemAccessV1::Remove] {
            if !approval
                .approved_permissions
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: journal_path.clone(),
                })
            {
                bail!("Shell operation journal permissions exceed the approved security Plan");
            }
        }
        let mut journal = ShellOperationJournalV1::new(action_ir, approval.clone());
        save_shell_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        for (action, prepared_action) in journal.action_ir.actions.clone().iter().zip(&prepared) {
            match prepared_action {
                PreparedShellAction::Cache { files } => {
                    for file in files {
                        if file.previous_present {
                            self.host()
                                .rename(&file.destination, &file.rollback)
                                .await
                                .map_err(|error| {
                                    error
                                        .into_anyhow("moving previous Shell cache file to rollback")
                                })?;
                        }
                        self.host()
                            .write_atomic(&file.destination, &file.bytes)
                            .await
                            .map_err(|error| error.into_anyhow("writing Shell cache file"))?;
                        if let Some(mode) = file.unix_mode {
                            self.host()
                                .set_mode(&file.destination, mode)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("setting Shell cache file mode")
                                })?;
                        }
                    }
                }
                PreparedShellAction::Snapshot {
                    destination,
                    stage,
                    rollback,
                    previous_present,
                    files,
                } => {
                    for (relative, bytes) in files {
                        self.host()
                            .write_atomic(&stage.join(relative), bytes)
                            .await
                            .map_err(|error| error.into_anyhow("staging Shell snapshot"))?;
                    }
                    if *previous_present {
                        self.host()
                            .rename(destination, rollback)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("moving previous Shell snapshot to rollback")
                            })?;
                    }
                    self.host()
                        .rename(stage, destination)
                        .await
                        .map_err(|error| error.into_anyhow("installing staged Shell snapshot"))?;
                }
                PreparedShellAction::RenderedFile {
                    destination,
                    rollback,
                    previous_present,
                    bytes,
                    unix_mode,
                } => {
                    if *previous_present {
                        self.host()
                            .rename(destination, rollback)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("moving previous Shell rendered file to rollback")
                            })?;
                    }
                    self.host()
                        .write_atomic(destination, bytes)
                        .await
                        .map_err(|error| error.into_anyhow("writing Shell rendered file"))?;
                    if let Some(mode) = unix_mode {
                        self.host()
                            .set_mode(destination, *mode)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("setting Shell rendered-file mode")
                            })?;
                    }
                }
                PreparedShellAction::RemoveRenderedFile {
                    destination,
                    rollback,
                } => {
                    let ActionKindV1::RemoveShellRenderedFile { previous, .. } = &action.kind
                    else {
                        bail!("prepared Shell rendered-file removal action changed shape");
                    };
                    if !observe_shell_file(self.host(), destination)
                        .await?
                        .matches(previous)
                    {
                        bail!("Shell rendered file changed before transactional removal");
                    }
                    self.host()
                        .rename(destination, rollback)
                        .await
                        .map_err(|error| {
                            error.into_anyhow("moving removed Shell rendered file to rollback")
                        })?;
                }
                PreparedShellAction::RemoveCache { files } => {
                    let ActionKindV1::RemoveShellCache {
                        files: action_files,
                        ..
                    } = &action.kind
                    else {
                        bail!("prepared Shell cache removal action changed shape");
                    };
                    for ((destination, rollback), action_file) in files.iter().zip(action_files) {
                        if !observe_shell_file(self.host(), destination)
                            .await?
                            .matches(&action_file.previous)
                        {
                            bail!("Shell cache file changed before transactional removal");
                        }
                        self.host()
                            .rename(destination, rollback)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("moving removed Shell cache file to rollback")
                            })?;
                    }
                }
                PreparedShellAction::RemoveSnapshot {
                    destination,
                    rollback,
                } => {
                    let ActionKindV1::RemoveShellSnapshot { previous_files, .. } = &action.kind
                    else {
                        bail!("prepared Shell snapshot removal action changed shape");
                    };
                    if observe_shell_tree(self.host(), destination, previous_files, false).await?
                        != ShellTreeObservation::Exact
                    {
                        bail!("Shell snapshot changed before transactional removal");
                    }
                    self.host()
                        .rename(destination, rollback)
                        .await
                        .map_err(|error| {
                            error.into_anyhow("moving removed Shell snapshot to rollback")
                        })?;
                }
                PreparedShellAction::Profile { files } => {
                    let ActionKindV1::ReconcileShellProfile {
                        files: action_files,
                        ..
                    } = &action.kind
                    else {
                        bail!("prepared Shell profile action changed shape");
                    };
                    for (file, action_file) in files.iter().zip(action_files) {
                        let previous = observe_shell_file(self.host(), &file.destination).await?;
                        if action_file
                            .previous
                            .as_ref()
                            .is_some_and(|expected| !previous.matches(expected))
                            || (action_file.previous.is_none()
                                && previous != ShellFileObservation::Missing)
                        {
                            bail!("Shell profile changed before transactional reconciliation");
                        }
                        if action_file.previous.is_some() {
                            self.host()
                                .rename(&file.destination, &file.rollback)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("moving previous Shell profile to rollback")
                                })?;
                        }
                        if let Some(desired) = &file.desired {
                            self.host()
                                .write_atomic(&file.destination, desired)
                                .await
                                .map_err(|error| error.into_anyhow("writing Shell profile"))?;
                            if let Some(mode) = file.unix_mode {
                                self.host()
                                    .set_mode(&file.destination, mode)
                                    .await
                                    .map_err(|error| {
                                        error.into_anyhow("setting Shell profile mode")
                                    })?;
                            }
                        }
                        if file.ownership == ShellProfileFileOwnershipV1::SentinelBlock
                            && observe_shell_profile_block_hash(self.host(), &file.destination)
                                .await?
                                != action_file.desired_block_hash
                        {
                            bail!("Shell profile sentinel write did not match its action");
                        }
                    }
                }
                PreparedShellAction::Create(resources) => {
                    for resource in resources {
                        apply_prepared_launcher_resource(self.host(), resource).await?;
                    }
                }
                PreparedShellAction::Update(resources) => {
                    for resource in resources {
                        if observe_launcher_resource(self.host(), &resource.previous).await?
                            != LauncherObservation::Exact
                        {
                            bail!("Shell launcher changed before transactional update");
                        }
                        self.host()
                            .rename(resource.previous.destination(), &resource.rollback)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("moving previous Shell launcher to rollback")
                            })?;
                        apply_prepared_launcher_resource(self.host(), &resource.desired).await?;
                    }
                }
                PreparedShellAction::Remove(resources) => {
                    for resource in resources {
                        if observe_launcher_resource(self.host(), &resource.previous).await?
                            != LauncherObservation::Exact
                        {
                            bail!("Shell launcher changed before transactional removal");
                        }
                        self.host()
                            .rename(resource.previous.destination(), &resource.rollback)
                            .await
                            .map_err(|error| {
                                error.into_anyhow("moving removed Shell launcher to rollback")
                            })?;
                    }
                }
            }
            journal.applied.push(action.action_id.clone());
            save_shell_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        }
        Ok(Some(ShellOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            _operation_guard: operation_guard,
        }))
    }

    pub(crate) async fn mark_shell_launcher_receipt_committed(
        &self,
        execution: &ShellOperationExecutionV1,
    ) -> Result<()> {
        let (mut journal, _) = load_shell_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .context("no Shell operation journal is available to mark committed")?;
        if journal.action_ir.operation_id != execution.operation_id {
            bail!("Shell operation journal changed before receipt commit marker");
        }
        let manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        for action in &journal.action_ir.actions {
            match &action.kind {
                ActionKindV1::RemoveShellLauncher {
                    previous_receipt, ..
                } => {
                    if !journal.applied.contains(&action.action_id)
                        || matching_shell_receipt(&manifest, &action.target, previous_receipt)
                            != ReceiptState::Missing
                    {
                        bail!(
                            "Shell launcher removal cannot mark its receipt committed before the exact receipt is removed"
                        );
                    }
                    if !journal.receipt_committed.contains(&action.action_id) {
                        journal.receipt_committed.push(action.action_id.clone());
                    }
                }
                ActionKindV1::RemoveLegacyShellLauncher { .. } => {
                    if !journal.applied.contains(&action.action_id)
                        || manifest.find(&action.target).is_some()
                    {
                        bail!(
                            "Legacy Shell launcher removal cannot mark committed before the launcher is removed and its receipt remains absent"
                        );
                    }
                    if !journal.receipt_committed.contains(&action.action_id) {
                        journal.receipt_committed.push(action.action_id.clone());
                    }
                }
                ActionKindV1::ReconcileShellProfile {
                    files,
                    receipt_transitions,
                    receipt_removals,
                    legacy_targets,
                } => {
                    let receipts_match = shell_profile_receipts_match_desired(
                        &manifest,
                        receipt_transitions,
                        receipt_removals,
                        legacy_targets,
                    );
                    let mut blocked_file = None;
                    for (index, file) in files.iter().enumerate() {
                        if assess_shell_profile_file_recovery(self.host(), file, true).await?
                            == ShellProfileFileRecoveryAssessment::Blocked
                        {
                            blocked_file = Some(index);
                            break;
                        }
                    }
                    if !journal.applied.contains(&action.action_id) {
                        bail!("Shell profile cannot mark committed before it is applied");
                    }
                    if !receipts_match {
                        bail!("Shell profile cannot mark committed before receipts are durable");
                    }
                    if let Some(index) = blocked_file {
                        bail!("Shell profile cannot mark committed before file {index} is durable");
                    }
                    if !journal.receipt_committed.contains(&action.action_id) {
                        journal.receipt_committed.push(action.action_id.clone());
                    }
                }
                ActionKindV1::ReplaceShellSnapshot {
                    destination,
                    desired_files,
                    receipts,
                    ..
                } => {
                    if !journal.applied.contains(&action.action_id)
                        || !shell_receipts_match_desired(&manifest, receipts)
                        || observe_shell_tree(self.host(), destination, desired_files, false)
                            .await?
                            != ShellTreeObservation::Exact
                    {
                        bail!(
                            "Shell snapshot cannot mark its receipt committed before the desired tree and receipts are durable"
                        );
                    }
                    if !journal.receipt_committed.contains(&action.action_id) {
                        journal.receipt_committed.push(action.action_id.clone());
                    }
                }
                ActionKindV1::ReplaceShellCache { files, receipts } => {
                    let mut files_match = true;
                    for file in files {
                        files_match &= observe_shell_file(self.host(), &file.destination)
                            .await?
                            .matches(&file.desired);
                    }
                    if !journal.applied.contains(&action.action_id)
                        || !shell_receipts_match_desired(&manifest, receipts)
                        || !files_match
                    {
                        bail!(
                            "Shell cache cannot mark its receipt committed before the desired files and receipts are durable"
                        );
                    }
                    if !journal.receipt_committed.contains(&action.action_id) {
                        journal.receipt_committed.push(action.action_id.clone());
                    }
                }
                ActionKindV1::RemoveShellCache { files, receipts } => {
                    let mut files_match = true;
                    for file in files {
                        files_match &= observe_shell_file(self.host(), &file.destination).await?
                            == ShellFileObservation::Missing
                            && observe_shell_file(self.host(), &file.rollback)
                                .await?
                                .matches(&file.previous);
                    }
                    if !journal.applied.contains(&action.action_id)
                        || !shell_removal_receipts_match_missing(&manifest, receipts)
                        || !files_match
                    {
                        bail!(
                            "Shell cache removal cannot mark committed before files are staged and receipts are removed"
                        );
                    }
                    if !journal.receipt_committed.contains(&action.action_id) {
                        journal.receipt_committed.push(action.action_id.clone());
                    }
                }
                ActionKindV1::RemoveShellSnapshot {
                    destination,
                    rollback,
                    previous_files,
                    receipts,
                } => {
                    if !journal.applied.contains(&action.action_id)
                        || !shell_removal_receipts_match_missing(&manifest, receipts)
                        || observe_shell_tree(self.host(), destination, previous_files, false)
                            .await?
                            != ShellTreeObservation::Missing
                        || observe_shell_tree(self.host(), rollback, previous_files, false).await?
                            != ShellTreeObservation::Exact
                    {
                        bail!(
                            "Shell snapshot removal cannot mark committed before the tree is staged and receipts are removed"
                        );
                    }
                    if !journal.receipt_committed.contains(&action.action_id) {
                        journal.receipt_committed.push(action.action_id.clone());
                    }
                }
                ActionKindV1::ReplaceShellRenderedFile {
                    destination,
                    desired,
                    receipts,
                    ..
                } => {
                    if !journal.applied.contains(&action.action_id)
                        || !shell_receipts_match_desired(&manifest, receipts)
                        || !observe_shell_file(self.host(), destination)
                            .await?
                            .matches(desired)
                    {
                        bail!(
                            "Shell rendered file cannot mark its receipt committed before the desired file and receipts are durable"
                        );
                    }
                    if !journal.receipt_committed.contains(&action.action_id) {
                        journal.receipt_committed.push(action.action_id.clone());
                    }
                }
                ActionKindV1::RemoveShellRenderedFile {
                    destination,
                    rollback,
                    previous,
                    receipts,
                } => {
                    if !journal.applied.contains(&action.action_id)
                        || !shell_removal_receipts_match_missing(&manifest, receipts)
                        || observe_shell_file(self.host(), destination).await?
                            != ShellFileObservation::Missing
                        || !observe_shell_file(self.host(), rollback)
                            .await?
                            .matches(previous)
                    {
                        bail!(
                            "Shell rendered-file removal cannot mark committed before the file is staged and receipts are removed"
                        );
                    }
                    if !journal.receipt_committed.contains(&action.action_id) {
                        journal.receipt_committed.push(action.action_id.clone());
                    }
                }
                _ => {}
            }
        }
        save_shell_operation_journal(self.host(), &self.context().shine_dir, &journal).await
    }

    pub(crate) async fn commit_shell_launcher_operation(
        &self,
        execution: &ShellOperationExecutionV1,
    ) -> Result<()> {
        let (journal, _) = load_shell_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .context("no Shell operation journal is available to commit")?;
        if journal.action_ir.operation_id != execution.operation_id
            || journal.applied.len() != journal.action_ir.actions.len()
        {
            bail!("Shell operation journal is not fully applied or changed before commit");
        }
        let manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        for action in &journal.action_ir.actions {
            match &action.kind {
                ActionKindV1::CreateShellLauncher { receipt, resources } => {
                    if matching_shell_receipt(&manifest, &action.target, receipt)
                        != ReceiptState::Matching
                    {
                        bail!("Shell operation journal cannot commit before its matching receipt");
                    }
                    for resource in resources {
                        if observe_launcher_resource(self.host(), resource).await?
                            != LauncherObservation::Exact
                        {
                            bail!("Shell launcher changed before operation commit");
                        }
                    }
                }
                ActionKindV1::UpdateShellLauncher {
                    desired_receipt,
                    resources,
                    ..
                } => {
                    if matching_shell_receipt(&manifest, &action.target, desired_receipt)
                        != ReceiptState::Matching
                    {
                        bail!("Shell operation journal cannot commit before its matching receipt");
                    }
                    for resource in resources {
                        if observe_launcher_resource(self.host(), &resource.desired).await?
                            != LauncherObservation::Exact
                            || observe_launcher_resource_at(
                                self.host(),
                                &resource.previous,
                                &resource.rollback,
                            )
                            .await?
                                != LauncherObservation::Exact
                        {
                            bail!("Shell launcher update changed before operation commit");
                        }
                    }
                }
                ActionKindV1::RemoveShellLauncher {
                    previous_receipt,
                    resources,
                } => {
                    if !journal.receipt_committed.contains(&action.action_id)
                        || matching_shell_receipt(&manifest, &action.target, previous_receipt)
                            != ReceiptState::Missing
                    {
                        bail!(
                            "Shell launcher removal cannot commit before its receipt commit marker"
                        );
                    }
                    for resource in resources {
                        if observe_launcher_resource(self.host(), &resource.previous).await?
                            != LauncherObservation::Missing
                            || observe_launcher_resource_at(
                                self.host(),
                                &resource.previous,
                                &resource.rollback,
                            )
                            .await?
                                != LauncherObservation::Exact
                        {
                            bail!("Shell launcher removal changed before operation commit");
                        }
                    }
                }
                ActionKindV1::RemoveLegacyShellLauncher { resources } => {
                    if !journal.receipt_committed.contains(&action.action_id)
                        || manifest.find(&action.target).is_some()
                    {
                        bail!(
                            "Legacy Shell launcher removal cannot commit before its commit marker"
                        );
                    }
                    for resource in resources {
                        if observe_launcher_resource(self.host(), &resource.previous).await?
                            != LauncherObservation::Missing
                            || observe_launcher_resource_at(
                                self.host(),
                                &resource.previous,
                                &resource.rollback,
                            )
                            .await?
                                != LauncherObservation::Exact
                        {
                            bail!("Legacy Shell launcher removal changed before operation commit");
                        }
                    }
                }
                ActionKindV1::ReplaceShellSnapshot {
                    destination,
                    stage,
                    rollback,
                    previous_present,
                    previous_files,
                    desired_files,
                    receipts,
                } => {
                    if !journal.receipt_committed.contains(&action.action_id)
                        || !shell_receipts_match_desired(&manifest, receipts)
                        || assess_shell_snapshot_recovery(
                            self.host(),
                            destination,
                            stage,
                            rollback,
                            *previous_present,
                            previous_files,
                            desired_files,
                            true,
                        )
                        .await?
                            == ShellSnapshotRecoveryAssessment::Blocked
                    {
                        bail!("Shell snapshot changed before operation commit");
                    }
                }
                ActionKindV1::ReplaceShellCache { files, receipts } => {
                    let mut changed = false;
                    for file in files {
                        changed |= assess_shell_rendered_file_recovery(
                            self.host(),
                            &file.destination,
                            &file.rollback,
                            file.previous.as_ref(),
                            &file.desired,
                            true,
                        )
                        .await?
                            == ShellRenderedFileRecoveryAssessment::Blocked;
                    }
                    if !journal.receipt_committed.contains(&action.action_id)
                        || !shell_receipts_match_desired(&manifest, receipts)
                        || changed
                    {
                        bail!("Shell cache changed before operation commit");
                    }
                }
                ActionKindV1::RemoveShellCache { files, receipts } => {
                    let mut changed = false;
                    for file in files {
                        changed |= assess_shell_rendered_file_removal_recovery(
                            self.host(),
                            &file.destination,
                            &file.rollback,
                            &file.previous,
                            true,
                        )
                        .await?
                            == ShellRenderedFileRemovalRecoveryAssessment::Blocked;
                    }
                    if !journal.receipt_committed.contains(&action.action_id)
                        || !shell_removal_receipts_match_missing(&manifest, receipts)
                        || changed
                    {
                        bail!("Shell cache removal changed before operation commit");
                    }
                }
                ActionKindV1::RemoveShellSnapshot {
                    destination,
                    rollback,
                    previous_files,
                    receipts,
                } => {
                    if !journal.receipt_committed.contains(&action.action_id)
                        || !shell_removal_receipts_match_missing(&manifest, receipts)
                        || assess_shell_tree_removal_recovery(
                            self.host(),
                            destination,
                            rollback,
                            previous_files,
                            true,
                        )
                        .await?
                            == ShellTreeRemovalRecoveryAssessment::Blocked
                    {
                        bail!("Shell snapshot removal changed before operation commit");
                    }
                }
                ActionKindV1::ReconcileShellProfile {
                    files,
                    receipt_transitions,
                    receipt_removals,
                    legacy_targets,
                } => {
                    let receipts_match = shell_profile_receipts_match_desired(
                        &manifest,
                        receipt_transitions,
                        receipt_removals,
                        legacy_targets,
                    );
                    let mut blocked = false;
                    for file in files {
                        blocked |= assess_shell_profile_file_recovery(self.host(), file, true)
                            .await?
                            == ShellProfileFileRecoveryAssessment::Blocked;
                    }
                    if !journal.receipt_committed.contains(&action.action_id)
                        || !receipts_match
                        || blocked
                    {
                        bail!("Shell profile changed before operation commit");
                    }
                }
                ActionKindV1::ReplaceShellRenderedFile {
                    destination,
                    rollback,
                    previous,
                    desired,
                    receipts,
                } => {
                    if !journal.receipt_committed.contains(&action.action_id)
                        || !shell_receipts_match_desired(&manifest, receipts)
                        || assess_shell_rendered_file_recovery(
                            self.host(),
                            destination,
                            rollback,
                            previous.as_ref(),
                            desired,
                            true,
                        )
                        .await?
                            == ShellRenderedFileRecoveryAssessment::Blocked
                    {
                        bail!("Shell rendered file changed before operation commit");
                    }
                }
                ActionKindV1::RemoveShellRenderedFile {
                    destination,
                    rollback,
                    previous,
                    receipts,
                } => {
                    if !journal.receipt_committed.contains(&action.action_id)
                        || !shell_removal_receipts_match_missing(&manifest, receipts)
                        || assess_shell_rendered_file_removal_recovery(
                            self.host(),
                            destination,
                            rollback,
                            previous,
                            true,
                        )
                        .await?
                            == ShellRenderedFileRemovalRecoveryAssessment::Blocked
                    {
                        bail!("Shell rendered-file removal changed before operation commit");
                    }
                }
                _ => unreachable!("validated Shell journal action kind"),
            }
        }
        for action in &journal.action_ir.actions {
            if let ActionKindV1::UpdateShellLauncher { resources, .. } = &action.kind {
                for resource in resources {
                    self.host()
                        .remove_file(&resource.rollback)
                        .await
                        .map_err(|error| error.into_anyhow("cleaning Shell launcher rollback"))?;
                }
            }
            if let ActionKindV1::RemoveShellLauncher { resources, .. }
            | ActionKindV1::RemoveLegacyShellLauncher { resources } = &action.kind
            {
                for resource in resources {
                    self.host()
                        .remove_file(&resource.rollback)
                        .await
                        .map_err(|error| error.into_anyhow("cleaning Shell launcher rollback"))?;
                }
            }
            if let ActionKindV1::ReplaceShellSnapshot {
                rollback,
                previous_present,
                ..
            } = &action.kind
                && *previous_present
            {
                match self.host().remove_dir_all(rollback).await {
                    Ok(()) => {}
                    Err(error) if error.is_not_found() => {}
                    Err(error) => {
                        return Err(error.into_anyhow("cleaning Shell snapshot rollback"));
                    }
                }
            }
            if let ActionKindV1::ReplaceShellCache { files, .. } = &action.kind {
                for file in files.iter().filter(|file| file.previous.is_some()) {
                    match self.host().remove_file(&file.rollback).await {
                        Ok(()) => {}
                        Err(error) if error.is_not_found() => {}
                        Err(error) => {
                            return Err(error.into_anyhow("cleaning Shell cache rollback"));
                        }
                    }
                }
            }
            if let ActionKindV1::RemoveShellCache { files, .. } = &action.kind {
                for file in files {
                    match self.host().remove_file(&file.rollback).await {
                        Ok(()) => {}
                        Err(error) if error.is_not_found() => {}
                        Err(error) => {
                            return Err(error.into_anyhow("cleaning removed Shell cache rollback"));
                        }
                    }
                }
            }
            if let ActionKindV1::RemoveShellSnapshot { rollback, .. } = &action.kind {
                match self.host().remove_dir_all(rollback).await {
                    Ok(()) => {}
                    Err(error) if error.is_not_found() => {}
                    Err(error) => {
                        return Err(error.into_anyhow("cleaning removed Shell snapshot rollback"));
                    }
                }
            }
            if let ActionKindV1::ReconcileShellProfile { files, .. } = &action.kind {
                for file in files.iter().filter(|file| file.previous.is_some()) {
                    match self.host().remove_file(&file.rollback).await {
                        Ok(()) => {}
                        Err(error) if error.is_not_found() => {}
                        Err(error) => {
                            return Err(error.into_anyhow("cleaning Shell profile rollback"));
                        }
                    }
                }
            }
            if let ActionKindV1::ReplaceShellRenderedFile {
                rollback, previous, ..
            } = &action.kind
                && previous.is_some()
            {
                match self.host().remove_file(rollback).await {
                    Ok(()) => {}
                    Err(error) if error.is_not_found() => {}
                    Err(error) => {
                        return Err(error.into_anyhow("cleaning Shell rendered-file rollback"));
                    }
                }
            }
            if let ActionKindV1::RemoveShellRenderedFile { rollback, .. } = &action.kind {
                match self.host().remove_file(rollback).await {
                    Ok(()) => {}
                    Err(error) if error.is_not_found() => {}
                    Err(error) => {
                        return Err(
                            error.into_anyhow("cleaning removed Shell rendered-file rollback")
                        );
                    }
                }
            }
        }
        remove_shell_operation_journal(self.host(), &self.context().shine_dir).await
    }

    pub async fn recover_shell_operation_approved(
        &self,
        approval: &PlanApprovalV1,
    ) -> Result<ShellRecoveryReportV1> {
        approval.validate(&self.plan_shell_operation_recovery().await?)?;
        let _guard = self.host().acquire_privileged_operation().await?;
        approval.validate(&self.plan_shell_operation_recovery().await?)?;
        let (journal, _) = load_shell_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .context("no interrupted Shell operation is available for recovery")?;
        let mut manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let mut rolled_back_actions = Vec::new();
        let mut restored_shared_receipts = BTreeSet::new();
        for action in &journal.action_ir.actions {
            match &action.kind {
                ActionKindV1::RemoveShellRenderedFile { receipts, .. }
                | ActionKindV1::RemoveShellCache { receipts, .. }
                | ActionKindV1::RemoveShellSnapshot { receipts, .. } => {
                    if journal.receipt_committed.contains(&action.action_id) {
                        if !shell_removal_receipts_match_missing(&manifest, receipts) {
                            bail!(
                                "Shell rendered-file removal receipts changed after recovery approval"
                            );
                        }
                    } else if shell_removal_receipts_match_previous(&manifest, receipts) {
                    } else if shell_removal_receipts_match_missing(&manifest, receipts) {
                        restore_shell_removed_receipts(&mut manifest, receipts)?;
                        restored_shared_receipts.insert(action.action_id.clone());
                    } else {
                        bail!(
                            "Shell rendered-file removal receipts changed after recovery approval"
                        );
                    }
                }
                ActionKindV1::ReconcileShellProfile {
                    receipt_transitions,
                    receipt_removals,
                    legacy_targets,
                    ..
                } => {
                    if journal.receipt_committed.contains(&action.action_id) {
                        if !shell_profile_receipts_match_desired(
                            &manifest,
                            receipt_transitions,
                            receipt_removals,
                            legacy_targets,
                        ) {
                            bail!("Shell profile receipts changed after recovery approval");
                        }
                    } else if shell_profile_receipts_match_previous(
                        &manifest,
                        receipt_transitions,
                        receipt_removals,
                        legacy_targets,
                    ) {
                    } else if shell_profile_receipts_match_desired(
                        &manifest,
                        receipt_transitions,
                        receipt_removals,
                        legacy_targets,
                    ) {
                        restore_shell_previous_receipts(&mut manifest, receipt_transitions)?;
                        restore_shell_removed_receipts(&mut manifest, receipt_removals)?;
                        restored_shared_receipts.insert(action.action_id.clone());
                    } else {
                        bail!("Shell profile receipts changed after recovery approval");
                    }
                }
                _ => {
                    let Some(receipts) = shell_shared_receipt_transitions(&action.kind) else {
                        continue;
                    };
                    if journal.receipt_committed.contains(&action.action_id) {
                        if !shell_receipts_match_desired(&manifest, receipts) {
                            bail!("Shell shared-resource receipts changed after recovery approval");
                        }
                    } else if shell_receipts_match_previous(&manifest, receipts) {
                    } else if shell_receipts_match_desired(&manifest, receipts) {
                        restore_shell_previous_receipts(&mut manifest, receipts)?;
                        restored_shared_receipts.insert(action.action_id.clone());
                    } else {
                        bail!("Shell shared-resource receipts changed after recovery approval");
                    }
                }
            }
        }
        if !restored_shared_receipts.is_empty() {
            manifest
                .save(self.host(), &self.context().shine_dir)
                .await?;
        }
        for action in journal.action_ir.actions.iter().rev() {
            let mut changed = false;
            match &action.kind {
                ActionKindV1::CreateShellLauncher { receipt, resources } => {
                    match matching_shell_receipt(&manifest, &action.target, receipt) {
                        ReceiptState::Matching => continue,
                        ReceiptState::Conflict => {
                            bail!("Shell receipt changed after recovery approval")
                        }
                        ReceiptState::Missing => {}
                    }
                    for resource in resources.iter().rev() {
                        match observe_launcher_resource(self.host(), resource).await? {
                            LauncherObservation::Missing => {}
                            LauncherObservation::Exact => {
                                self.host()
                                    .remove_file(resource.destination())
                                    .await
                                    .map_err(|error| {
                                        error.into_anyhow("rolling back Shell launcher")
                                    })?;
                                changed = true;
                            }
                            LauncherObservation::Changed => {
                                bail!("Shell launcher changed after recovery approval")
                            }
                        }
                    }
                }
                ActionKindV1::UpdateShellLauncher {
                    previous_receipt,
                    desired_receipt,
                    resources,
                } => {
                    let committed =
                        matching_shell_receipt(&manifest, &action.target, desired_receipt)
                            == ReceiptState::Matching;
                    if !committed
                        && matching_shell_receipt(&manifest, &action.target, previous_receipt)
                            != ReceiptState::Matching
                    {
                        bail!("Shell receipt changed after recovery approval");
                    }
                    for resource in resources.iter().rev() {
                        match observe_launcher_update_resource(self.host(), resource, committed)
                            .await?
                        {
                            LauncherUpdateObservation::Stable => {}
                            LauncherUpdateObservation::RestoreMoved => {
                                self.host()
                                    .rename(&resource.rollback, resource.previous.destination())
                                    .await
                                    .map_err(|error| {
                                        error.into_anyhow("restoring previous Shell launcher")
                                    })?;
                                changed = true;
                            }
                            LauncherUpdateObservation::RestoreReplaced => {
                                self.host()
                                    .remove_file(resource.desired.destination())
                                    .await
                                    .map_err(|error| {
                                        error.into_anyhow("removing replacement Shell launcher")
                                    })?;
                                self.host()
                                    .rename(&resource.rollback, resource.previous.destination())
                                    .await
                                    .map_err(|error| {
                                        error.into_anyhow("restoring previous Shell launcher")
                                    })?;
                                changed = true;
                            }
                            LauncherUpdateObservation::CleanupRollback => {
                                self.host().remove_file(&resource.rollback).await.map_err(
                                    |error| error.into_anyhow("cleaning Shell launcher rollback"),
                                )?;
                                changed = true;
                            }
                            LauncherUpdateObservation::Changed => {
                                bail!("Shell launcher update changed after recovery approval")
                            }
                        }
                    }
                }
                ActionKindV1::RemoveShellLauncher {
                    previous_receipt,
                    resources,
                } => {
                    let committed = journal.receipt_committed.contains(&action.action_id);
                    match matching_shell_receipt(&manifest, &action.target, previous_receipt) {
                        ReceiptState::Missing if committed => {}
                        ReceiptState::Missing => {
                            manifest.replace_targets(
                                &BTreeSet::from([action.target.clone()]),
                                vec![manifest_entry_from_receipt(previous_receipt)?],
                            );
                            manifest
                                .save(self.host(), &self.context().shine_dir)
                                .await?;
                            changed = true;
                        }
                        ReceiptState::Matching if !committed => {}
                        ReceiptState::Matching | ReceiptState::Conflict => {
                            bail!("Shell receipt changed after recovery approval")
                        }
                    }
                    for resource in resources.iter().rev() {
                        match observe_launcher_removal_resource(self.host(), resource, committed)
                            .await?
                        {
                            LauncherRemovalObservation::Stable => {}
                            LauncherRemovalObservation::RestoreMoved => {
                                self.host()
                                    .rename(&resource.rollback, resource.previous.destination())
                                    .await
                                    .map_err(|error| {
                                        error.into_anyhow("restoring removed Shell launcher")
                                    })?;
                                changed = true;
                            }
                            LauncherRemovalObservation::CleanupRollback => {
                                self.host().remove_file(&resource.rollback).await.map_err(
                                    |error| error.into_anyhow("cleaning Shell launcher rollback"),
                                )?;
                                changed = true;
                            }
                            LauncherRemovalObservation::Changed => {
                                bail!("Shell launcher removal changed after recovery approval")
                            }
                        }
                    }
                }
                ActionKindV1::RemoveLegacyShellLauncher { resources } => {
                    if manifest.find(&action.target).is_some() {
                        bail!("Legacy Shell launcher acquired a receipt after recovery approval");
                    }
                    let committed = journal.receipt_committed.contains(&action.action_id);
                    for resource in resources.iter().rev() {
                        match observe_launcher_removal_resource(self.host(), resource, committed)
                            .await?
                        {
                            LauncherRemovalObservation::Stable => {}
                            LauncherRemovalObservation::RestoreMoved => {
                                self.host()
                                    .rename(&resource.rollback, resource.previous.destination())
                                    .await
                                    .map_err(|error| {
                                        error.into_anyhow("restoring removed legacy Shell launcher")
                                    })?;
                                changed = true;
                            }
                            LauncherRemovalObservation::CleanupRollback => {
                                self.host().remove_file(&resource.rollback).await.map_err(
                                    |error| {
                                        error.into_anyhow("cleaning legacy Shell launcher rollback")
                                    },
                                )?;
                                changed = true;
                            }
                            LauncherRemovalObservation::Changed => {
                                bail!(
                                    "Legacy Shell launcher removal changed after recovery approval"
                                )
                            }
                        }
                    }
                }
                ActionKindV1::ReplaceShellSnapshot {
                    destination,
                    stage,
                    rollback,
                    previous_present,
                    previous_files,
                    desired_files,
                    receipts,
                } => {
                    let committed = journal.receipt_committed.contains(&action.action_id);
                    if committed && !shell_receipts_match_desired(&manifest, receipts) {
                        bail!("Shell snapshot receipts changed after recovery approval");
                    }
                    if !committed && !shell_receipts_match_previous(&manifest, receipts) {
                        bail!("Shell snapshot receipts changed after recovery approval");
                    }
                    changed |= restored_shared_receipts.contains(&action.action_id);
                    match assess_shell_snapshot_recovery(
                        self.host(),
                        destination,
                        stage,
                        rollback,
                        *previous_present,
                        previous_files,
                        desired_files,
                        committed,
                    )
                    .await?
                    {
                        ShellSnapshotRecoveryAssessment::Stable => {}
                        ShellSnapshotRecoveryAssessment::RemoveStage => {
                            self.host().remove_dir_all(stage).await.map_err(|error| {
                                error.into_anyhow("removing staged Shell snapshot")
                            })?;
                            changed = true;
                        }
                        ShellSnapshotRecoveryAssessment::RestoreMoved => {
                            if self.host().metadata(stage).await.is_ok() {
                                self.host().remove_dir_all(stage).await.map_err(|error| {
                                    error.into_anyhow("removing staged Shell snapshot")
                                })?;
                            }
                            self.host()
                                .rename(rollback, destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("restoring previous Shell snapshot")
                                })?;
                            changed = true;
                        }
                        ShellSnapshotRecoveryAssessment::RestoreReplaced => {
                            self.host()
                                .remove_dir_all(destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("removing replacement Shell snapshot")
                                })?;
                            self.host()
                                .rename(rollback, destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("restoring previous Shell snapshot")
                                })?;
                            changed = true;
                        }
                        ShellSnapshotRecoveryAssessment::RemoveCreated => {
                            self.host()
                                .remove_dir_all(destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("removing created Shell snapshot")
                                })?;
                            changed = true;
                        }
                        ShellSnapshotRecoveryAssessment::CleanupRollback => {
                            self.host()
                                .remove_dir_all(rollback)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("cleaning Shell snapshot rollback")
                                })?;
                            changed = true;
                        }
                        ShellSnapshotRecoveryAssessment::Blocked => {
                            bail!("Shell snapshot changed after recovery approval")
                        }
                    }
                }
                ActionKindV1::ReplaceShellCache { files, receipts } => {
                    let committed = journal.receipt_committed.contains(&action.action_id);
                    if committed && !shell_receipts_match_desired(&manifest, receipts) {
                        bail!("Shell cache receipts changed after recovery approval");
                    }
                    if !committed && !shell_receipts_match_previous(&manifest, receipts) {
                        bail!("Shell cache receipts changed after recovery approval");
                    }
                    changed |= restored_shared_receipts.contains(&action.action_id);
                    for file in files.iter().rev() {
                        match assess_shell_rendered_file_recovery(
                            self.host(),
                            &file.destination,
                            &file.rollback,
                            file.previous.as_ref(),
                            &file.desired,
                            committed,
                        )
                        .await?
                        {
                            ShellRenderedFileRecoveryAssessment::Stable => {}
                            ShellRenderedFileRecoveryAssessment::RestoreMoved => {
                                self.host()
                                    .rename(&file.rollback, &file.destination)
                                    .await
                                    .map_err(|error| {
                                        error.into_anyhow("restoring previous Shell cache file")
                                    })?;
                                changed = true;
                            }
                            ShellRenderedFileRecoveryAssessment::RestoreReplaced => {
                                self.host().remove_file(&file.destination).await.map_err(
                                    |error| {
                                        error.into_anyhow("removing replacement Shell cache file")
                                    },
                                )?;
                                self.host()
                                    .rename(&file.rollback, &file.destination)
                                    .await
                                    .map_err(|error| {
                                        error.into_anyhow("restoring previous Shell cache file")
                                    })?;
                                changed = true;
                            }
                            ShellRenderedFileRecoveryAssessment::RemoveCreated => {
                                self.host().remove_file(&file.destination).await.map_err(
                                    |error| error.into_anyhow("removing created Shell cache file"),
                                )?;
                                changed = true;
                            }
                            ShellRenderedFileRecoveryAssessment::CleanupRollback => {
                                self.host()
                                    .remove_file(&file.rollback)
                                    .await
                                    .map_err(|error| {
                                        error.into_anyhow("cleaning Shell cache rollback")
                                    })?;
                                changed = true;
                            }
                            ShellRenderedFileRecoveryAssessment::Blocked => {
                                bail!("Shell cache changed after recovery approval")
                            }
                        }
                    }
                }
                ActionKindV1::RemoveShellCache { files, receipts } => {
                    let committed = journal.receipt_committed.contains(&action.action_id);
                    if committed && !shell_removal_receipts_match_missing(&manifest, receipts) {
                        bail!("Shell cache removal receipts changed after recovery approval");
                    }
                    if !committed && !shell_removal_receipts_match_previous(&manifest, receipts) {
                        bail!("Shell cache removal receipts changed after recovery approval");
                    }
                    changed |= restored_shared_receipts.contains(&action.action_id);
                    for file in files.iter().rev() {
                        match assess_shell_rendered_file_removal_recovery(
                            self.host(),
                            &file.destination,
                            &file.rollback,
                            &file.previous,
                            committed,
                        )
                        .await?
                        {
                            ShellRenderedFileRemovalRecoveryAssessment::Stable => {}
                            ShellRenderedFileRemovalRecoveryAssessment::RestoreMoved => {
                                self.host()
                                    .rename(&file.rollback, &file.destination)
                                    .await
                                    .map_err(|error| {
                                        error.into_anyhow("restoring removed Shell cache file")
                                    })?;
                                changed = true;
                            }
                            ShellRenderedFileRemovalRecoveryAssessment::CleanupRollback => {
                                self.host()
                                    .remove_file(&file.rollback)
                                    .await
                                    .map_err(|error| {
                                        error.into_anyhow("cleaning removed Shell cache rollback")
                                    })?;
                                changed = true;
                            }
                            ShellRenderedFileRemovalRecoveryAssessment::Blocked => {
                                bail!("Shell cache removal changed after recovery approval")
                            }
                        }
                    }
                }
                ActionKindV1::RemoveShellSnapshot {
                    destination,
                    rollback,
                    previous_files,
                    receipts,
                } => {
                    let committed = journal.receipt_committed.contains(&action.action_id);
                    if committed && !shell_removal_receipts_match_missing(&manifest, receipts) {
                        bail!("Shell snapshot removal receipts changed after recovery approval");
                    }
                    if !committed && !shell_removal_receipts_match_previous(&manifest, receipts) {
                        bail!("Shell snapshot removal receipts changed after recovery approval");
                    }
                    changed |= restored_shared_receipts.contains(&action.action_id);
                    match assess_shell_tree_removal_recovery(
                        self.host(),
                        destination,
                        rollback,
                        previous_files,
                        committed,
                    )
                    .await?
                    {
                        ShellTreeRemovalRecoveryAssessment::Stable => {}
                        ShellTreeRemovalRecoveryAssessment::RestoreMoved => {
                            self.host()
                                .rename(rollback, destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("restoring removed Shell snapshot")
                                })?;
                            changed = true;
                        }
                        ShellTreeRemovalRecoveryAssessment::CleanupRollback => {
                            self.host()
                                .remove_dir_all(rollback)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("cleaning removed Shell snapshot rollback")
                                })?;
                            changed = true;
                        }
                        ShellTreeRemovalRecoveryAssessment::Blocked => {
                            bail!("Shell snapshot removal changed after recovery approval")
                        }
                    }
                }
                ActionKindV1::ReconcileShellProfile {
                    files,
                    receipt_transitions,
                    receipt_removals,
                    legacy_targets,
                } => {
                    let committed = journal.receipt_committed.contains(&action.action_id);
                    if committed
                        && !shell_profile_receipts_match_desired(
                            &manifest,
                            receipt_transitions,
                            receipt_removals,
                            legacy_targets,
                        )
                    {
                        bail!("Shell profile receipts changed after recovery approval");
                    }
                    if !committed
                        && !shell_profile_receipts_match_previous(
                            &manifest,
                            receipt_transitions,
                            receipt_removals,
                            legacy_targets,
                        )
                    {
                        bail!("Shell profile receipts changed after recovery approval");
                    }
                    changed |= restored_shared_receipts.contains(&action.action_id);
                    for file in files.iter().rev() {
                        match assess_shell_profile_file_recovery(self.host(), file, committed)
                            .await?
                        {
                            ShellProfileFileRecoveryAssessment::Stable => {}
                            ShellProfileFileRecoveryAssessment::RestoreWholeMoved => {
                                self.host()
                                    .rename(&file.rollback, &file.destination)
                                    .await
                                    .map_err(|error| {
                                        error.into_anyhow("restoring previous Shell profile")
                                    })?;
                                changed = true;
                            }
                            ShellProfileFileRecoveryAssessment::RestoreWholeReplaced => {
                                self.host().remove_file(&file.destination).await.map_err(
                                    |error| error.into_anyhow("removing replacement Shell profile"),
                                )?;
                                self.host()
                                    .rename(&file.rollback, &file.destination)
                                    .await
                                    .map_err(|error| {
                                        error.into_anyhow("restoring previous Shell profile")
                                    })?;
                                changed = true;
                            }
                            ShellProfileFileRecoveryAssessment::RemoveWholeCreated => {
                                self.host().remove_file(&file.destination).await.map_err(
                                    |error| error.into_anyhow("removing created Shell profile"),
                                )?;
                                changed = true;
                            }
                            ShellProfileFileRecoveryAssessment::RestoreSentinel => {
                                restore_shell_profile_sentinel(self.host(), file).await?;
                                changed = true;
                            }
                            ShellProfileFileRecoveryAssessment::CleanupRollback => {
                                self.host()
                                    .remove_file(&file.rollback)
                                    .await
                                    .map_err(|error| {
                                        error.into_anyhow("cleaning Shell profile rollback")
                                    })?;
                                changed = true;
                            }
                            ShellProfileFileRecoveryAssessment::Blocked => {
                                bail!("Shell profile changed after recovery approval")
                            }
                        }
                    }
                }
                ActionKindV1::ReplaceShellRenderedFile {
                    destination,
                    rollback,
                    previous,
                    desired,
                    receipts,
                } => {
                    let committed = journal.receipt_committed.contains(&action.action_id);
                    if committed && !shell_receipts_match_desired(&manifest, receipts) {
                        bail!("Shell rendered-file receipts changed after recovery approval");
                    }
                    if !committed && !shell_receipts_match_previous(&manifest, receipts) {
                        bail!("Shell rendered-file receipts changed after recovery approval");
                    }
                    changed |= restored_shared_receipts.contains(&action.action_id);
                    match assess_shell_rendered_file_recovery(
                        self.host(),
                        destination,
                        rollback,
                        previous.as_ref(),
                        desired,
                        committed,
                    )
                    .await?
                    {
                        ShellRenderedFileRecoveryAssessment::Stable => {}
                        ShellRenderedFileRecoveryAssessment::RestoreMoved => {
                            self.host()
                                .rename(rollback, destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("restoring previous Shell rendered file")
                                })?;
                            changed = true;
                        }
                        ShellRenderedFileRecoveryAssessment::RestoreReplaced => {
                            self.host()
                                .remove_file(destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("removing replacement Shell rendered file")
                                })?;
                            self.host()
                                .rename(rollback, destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("restoring previous Shell rendered file")
                                })?;
                            changed = true;
                        }
                        ShellRenderedFileRecoveryAssessment::RemoveCreated => {
                            self.host()
                                .remove_file(destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("removing created Shell rendered file")
                                })?;
                            changed = true;
                        }
                        ShellRenderedFileRecoveryAssessment::CleanupRollback => {
                            self.host().remove_file(rollback).await.map_err(|error| {
                                error.into_anyhow("cleaning Shell rendered-file rollback")
                            })?;
                            changed = true;
                        }
                        ShellRenderedFileRecoveryAssessment::Blocked => {
                            bail!("Shell rendered file changed after recovery approval")
                        }
                    }
                }
                ActionKindV1::RemoveShellRenderedFile {
                    destination,
                    rollback,
                    previous,
                    receipts,
                } => {
                    let committed = journal.receipt_committed.contains(&action.action_id);
                    if committed && !shell_removal_receipts_match_missing(&manifest, receipts) {
                        bail!(
                            "Shell rendered-file removal receipts changed after recovery approval"
                        );
                    }
                    if !committed && !shell_removal_receipts_match_previous(&manifest, receipts) {
                        bail!(
                            "Shell rendered-file removal receipts changed after recovery approval"
                        );
                    }
                    changed |= restored_shared_receipts.contains(&action.action_id);
                    match assess_shell_rendered_file_removal_recovery(
                        self.host(),
                        destination,
                        rollback,
                        previous,
                        committed,
                    )
                    .await?
                    {
                        ShellRenderedFileRemovalRecoveryAssessment::Stable => {}
                        ShellRenderedFileRemovalRecoveryAssessment::RestoreMoved => {
                            self.host()
                                .rename(rollback, destination)
                                .await
                                .map_err(|error| {
                                    error.into_anyhow("restoring removed Shell rendered file")
                                })?;
                            changed = true;
                        }
                        ShellRenderedFileRemovalRecoveryAssessment::CleanupRollback => {
                            self.host().remove_file(rollback).await.map_err(|error| {
                                error.into_anyhow("cleaning removed Shell rendered-file rollback")
                            })?;
                            changed = true;
                        }
                        ShellRenderedFileRemovalRecoveryAssessment::Blocked => {
                            bail!("Shell rendered-file removal changed after recovery approval")
                        }
                    }
                }
                _ => unreachable!("validated Shell journal action kind"),
            }
            if changed {
                rolled_back_actions.push(action.action_id.clone());
            }
        }
        remove_shell_operation_journal(self.host(), &self.context().shine_dir).await?;
        Ok(ShellRecoveryReportV1 {
            operation_id: journal.action_ir.operation_id,
            rolled_back_actions,
        })
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

fn shell_tree_contract(files: &[(PathBuf, Vec<u8>)]) -> Vec<ShellTreeFileV1> {
    let mut contracts = files
        .iter()
        .map(|(relative_path, bytes)| ShellTreeFileV1 {
            relative_path: relative_path.clone(),
            content_hash: hash_content(bytes),
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
    let mode = match receipt.mode.as_str() {
        "snapshot" => super::ExternalShellMode::Snapshot,
        "live" => super::ExternalShellMode::Live,
        _ => bail!("Shell launcher receipt contains an unsupported mode"),
    };
    Ok(ShellManifestEntry {
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

fn matching_shell_receipt(
    manifest: &ShellManifest,
    target: &str,
    expected: &ShellLauncherReceiptV1,
) -> ReceiptState {
    let Some(entry) = manifest.find(target) else {
        return ReceiptState::Missing;
    };
    if receipt_contract(entry) == *expected {
        ReceiptState::Matching
    } else {
        ReceiptState::Conflict
    }
}

fn shell_shared_receipt_transitions(kind: &ActionKindV1) -> Option<&[ShellReceiptTransitionV1]> {
    match kind {
        ActionKindV1::ReplaceShellSnapshot { receipts, .. }
        | ActionKindV1::ReplaceShellCache { receipts, .. }
        | ActionKindV1::ReplaceShellRenderedFile { receipts, .. } => Some(receipts),
        _ => None,
    }
}

fn shell_receipts_match_previous(
    manifest: &ShellManifest,
    transitions: &[ShellReceiptTransitionV1],
) -> bool {
    transitions
        .iter()
        .all(|transition| match &transition.previous {
            Some(previous) => {
                matching_shell_receipt(manifest, &transition.target, previous)
                    == ReceiptState::Matching
            }
            None => manifest.find(&transition.target).is_none(),
        })
}

fn shell_receipts_match_desired(
    manifest: &ShellManifest,
    transitions: &[ShellReceiptTransitionV1],
) -> bool {
    transitions.iter().all(|transition| {
        matching_shell_receipt(manifest, &transition.target, &transition.desired)
            == ReceiptState::Matching
    })
}

fn shell_removal_receipts_match_previous(
    manifest: &ShellManifest,
    removals: &[ShellReceiptRemovalV1],
) -> bool {
    removals.iter().all(|removal| {
        matching_shell_receipt(manifest, &removal.target, &removal.previous)
            == ReceiptState::Matching
    })
}

fn shell_removal_receipts_match_missing(
    manifest: &ShellManifest,
    removals: &[ShellReceiptRemovalV1],
) -> bool {
    removals
        .iter()
        .all(|removal| manifest.find(&removal.target).is_none())
}

fn shell_profile_receipts_match_previous(
    manifest: &ShellManifest,
    transitions: &[ShellReceiptTransitionV1],
    removals: &[ShellReceiptRemovalV1],
    legacy_targets: &[String],
) -> bool {
    shell_receipts_match_previous(manifest, transitions)
        && shell_removal_receipts_match_previous(manifest, removals)
        && legacy_targets
            .iter()
            .all(|target| manifest.find(target).is_none())
}

fn shell_profile_receipts_match_desired(
    manifest: &ShellManifest,
    transitions: &[ShellReceiptTransitionV1],
    removals: &[ShellReceiptRemovalV1],
    legacy_targets: &[String],
) -> bool {
    shell_receipts_match_desired(manifest, transitions)
        && shell_removal_receipts_match_missing(manifest, removals)
        && legacy_targets
            .iter()
            .all(|target| manifest.find(target).is_none())
}

fn restore_shell_previous_receipts(
    manifest: &mut ShellManifest,
    transitions: &[ShellReceiptTransitionV1],
) -> Result<()> {
    let targets = transitions
        .iter()
        .map(|transition| transition.target.clone())
        .collect::<BTreeSet<_>>();
    let entries = transitions
        .iter()
        .filter_map(|transition| transition.previous.as_deref())
        .map(manifest_entry_from_receipt)
        .collect::<Result<Vec<_>>>()?;
    manifest.replace_targets(&targets, entries);
    Ok(())
}

fn restore_shell_removed_receipts(
    manifest: &mut ShellManifest,
    removals: &[ShellReceiptRemovalV1],
) -> Result<()> {
    let targets = removals
        .iter()
        .map(|removal| removal.target.clone())
        .collect::<BTreeSet<_>>();
    let entries = removals
        .iter()
        .map(|removal| manifest_entry_from_receipt(&removal.previous))
        .collect::<Result<Vec<_>>>()?;
    manifest.replace_targets(&targets, entries);
    Ok(())
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
    if actual == expected {
        return Ok(ShellTreeObservation::Exact);
    }
    if allow_safe_partial {
        let expected = expected
            .iter()
            .map(|file| (&file.relative_path, file.content_hash))
            .collect::<BTreeMap<_, _>>();
        if actual
            .iter()
            .all(|file| expected.get(&file.relative_path).copied() == Some(file.content_hash))
        {
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

async fn load_shell_operation_journal(
    host: &impl FileSystemObservationHost,
    shine_dir: &Path,
) -> Result<Option<(ShellOperationJournalV1, Vec<u8>)>> {
    let path = shine_dir.join(SHELL_OPERATION_JOURNAL_FILE);
    let bytes = match host.read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.is_not_found() => return Ok(None),
        Err(error) => return Err(error.into_anyhow("reading Shell operation journal")),
    };
    let journal: ShellOperationJournalV1 =
        toml::from_slice(&bytes).context("failed to parse Shell operation journal")?;
    journal.validate()?;
    Ok(Some((journal, bytes)))
}

async fn save_shell_operation_journal(
    host: &impl FileSystemHost,
    shine_dir: &Path,
    journal: &ShellOperationJournalV1,
) -> Result<()> {
    journal.validate()?;
    let bytes = toml::to_string_pretty(journal).context("serializing Shell operation journal")?;
    host.write_atomic(
        &shine_dir.join(SHELL_OPERATION_JOURNAL_FILE),
        bytes.as_bytes(),
    )
    .await
    .map_err(|error| error.into_anyhow("writing Shell operation journal"))
}

async fn remove_shell_operation_journal(
    host: &impl FileSystemHost,
    shine_dir: &Path,
) -> Result<()> {
    match host
        .remove_file(&shine_dir.join(SHELL_OPERATION_JOURNAL_FILE))
        .await
    {
        Ok(()) => Ok(()),
        Err(error) if error.is_not_found() => Ok(()),
        Err(error) => Err(error.into_anyhow("removing Shell operation journal")),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::InMemoryHost;

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
        }];
        let desired_files = vec![ShellTreeFileV1 {
            relative_path: PathBuf::from("demo.sh"),
            content_hash: hash_content(b"desired"),
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
