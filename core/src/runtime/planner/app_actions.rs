//! Approved static App Action construction; resource handlers keep observation-only capability.
mod copy;
mod file_relocation;
mod json_merge;
mod json_relocation;

use super::*;

/// Per-file inputs selected by the existing approved factory. This is not execution authority.
struct AppFileActionInput<'a> {
    request: &'a AppPlanRequest,
    plan: &'a PlanV1,
    manifest: &'a AppManifest,
    file: &'a AppFile,
    direct: Option<&'a AppEntry>,
    destination: PathBuf,
    target: String,
    resource: String,
    action_identity: String,
    desired: Vec<u8>,
    desired_hash: u64,
}

impl<'a> AppFileActionInput<'a> {
    fn relocation_entry(&self, relocation: StaticAppRelocation) -> Option<&'a AppEntry> {
        self.direct.filter(|entry| {
            static_app_relocation(
                self.request.operation,
                self.request.force,
                self.file,
                entry,
                &self.destination,
            ) == Some(relocation)
                && has_app_file_step(
                    self.plan,
                    &self.target,
                    &self.resource,
                    PlanActionV1::Update,
                    Some(relocation.step_kind()),
                )
        })
    }
}

// Ordinary create/update steps deliberately impose no step-kind restriction.
fn has_app_file_step(
    plan: &PlanV1,
    target: &str,
    resource: &str,
    action: PlanActionV1,
    kind: Option<PlanStepKindV1>,
) -> bool {
    plan.steps.iter().any(|step| {
        step.target == target
            && step.resource.as_deref() == Some(resource)
            && step.action == action
            && kind
                .as_ref()
                .is_none_or(|kind| step.kind.as_ref() == Some(kind))
    })
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    /// Emit exact static actions only after the corresponding Plan has been freshly approved.
    /// Managed bytes remain outside the Action IR and are checked again by the executor.
    pub(super) async fn approved_app_file_action_irs(
        &self,
        request: &AppPlanRequest,
        plan: &PlanV1,
        approval: &PlanApprovalV1,
    ) -> Result<Vec<ActionIrV1>> {
        approval.validate(plan)?;
        if request.operation == LifecycleOperation::Uninstall {
            return self.approved_app_remove_action_irs(request, plan).await;
        }
        let fingerprint = plan.fingerprint()?.as_hex();
        let operation = match request.operation {
            LifecycleOperation::Install => "install",
            LifecycleOperation::Upgrade => "upgrade",
            _ => return Ok(Vec::new()),
        };
        let categories = self.app_categories(request.target.as_deref())?;
        let (manifest, _) = load_app_manifest(self.host(), &self.context().shine_dir).await?;
        let mut actions = Vec::new();
        for category in categories {
            for file in &category.files {
                if file.generator.is_some() {
                    continue;
                }
                let source = logical_app_source(&category, file);
                let destination = self.app_destination(&category, file)?;
                let resource = file.source_rel.display().to_string();
                let target = format!("app/{}", category.name);
                let desired = crate::install::transforms::apply(
                    &file.transforms,
                    self.app_source_bytes(&category.name, file)?,
                    &self.context().env,
                )?;
                let action_identity = format!("{target}/{resource}");
                let desired_hash = desired_app_hash(file, &desired)?;
                let direct = manifest.find_by_source(&source);
                let ir_id = format!("app-{operation}:{fingerprint}:{action_identity}");
                let input = AppFileActionInput {
                    request,
                    plan,
                    manifest: &manifest,
                    file,
                    direct,
                    destination,
                    target,
                    resource,
                    action_identity,
                    desired,
                    desired_hash,
                };
                let action = if let Some(entry) = input.relocation_entry(StaticAppRelocation::Json)
                {
                    Some(
                        self.approved_app_json_relocation_action(input, entry)
                            .await?,
                    )
                } else if let Some(entry) = input.relocation_entry(StaticAppRelocation::File) {
                    Some(
                        self.approved_app_file_relocation_action(input, entry)
                            .await?,
                    )
                } else if let crate::install::AppInstallStrategy::JsonMerge { managed_keys } =
                    &file.install_strategy
                {
                    self.approved_app_json_merge_action(input, managed_keys)
                        .await?
                } else {
                    self.approved_app_copy_action(input).await?
                };
                if let Some(action) = action {
                    actions.push(ActionIrV1::new(ir_id, vec![action]));
                }
            }
        }
        if request.operation == LifecycleOperation::Upgrade && request.prune_stale {
            actions.extend(
                self.approved_app_stale_remove_action_irs(request, plan)
                    .await?,
            );
        }
        Ok(actions)
    }
}
