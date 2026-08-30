use super::{
    CoreRuntime, FileSystemHost, FileSystemObservationHost, PrivilegedFileSystemHost,
    RuntimeContext,
};
use crate::action::{ACTION_IR_SCHEMA_VERSION, ActionIrV1, ActionKindV1, RollbackSupportV1};
use crate::install::manifest::APP_MANIFEST_SCHEMA_VERSION;
use crate::install::{AppInstallStrategy, AppManifest, hash_content};
use crate::plan::{
    FilesystemAccessV1, PLAN_APPROVAL_SCHEMA_VERSION, PermissionSetV1, PermissionV1, PlanActionV1,
    PlanApprovalV1, PlanInputsV1, PlanOperationV1, PlanStepV1, PlanV1, SnapshotDigestV1,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const APP_OPERATION_JOURNAL_FILE: &str = "app-operation-journal.toml";
const APP_OPERATION_JOURNAL_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppOperationExecutionV1 {
    pub operation_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppRecoveryReportV1 {
    pub operation_id: String,
    pub rolled_back_actions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct AppOperationJournalV1 {
    schema_version: u32,
    action_ir: ActionIrV1,
    approval: PlanApprovalV1,
    actions: Vec<JournalActionV1>,
}

impl AppOperationJournalV1 {
    fn new(action_ir: ActionIrV1, approval: PlanApprovalV1) -> Self {
        let actions = action_ir
            .actions
            .iter()
            .map(|action| JournalActionV1 {
                action_id: action.action_id.clone(),
                state: JournalActionStateV1::Prepared,
            })
            .collect();
        Self {
            schema_version: APP_OPERATION_JOURNAL_SCHEMA_VERSION,
            action_ir,
            approval,
            actions,
        }
    }

    fn validate(&self) -> Result<()> {
        if self.schema_version != APP_OPERATION_JOURNAL_SCHEMA_VERSION {
            bail!(
                "app operation journal schema version {} is newer than this Shine supports ({APP_OPERATION_JOURNAL_SCHEMA_VERSION})",
                self.schema_version
            );
        }
        self.action_ir.validate()?;
        if self.approval.schema_version != PLAN_APPROVAL_SCHEMA_VERSION {
            bail!(
                "unsupported Plan approval schema version {} in App operation journal",
                self.approval.schema_version
            );
        }
        if self.action_ir.schema_version != ACTION_IR_SCHEMA_VERSION {
            bail!("unsupported action IR in App operation journal");
        }
        if self.actions.len() != self.action_ir.actions.len()
            || self
                .actions
                .iter()
                .zip(&self.action_ir.actions)
                .any(|(journal, action)| journal.action_id != action.action_id)
        {
            bail!("App operation journal action state does not match its action IR");
        }
        Ok(())
    }

    fn mark_applied(&mut self, action_id: &str) -> Result<()> {
        let action = self
            .actions
            .iter_mut()
            .find(|action| action.action_id == action_id)
            .with_context(|| format!("App operation journal action not found: {action_id}"))?;
        action.state = JournalActionStateV1::Applied;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct JournalActionV1 {
    action_id: String,
    state: JournalActionStateV1,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum JournalActionStateV1 {
    Prepared,
    Applied,
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(crate) async fn app_operation_journal_bytes(&self) -> Result<Option<Vec<u8>>> {
        Ok(
            load_app_operation_journal(self.host(), &self.context().shine_dir)
                .await?
                .map(|(_, bytes)| bytes),
        )
    }

    /// Plan an explicit rollback of an interrupted App managed-file action.
    /// Recovery is never an implicit side effect of ordinary planning.
    pub async fn plan_app_operation_recovery(&self) -> Result<PlanV1> {
        let (journal, journal_bytes) =
            load_app_operation_journal(self.host(), &self.context().shine_dir)
                .await?
                .context("no interrupted App operation is available for recovery")?;
        let (manifest, manifest_bytes) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        let mut state = SnapshotDigestV1::builder("state:app-recovery");
        state.add_observation("operation", PlanOperationV1::AppRecovery.as_str())?;
        state.add_observation("journal", &journal_bytes)?;
        state.add_observation(
            "app-manifest",
            manifest_bytes.as_deref().unwrap_or(b"missing"),
        )?;
        let mut steps = Vec::new();
        let mut blocked = false;
        let mut required = PermissionSetV1::new([PermissionV1::Filesystem {
            access: FilesystemAccessV1::Remove,
            path: review_path(
                self.context(),
                &self.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            ),
        }]);

        for action in &journal.action_ir.actions {
            match &action.kind {
                ActionKindV1::CreateManagedFile {
                    destination,
                    desired_hash,
                } => {
                    let current = read_optional(self.host(), destination).await?;
                    state.add_observation(
                        format!("destination:{}", action.action_id),
                        current
                            .as_deref()
                            .map(|bytes| format!("present:{}", hash_content(bytes)))
                            .unwrap_or_else(|| "missing".to_string()),
                    )?;
                    if matching_app_receipt(&manifest, action) {
                        steps.push(
                            PlanStepV1::new(
                                &action.target,
                                Some(&action.resource),
                                PlanActionV1::None,
                            )
                            .with_diagnostic_code("app_recovery_receipt_already_committed"),
                        );
                        continue;
                    }
                    let (plan_action, code) = match current.as_deref() {
                        None => (PlanActionV1::None, "app_recovery_resource_absent"),
                        Some(bytes) if hash_content(bytes) == *desired_hash => {
                            required.insert(PermissionV1::Filesystem {
                                access: FilesystemAccessV1::Remove,
                                path: review_path(self.context(), destination),
                            });
                            (PlanActionV1::Remove, "app_recovery_remove_created_file")
                        }
                        Some(_) => {
                            blocked = true;
                            (PlanActionV1::Blocked, "app_recovery_user_modified")
                        }
                    };
                    steps.push(
                        PlanStepV1::new(&action.target, Some(&action.resource), plan_action)
                            .with_diagnostic_code(code),
                    );
                }
                ActionKindV1::OpaqueExecution { .. } => {
                    blocked = true;
                    steps.push(
                        PlanStepV1::new(
                            &action.target,
                            Some(&action.resource),
                            PlanActionV1::Blocked,
                        )
                        .with_diagnostic_code("app_recovery_opaque_action"),
                    );
                }
            }
        }

        steps.push(
            PlanStepV1::new(
                "app",
                Some("operation-journal"),
                if blocked {
                    PlanActionV1::Preserve
                } else {
                    PlanActionV1::Remove
                },
            )
            .with_diagnostic_code(if blocked {
                "app_recovery_journal_preserved"
            } else {
                "app_recovery_clear_journal"
            }),
        );

        let preset = self.presets().digest_v1()?;
        Ok(PlanV1::new(
            PlanOperationV1::AppRecovery,
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
    /// Execute the first Phase 4 action slice: create one previously absent
    /// App managed file and leave the journal active until its owner persists
    /// the corresponding receipt and commits the operation.
    pub async fn execute_app_managed_file_creation_approved(
        &self,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
        action_ir: ActionIrV1,
        content: &[u8],
    ) -> Result<AppOperationExecutionV1> {
        approval.validate(plan)?;
        if plan.operation != PlanOperationV1::Install {
            bail!("App managed-file creation requires an install Plan");
        }
        action_ir.validate()?;
        let requirements =
            action_ir.permission_requirements(|path| review_path(self.context(), path));
        if !requirements.uncomputable_codes.is_empty() {
            bail!("action permissions are not fully computable");
        }
        for permission in requirements.required.iter() {
            if !approval.approved_permissions.contains(permission) {
                bail!("action permission was not included in the approved security Plan");
            }
        }
        let [action] = action_ir.actions.as_slice() else {
            bail!("the App managed-file creation slice accepts exactly one action");
        };
        if !plan.steps.iter().any(|step| {
            step.target == action.target
                && step.resource.as_deref() == Some(action.resource.as_str())
                && step.action == PlanActionV1::Create
        }) {
            bail!("App managed-file action was not described by the approved security Plan");
        }
        let ActionKindV1::CreateManagedFile {
            destination,
            desired_hash,
        } = &action.kind
        else {
            bail!("the App managed-file creation slice accepts only declarative file creation");
        };
        if action.rollback != RollbackSupportV1::RemoveCreatedIfUnchanged {
            bail!("the App managed-file creation action is not safely reversible");
        }
        if hash_content(content) != *desired_hash {
            bail!("managed-file content does not match the action IR identity");
        }
        let action_id = action.action_id.clone();
        let destination = destination.clone();

        let _guard = self.host().acquire_privileged_operation().await?;
        if load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .is_some()
        {
            bail!("an interrupted App operation must be recovered before starting another one");
        }
        if read_optional(self.host(), &destination).await?.is_some() {
            bail!("managed-file creation requires an absent destination");
        }

        let mut journal = AppOperationJournalV1::new(action_ir, approval.clone());
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;
        self.host()
            .write_atomic(&destination, content)
            .await
            .map_err(|error| error.into_anyhow("failed to create managed App file"))?;
        journal.mark_applied(&action_id)?;
        save_app_operation_journal(self.host(), &self.context().shine_dir, &journal).await?;

        Ok(AppOperationExecutionV1 {
            operation_id: journal.action_ir.operation_id,
        })
    }

    /// Clear a completed journal only after the caller has durably persisted
    /// the App receipt that owns the created file.
    pub async fn commit_app_managed_file_operation(&self, operation_id: &str) -> Result<()> {
        let _guard = self.host().acquire_privileged_operation().await?;
        let (journal, _) = load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .context("no App operation journal is available to commit")?;
        if journal.action_ir.operation_id != operation_id {
            bail!("App operation journal identity changed before commit");
        }
        if journal
            .actions
            .iter()
            .any(|action| action.state != JournalActionStateV1::Applied)
        {
            bail!("App operation journal cannot commit before every action is applied");
        }
        let (manifest, _) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        if journal
            .action_ir
            .actions
            .iter()
            .any(|action| !matching_app_receipt(&manifest, action))
        {
            bail!("App operation journal cannot commit before its matching manifest receipt");
        }
        remove_app_operation_journal(self.host(), &self.context().shine_dir).await
    }

    /// Roll back an interrupted creation only after reviewing an exact
    /// recovery Plan. A changed destination blocks before any removal.
    pub async fn recover_app_operation_approved(
        &self,
        approval: &PlanApprovalV1,
    ) -> Result<AppRecoveryReportV1> {
        approval.validate(&self.plan_app_operation_recovery().await?)?;
        let _guard = self.host().acquire_privileged_operation().await?;
        approval.validate(&self.plan_app_operation_recovery().await?)?;
        let (journal, _) = load_app_operation_journal(self.host(), &self.context().shine_dir)
            .await?
            .context("no interrupted App operation is available for recovery")?;
        let (manifest, _) =
            load_app_manifest_receipts(self.host(), &self.context().shine_dir).await?;
        let mut rolled_back_actions = Vec::new();
        for action in journal.action_ir.actions.iter().rev() {
            if matching_app_receipt(&manifest, action) {
                continue;
            }
            let ActionKindV1::CreateManagedFile {
                destination,
                desired_hash,
            } = &action.kind
            else {
                bail!("opaque App actions cannot be rolled back automatically");
            };
            match read_optional(self.host(), destination).await? {
                None => {}
                Some(bytes) if hash_content(&bytes) == *desired_hash => {
                    self.host()
                        .remove_file(destination)
                        .await
                        .map_err(|error| {
                            error.into_anyhow("failed to roll back managed App file")
                        })?;
                }
                Some(_) => bail!(
                    "managed App file changed after the interrupted operation; recovery preserved it"
                ),
            }
            rolled_back_actions.push(action.action_id.clone());
        }
        remove_app_operation_journal(self.host(), &self.context().shine_dir).await?;
        Ok(AppRecoveryReportV1 {
            operation_id: journal.action_ir.operation_id,
            rolled_back_actions,
        })
    }
}

async fn load_app_manifest_receipts(
    host: &impl FileSystemObservationHost,
    shine_dir: &Path,
) -> Result<(AppManifest, Option<Vec<u8>>)> {
    let bytes = read_optional(host, &shine_dir.join("app-manifest.toml")).await?;
    let mut manifest: AppManifest = bytes
        .as_deref()
        .map(toml::from_slice)
        .transpose()
        .context("failed to parse app manifest")?
        .unwrap_or_default();
    match manifest.schema_version {
        0 => manifest.schema_version = APP_MANIFEST_SCHEMA_VERSION,
        APP_MANIFEST_SCHEMA_VERSION => {}
        version => bail!(
            "app manifest schema version {version} is newer than this Shine supports ({APP_MANIFEST_SCHEMA_VERSION})"
        ),
    }
    Ok((manifest, bytes))
}

fn matching_app_receipt(
    manifest: &AppManifest,
    action: &crate::action::DeclarativeActionV1,
) -> bool {
    let ActionKindV1::CreateManagedFile {
        destination,
        desired_hash,
    } = &action.kind
    else {
        return false;
    };
    let source = format!(
        "{}/{}",
        action.target.trim_end_matches('/'),
        action.resource.trim_start_matches('/')
    );
    manifest.find_by_source(&source).is_some_and(|entry| {
        entry.destination == *destination
            && entry.content_hash == *desired_hash
            && entry.backup.is_none()
            && entry.install_strategy == AppInstallStrategy::Copy
            && !entry.requires_admin
    })
}

async fn load_app_operation_journal(
    host: &impl FileSystemObservationHost,
    shine_dir: &Path,
) -> Result<Option<(AppOperationJournalV1, Vec<u8>)>> {
    let path = shine_dir.join(APP_OPERATION_JOURNAL_FILE);
    let bytes = match host.read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.is_not_found() => return Ok(None),
        Err(error) => return Err(error.into_anyhow("failed to read App operation journal")),
    };
    let journal: AppOperationJournalV1 =
        toml::from_slice(&bytes).context("failed to parse App operation journal")?;
    journal.validate()?;
    Ok(Some((journal, bytes)))
}

async fn save_app_operation_journal(
    host: &impl FileSystemHost,
    shine_dir: &Path,
    journal: &AppOperationJournalV1,
) -> Result<()> {
    journal.validate()?;
    let bytes =
        toml::to_string_pretty(journal).context("failed to serialize App operation journal")?;
    host.write_atomic(
        &shine_dir.join(APP_OPERATION_JOURNAL_FILE),
        bytes.as_bytes(),
    )
    .await
    .map_err(|error| error.into_anyhow("failed to write App operation journal"))
}

async fn remove_app_operation_journal(host: &impl FileSystemHost, shine_dir: &Path) -> Result<()> {
    match host
        .remove_file(&shine_dir.join(APP_OPERATION_JOURNAL_FILE))
        .await
    {
        Ok(()) => Ok(()),
        Err(error) if error.is_not_found() => Ok(()),
        Err(error) => Err(error.into_anyhow("failed to remove App operation journal")),
    }
}

async fn read_optional(
    host: &impl FileSystemObservationHost,
    path: &Path,
) -> Result<Option<Vec<u8>>> {
    match host.read(path).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.is_not_found() => Ok(None),
        Err(error) => Err(error.into_anyhow("failed to observe App recovery resource")),
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
    use crate::action::DeclarativeActionV1;
    use crate::install::AppEntry;
    use crate::plan::PlanStepV1;
    use crate::runtime::{InMemoryHost, PresetSnapshot, PresetSourceKind, RuntimePlatform};
    use std::path::PathBuf;

    fn runtime() -> CoreRuntime<InMemoryHost> {
        let home = PathBuf::from("/home/test");
        let shine = home.join(".shine");
        let context = RuntimeContext::isolated(
            home,
            shine.clone(),
            shine.join("presets"),
            shine.join("bin"),
            RuntimePlatform::Linux,
        );
        CoreRuntime::new(
            InMemoryHost::new(),
            context,
            PresetSnapshot::builder(PresetSourceKind::External).build(),
        )
    }

    fn action_ir(runtime: &CoreRuntime<InMemoryHost>, content: &[u8]) -> ActionIrV1 {
        ActionIrV1::new(
            "operation-1",
            vec![DeclarativeActionV1::create_managed_file(
                "action-1",
                "app/demo",
                "config",
                runtime.context().home_dir.join(".config/demo/config"),
                hash_content(content),
            )],
        )
    }

    fn approved_install_plan(
        runtime: &CoreRuntime<InMemoryHost>,
        ir: &ActionIrV1,
    ) -> (PlanV1, PlanApprovalV1) {
        let required = ir
            .permission_requirements(|path| review_path(runtime.context(), path))
            .required;
        let plan = PlanV1::new(
            PlanOperationV1::Install,
            PlanInputsV1 {
                preset: runtime.presets().digest_v1().unwrap(),
                state: SnapshotDigestV1::builder("test-state").finish(),
            },
            vec![PlanStepV1::new(
                "app/demo",
                Some("config"),
                PlanActionV1::Create,
            )],
            required.clone(),
            &required,
            std::iter::empty::<String>(),
        );
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        (plan, approval)
    }

    async fn save_matching_receipt(runtime: &CoreRuntime<InMemoryHost>, content: &[u8]) {
        AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: vec![AppEntry {
                source: "app/demo/config".to_string(),
                destination: runtime.context().home_dir.join(".config/demo/config"),
                backup: None,
                content_hash: hash_content(content),
                install_strategy: AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
        }
        .save(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn managed_file_creation_stays_journaled_until_receipt_commit() {
        let runtime = runtime();
        let content = b"managed-content";
        let ir = action_ir(&runtime, content);
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        let execution = runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await
            .unwrap();
        let journal_bytes = runtime
            .host()
            .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
            .await
            .unwrap();
        assert!(
            !String::from_utf8(journal_bytes)
                .unwrap()
                .contains("managed-content")
        );
        let error = runtime
            .commit_app_managed_file_operation(&execution.operation_id)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("matching manifest receipt"));
        assert!(
            runtime
                .host()
                .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
                .await
                .is_ok()
        );
        save_matching_receipt(&runtime, content).await;
        runtime
            .commit_app_managed_file_operation(&execution.operation_id)
            .await
            .unwrap();
        assert!(
            runtime
                .host()
                .read(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
                .await
                .is_err()
        );
        assert_eq!(
            runtime
                .host()
                .read(&runtime.context().home_dir.join(".config/demo/config"))
                .await
                .unwrap(),
            content
        );
    }

    #[tokio::test]
    async fn interrupted_creation_rolls_back_only_after_a_fresh_recovery_plan() {
        let runtime = runtime();
        let content = b"managed-content";
        let ir = action_ir(&runtime, content);
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        assert!(
            runtime
                .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
                .await
                .is_err()
        );
        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert_eq!(recovery_plan.operation, PlanOperationV1::AppRecovery);
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        let recovered = runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert_eq!(recovered.rolled_back_actions, vec!["action-1"]);
        assert!(
            runtime
                .host()
                .read(&runtime.context().home_dir.join(".config/demo/config"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn recovery_preserves_a_created_file_after_its_receipt_is_durable() {
        let runtime = runtime();
        let content = b"managed-content";
        let ir = action_ir(&runtime, content);
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await
            .unwrap();
        save_matching_receipt(&runtime, content).await;

        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(recovery_plan.steps.iter().any(|step| {
            step.diagnostic_codes
                .contains(&"app_recovery_receipt_already_committed".to_string())
                && step.action == PlanActionV1::None
        }));
        assert!(recovery_plan.steps.iter().any(|step| {
            step.target == "app"
                && step.resource.as_deref() == Some("operation-journal")
                && step.action == PlanActionV1::Remove
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_clear_journal".to_string())
        }));
        let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
        let recovered = runtime
            .recover_app_operation_approved(&recovery_approval)
            .await
            .unwrap();
        assert!(recovered.rolled_back_actions.is_empty());
        assert_eq!(
            runtime
                .host()
                .read(&runtime.context().home_dir.join(".config/demo/config"))
                .await
                .unwrap(),
            content
        );
    }

    #[tokio::test]
    async fn recovery_plan_blocks_after_the_created_file_is_modified() {
        let runtime = runtime();
        let content = b"managed-content";
        let ir = action_ir(&runtime, content);
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let _ = runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await;
        let destination = runtime.context().home_dir.join(".config/demo/config");
        runtime
            .host()
            .put_file(&destination, b"user-change".to_vec());
        let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
        assert!(!recovery_plan.is_ready());
        assert!(recovery_plan.steps.iter().any(|step| {
            step.target == "app"
                && step.resource.as_deref() == Some("operation-journal")
                && step.action == PlanActionV1::Preserve
                && step
                    .diagnostic_codes
                    .contains(&"app_recovery_journal_preserved".to_string())
        }));
        assert_eq!(
            PlanApprovalV1::for_reviewed_plan(&recovery_plan),
            Err(crate::plan::PlanApprovalError::PlanNotReady)
        );
        assert_eq!(
            runtime.host().read(&destination).await.unwrap(),
            b"user-change"
        );
    }

    #[tokio::test]
    async fn future_journal_schema_fails_before_recovery_mutation() {
        let runtime = runtime();
        let content = b"managed-content";
        let ir = action_ir(&runtime, content);
        let (plan, approval) = approved_install_plan(&runtime, &ir);
        runtime.host().fail_write_after(
            runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE),
            1,
        );
        let _ = runtime
            .execute_app_managed_file_creation_approved(&plan, &approval, ir, content)
            .await;
        let journal_path = runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE);
        let journal = String::from_utf8(runtime.host().read(&journal_path).await.unwrap()).unwrap();
        runtime.host().put_file(
            &journal_path,
            journal
                .replacen("schema_version = 1", "schema_version = 99", 1)
                .into_bytes(),
        );
        assert!(runtime.plan_app_operation_recovery().await.is_err());
        assert_eq!(
            runtime
                .host()
                .read(&runtime.context().home_dir.join(".config/demo/config"))
                .await
                .unwrap(),
            content
        );
    }
}
