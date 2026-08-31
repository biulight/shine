//! Transactional creation/update and explicit recovery for Shell launchers.

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
    ShellLauncherReceiptV1, ShellLauncherResourceV1, ShellLauncherUpdateResourceV1,
    managed_file_rollback_path,
};
use crate::install::hash_content;
use crate::plan::{
    FilesystemAccessV1, PLAN_APPROVAL_SCHEMA_VERSION, PermissionSetV1, PermissionV1, PlanActionV1,
    PlanApprovalV1, PlanInputsV1, PlanOperationV1, PlanStepV1, PlanV1, SnapshotDigestV1,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

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

enum PreparedShellLauncherAction {
    Create(Vec<PreparedLauncherResource>),
    Update(Vec<PreparedShellLauncherUpdateResource>),
}

struct PreparedShellLauncherUpdateResource {
    previous: ShellLauncherResourceV1,
    desired: PreparedLauncherResource,
    rollback: std::path::PathBuf,
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
}

impl ShellOperationJournalV1 {
    fn new(action_ir: ActionIrV1, approval: PlanApprovalV1) -> Self {
        Self {
            schema_version: SHELL_OPERATION_JOURNAL_SCHEMA_VERSION,
            action_ir,
            approval,
            applied: Vec::new(),
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
                ActionKindV1::CreateShellLauncher { .. } | ActionKindV1::UpdateShellLauncher { .. }
            )
        }) {
            bail!("Shell operation journal contains a non-launcher action");
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
        let manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
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
        creations: &[ShellLauncherCreation<'_>],
        updates: &[ShellLauncherUpdate<'_>],
        approval: &PlanApprovalV1,
    ) -> Result<Option<ShellOperationExecutionV1>> {
        if creations.is_empty() && updates.is_empty() {
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
            prepared.push(PreparedShellLauncherAction::Create(resources));
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
            prepared.push(PreparedShellLauncherAction::Update(prepared_resources));
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
                PreparedShellLauncherAction::Create(resources) => {
                    for resource in resources {
                        apply_prepared_launcher_resource(self.host(), resource).await?;
                    }
                }
                PreparedShellLauncherAction::Update(resources) => {
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
            }
            journal.applied.push(action.action_id.clone());
            save_shell_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        }
        Ok(Some(ShellOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
            _operation_guard: operation_guard,
        }))
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
        let manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let mut rolled_back_actions = Vec::new();
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReceiptState {
    Missing,
    Matching,
    Conflict,
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
