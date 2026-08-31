//! Transactional creation and explicit recovery for Shell launchers.

use super::launcher::{
    PreparedLauncherResource, apply_prepared_launcher_resource, prepare_launcher_resources,
};
use super::shell::{ShellManifest, ShellManifestEntry, load_shell_manifest_with_host};
use super::{
    CoreRuntime, FileKind, FileSystemHost, FileSystemObservationHost, LinkSpec,
    PrivilegedFileSystemHost, RuntimeContext,
};
use crate::action::{
    ACTION_IR_SCHEMA_VERSION, ActionIrV1, ActionKindV1, DeclarativeActionV1,
    ShellLauncherReceiptV1, ShellLauncherResourceV1,
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
        if self
            .action_ir
            .actions
            .iter()
            .any(|action| !matches!(action.kind, ActionKindV1::CreateShellLauncher { .. }))
        {
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
            let ActionKindV1::CreateShellLauncher { receipt, resources } = &action.kind else {
                unreachable!("validated Shell journal action kind")
            };
            let receipt_state = matching_shell_receipt(&manifest, &action.target, receipt);
            if receipt_state == ReceiptState::Conflict {
                blocked = true;
                steps.push(
                    PlanStepV1::new(
                        &action.target,
                        Some(&action.resource),
                        PlanActionV1::Blocked,
                    )
                    .with_diagnostic_code("shell_recovery_receipt_conflict"),
                );
                continue;
            }
            let mut action_changed = false;
            let mut action_blocked = false;
            for (index, resource) in resources.iter().enumerate() {
                let observation = observe_launcher_resource(self.host(), resource).await?;
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
            blocked |= action_blocked;
            let (plan_action, code) = if action_blocked {
                (PlanActionV1::Blocked, "shell_recovery_launcher_changed")
            } else if receipt_state == ReceiptState::Matching {
                (
                    PlanActionV1::None,
                    "shell_recovery_receipt_already_committed",
                )
            } else if action_changed {
                (
                    PlanActionV1::Remove,
                    "shell_recovery_remove_created_launcher",
                )
            } else {
                (PlanActionV1::None, "shell_recovery_launcher_absent")
            };
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
    pub(crate) async fn create_shell_launchers_approved(
        &self,
        creations: &[ShellLauncherCreation<'_>],
        approval: &PlanApprovalV1,
    ) -> Result<Option<ShellOperationExecutionV1>> {
        if creations.is_empty() {
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
            prepared.push(resources);
        }
        let action_ir = ActionIrV1::new(
            format!(
                "shell-launcher-create:{}",
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
        for (action, resources) in journal.action_ir.actions.clone().iter().zip(&prepared) {
            for resource in resources {
                apply_prepared_launcher_resource(self.host(), resource).await?;
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
            let ActionKindV1::CreateShellLauncher { receipt, resources } = &action.kind else {
                unreachable!("validated Shell journal action kind")
            };
            if matching_shell_receipt(&manifest, &action.target, receipt) != ReceiptState::Matching
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
            let ActionKindV1::CreateShellLauncher { receipt, resources } = &action.kind else {
                unreachable!("validated Shell journal action kind")
            };
            match matching_shell_receipt(&manifest, &action.target, receipt) {
                ReceiptState::Matching => continue,
                ReceiptState::Conflict => bail!("Shell receipt changed after recovery approval"),
                ReceiptState::Missing => {}
            }
            let mut changed = false;
            for resource in resources.iter().rev() {
                match observe_launcher_resource(self.host(), resource).await? {
                    LauncherObservation::Missing => {}
                    LauncherObservation::Exact => {
                        self.host()
                            .remove_file(resource.destination())
                            .await
                            .map_err(|error| error.into_anyhow("rolling back Shell launcher"))?;
                        changed = true;
                    }
                    LauncherObservation::Changed => {
                        bail!("Shell launcher changed after recovery approval")
                    }
                }
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
