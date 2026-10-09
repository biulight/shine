//! Code boundaries.

use super::*;

pub(super) fn finish_plan<H>(
    runtime: &CoreRuntime<H>,
    operation: LifecycleOperation,
    state: StateCapture,
    permissions: PermissionAccumulator,
    steps: Vec<PlanStepV1>,
) -> Result<PlanV1> {
    finish_specialized_plan(runtime, operation.into(), state, permissions, steps)
}

pub(super) fn finish_specialized_plan<H>(
    runtime: &CoreRuntime<H>,
    operation: PlanOperationV1,
    mut state: StateCapture,
    permissions: PermissionAccumulator,
    steps: Vec<PlanStepV1>,
) -> Result<PlanV1> {
    let code_entries = std::mem::take(&mut state.code_entries);
    let filesystem_review = permissions.review_groups();
    let (required, declared, author, uncomputable) = permissions.finish();
    let mut plan = PlanV1::new(
        operation,
        PlanInputsV1 {
            preset: runtime.presets().digest_v1()?,
            state: state.finish(),
        },
        steps,
        required,
        &declared,
        uncomputable,
    );
    plan.filesystem_review = filesystem_review;
    plan.author_capabilities = author;
    attach_code_boundaries(runtime, &mut plan, code_entries)?;
    Ok(plan)
}

pub(super) fn attach_code_boundaries<H>(
    runtime: &CoreRuntime<H>,
    plan: &mut PlanV1,
    code_entries: BTreeMap<(String, CodeEntryKindV2), CodeTargetRoleV2>,
) -> Result<()> {
    let mut boundaries = BTreeMap::<(String, CodeEntryKindV2), CodeBoundaryV2>::new();
    for ((target, entry_kind), target_role) in code_entries {
        let capability = match entry_kind {
            CodeEntryKindV2::AppGenerator => TrustCapabilityV1::AppGenerator,
            CodeEntryKindV2::AppHook => TrustCapabilityV1::AppHook,
            CodeEntryKindV2::AppArtifact => TrustCapabilityV1::AppArtifact,
            _ => unreachable!("only App entry semantics are recorded here"),
        };
        let (source, trust) = code_boundary_trust(runtime, &target, capability)?;
        boundaries.insert(
            (target.clone(), entry_kind),
            CodeBoundaryV2 {
                target,
                entry_kind,
                timing: CodeTimingV2::ExecuteNow,
                source,
                trust,
                unisolated: true,
                target_role,
                shared_resource: None,
            },
        );
    }
    for step in &plan.steps {
        if step.action == PlanActionV1::None {
            continue;
        }
        let resource = step.resource.as_deref().unwrap_or_default();
        let classified = if step.target.starts_with("shell/")
            && step.target.split('/').count() == 3
            && plan.operation != PlanOperationV1::Uninstall
        {
            Some((
                CodeEntryKindV2::ShellCommand,
                TrustCapabilityV1::ShellCommand,
            ))
        } else if step.target.starts_with("sys/") && resource == "bootstrap" {
            Some((
                CodeEntryKindV2::SysBootstrapScript,
                TrustCapabilityV1::SysBootstrapScript,
            ))
        } else if step.target == "sys/profile"
            || (step.target.starts_with("sys/") && resource == "profile-state")
        {
            Some((
                CodeEntryKindV2::SysProfileCode,
                TrustCapabilityV1::SysProfileCode,
            ))
        } else {
            None
        };
        let Some((entry_kind, capability)) = classified else {
            continue;
        };
        let timing = if entry_kind == CodeEntryKindV2::ShellCommand
            || entry_kind == CodeEntryKindV2::SysProfileCode
        {
            CodeTimingV2::DeliverForLater
        } else {
            CodeTimingV2::ExecuteNow
        };
        let (source, trust) = code_boundary_trust(runtime, &step.target, capability)?;
        let target_role = if step.kind == Some(PlanStepKindV1::ShellSharedCodeAffected) {
            CodeTargetRoleV2::SharedResourceAffected
        } else {
            CodeTargetRoleV2::Selected
        };
        boundaries.insert(
            (step.target.clone(), entry_kind),
            CodeBoundaryV2 {
                target: step.target.clone(),
                entry_kind,
                timing,
                source,
                trust,
                unisolated: true,
                target_role,
                shared_resource: (entry_kind == CodeEntryKindV2::ShellCommand).then(|| {
                    format!(
                        "shell/{}/shared-category",
                        step.target.split('/').nth(1).unwrap_or_default()
                    )
                }),
            },
        );
    }
    plan.code_boundaries = boundaries.into_values().collect();
    Ok(())
}

pub(super) fn attach_sys_profile_boundaries<H>(
    runtime: &CoreRuntime<H>,
    plan: &mut PlanV1,
    os_id: &str,
    manifest: &SysManifest,
    affected: &BTreeSet<String>,
) -> Result<()> {
    plan.code_boundaries
        .retain(|boundary| boundary.entry_kind != CodeEntryKindV2::SysProfileCode);
    for item in manifest
        .items
        .iter()
        .filter(|item| affected.contains(&item.id))
    {
        for requirement in runtime.sys_external_code_requirements(os_id, item)? {
            let trust = if runtime
                .operation_code_grants
                .iter()
                .any(|grant| grant.matches(&requirement))
            {
                CodeTrustStateV2::OperationConfirmation
            } else {
                match runtime.trust_decision(&requirement) {
                    TrustDecisionV1::Trusted => CodeTrustStateV2::SnapshotTrusted,
                    TrustDecisionV1::DevelopmentTrusted => CodeTrustStateV2::DevelopmentTrusted,
                    _ => CodeTrustStateV2::MissingOrStale,
                }
            };
            if requirement.capability == TrustCapabilityV1::SysProfileCode {
                plan.code_boundaries.push(CodeBoundaryV2 {
                    target: requirement.target,
                    entry_kind: CodeEntryKindV2::SysProfileCode,
                    timing: CodeTimingV2::DeliverForLater,
                    source: CodeSourceV2::ExternalOrOverlay,
                    trust,
                    unisolated: true,
                    target_role: CodeTargetRoleV2::Selected,
                    shared_resource: Some(format!("sys/{os_id}/profile")),
                });
            } else {
                for boundary in plan
                    .code_boundaries
                    .iter_mut()
                    .filter(|boundary| boundary.target == requirement.target)
                {
                    boundary.source = CodeSourceV2::ExternalOrOverlay;
                    boundary.trust = trust;
                }
            }
        }
        if (sys_item_has_executable_profile_code(item)
            || sys_profile_base_code_present(runtime, os_id))
            && !plan.code_boundaries.iter().any(|b| {
                b.target == format!("sys/{}", item.id)
                    && b.entry_kind == CodeEntryKindV2::SysProfileCode
            })
        {
            plan.code_boundaries.push(CodeBoundaryV2 {
                target: format!("sys/{}", item.id),
                entry_kind: CodeEntryKindV2::SysProfileCode,
                timing: CodeTimingV2::DeliverForLater,
                source: CodeSourceV2::ShineDistribution,
                trust: CodeTrustStateV2::Distribution,
                unisolated: true,
                target_role: CodeTargetRoleV2::Selected,
                shared_resource: Some(format!("sys/{os_id}/profile")),
            });
        }
    }
    plan.code_boundaries
        .sort_by(|a, b| (&a.target, a.entry_kind).cmp(&(&b.target, b.entry_kind)));
    Ok(())
}

pub(super) fn code_boundary_trust<H>(
    runtime: &CoreRuntime<H>,
    target: &str,
    capability: TrustCapabilityV1,
) -> Result<(CodeSourceV2, CodeTrustStateV2)> {
    let requirement = if let Some(name) = target.strip_prefix("app/") {
        runtime
            .app_categories(Some(name))?
            .into_iter()
            .next()
            .map(|category| runtime.app_external_code_requirements(&category))
            .transpose()?
            .unwrap_or_default()
            .into_iter()
            .find(|requirement| requirement.capability == capability)
    } else if let Some(value) = target.strip_prefix("shell/") {
        value
            .split_once('/')
            .and_then(|(category_name, command_name)| {
                let category = runtime
                    .shell_categories(Some(category_name))
                    .ok()?
                    .into_iter()
                    .next()?;
                let file = category
                    .files
                    .iter()
                    .find(|file| file.command_name == command_name)?;
                runtime
                    .shell_external_code_requirements(&category, file)
                    .ok()?
                    .into_iter()
                    .find(|requirement| requirement.capability == capability)
            })
    } else {
        None
    };
    if let Some(requirement) = requirement {
        let trust = if runtime
            .operation_code_grants
            .iter()
            .any(|grant| grant.matches(&requirement))
        {
            CodeTrustStateV2::OperationConfirmation
        } else {
            match runtime.trust_decision(&requirement) {
                TrustDecisionV1::Trusted => CodeTrustStateV2::SnapshotTrusted,
                TrustDecisionV1::DevelopmentTrusted => CodeTrustStateV2::DevelopmentTrusted,
                _ => CodeTrustStateV2::MissingOrStale,
            }
        };
        return Ok((CodeSourceV2::ExternalOrOverlay, trust));
    }
    if runtime
        .operation_code_grants
        .iter()
        .any(|grant| grant.target == target && grant.capability == capability)
    {
        return Ok((
            CodeSourceV2::ExternalOrOverlay,
            CodeTrustStateV2::OperationConfirmation,
        ));
    }
    if runtime.context().is_external_presets {
        let trust = runtime
            .context()
            .trust_grants
            .iter()
            .find(|grant| {
                grant.schema_version == TRUST_GRANT_SCHEMA_VERSION
                    && grant.target == target
                    && grant.capability == capability
            })
            .map_or(CodeTrustStateV2::MissingOrStale, |grant| match grant.mode {
                TrustModeV1::Snapshot => CodeTrustStateV2::SnapshotTrusted,
                TrustModeV1::Development => CodeTrustStateV2::DevelopmentTrusted,
            });
        Ok((CodeSourceV2::ExternalOrOverlay, trust))
    } else {
        Ok((
            CodeSourceV2::ShineDistribution,
            CodeTrustStateV2::Distribution,
        ))
    }
}
