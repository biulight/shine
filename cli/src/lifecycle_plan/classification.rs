//! Local presentation classification over unchanged, fingerprint-bound Plan DTOs.
//! These classes control terminal grouping only and never grant execution authority.
use shine_core::plan::{PlanActionV1, PlanStepV1, PlanV1};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StepPresentation {
    AppCacheMaintenance,
    ShellCacheMaintenance,
    StaleAppPreservation,
    Other,
}
impl StepPresentation {
    pub(super) fn classify(step: &PlanStepV1) -> Self {
        if matches!(step.action, PlanActionV1::Create | PlanActionV1::Update) {
            match step.resource.as_deref() {
                Some("preset-cache")
                    if step.target.starts_with("shell/")
                        && step.diagnostic_codes == ["shell_cache_replace_transaction"] =>
                {
                    return Self::ShellCacheMaintenance;
                }
                resource
                    if is_app_cache_resource(resource)
                        && step.target.starts_with("app/")
                        && step.diagnostic_codes.is_empty() =>
                {
                    return Self::AppCacheMaintenance;
                }
                _ => {}
            }
        }
        if step.target.starts_with("app/")
            && step.action == PlanActionV1::Preserve
            && step.diagnostic_codes == ["app_stale_source_preserved"]
        {
            Self::StaleAppPreservation
        } else {
            Self::Other
        }
    }
}

pub(super) fn is_routine_cache_maintenance(step: &PlanStepV1) -> bool {
    matches!(
        StepPresentation::classify(step),
        StepPresentation::AppCacheMaintenance | StepPresentation::ShellCacheMaintenance
    )
}
pub(super) fn is_stale_app_preservation(step: &PlanStepV1) -> bool {
    StepPresentation::classify(step) == StepPresentation::StaleAppPreservation
}
pub(super) fn is_app_cache_resource(resource: Option<&str>) -> bool {
    resource.is_some_and(|resource| resource.starts_with("preset-cache:"))
}
pub(super) fn is_manual_refresh_notice(code: &str) -> bool {
    code == "app_manual_refresh_required"
}
pub(super) fn is_routine_transaction_notice(code: &str) -> bool {
    matches!(
        code,
        "shell_cache_replace_transaction"
            | "shell_snapshot_replace_transaction"
            | "shell_managed_launcher_update_transaction"
            | "shell_profile_reconcile_transaction"
            | "app_hook_execution"
    )
}

pub(super) fn quiet_uninstall_step(step: &PlanStepV1) -> bool {
    step.action == PlanActionV1::None
        && (step.diagnostic_codes.is_empty()
            || (step.resource.as_deref() == Some("preset-cache")
                && step.diagnostic_codes.iter().all(|code| {
                    matches!(
                        code.as_str(),
                        "app_preset_cache_remove" | "shell_cache_remove_transaction"
                    )
                })))
}

pub(super) fn upgrade_step_has_review_content(step: &PlanStepV1) -> bool {
    step.action != PlanActionV1::None
        || step
            .diagnostic_codes
            .iter()
            .any(|code| !is_manual_refresh_notice(code))
}

pub(super) fn upgrade_plan_has_review_content(plan: &PlanV1) -> bool {
    !plan.is_ready()
        || plan.steps.iter().any(upgrade_step_has_review_content)
        || !plan.permissions.required.is_empty()
        || !plan.permissions.missing_declarations.is_empty()
        || !plan.permissions.uncomputable_codes.is_empty()
        || !plan.code_boundaries.is_empty()
}
