use super::{
    permission_name, render_code_boundaries, stale_app_source_guidance, styled_action_name,
};
use crate::lifecycle_plan::{
    LifecyclePlanRequest,
    classification::{upgrade_plan_has_review_content, upgrade_step_has_review_content},
};
use crate::output::escape_controls;
use anyhow::Result;
use shine_core::plan::PlanV1;

pub(in crate::lifecycle_plan) fn render_upgrade_detailed_plan_lines(
    planned: &[(LifecyclePlanRequest, PlanV1)],
    config_digest: &str,
) -> Result<Vec<String>> {
    planned
        .iter()
        .filter(|(_, plan)| upgrade_plan_has_review_content(plan))
        .map(|(_, plan)| render_plan_lines_with_steps(plan, config_digest, false))
        .collect::<Result<Vec<_>>>()
        .map(|sections| sections.into_iter().flatten().collect())
}

pub(in crate::lifecycle_plan) fn render_plan_lines(
    plan: &PlanV1,
    config_digest: &str,
) -> Result<Vec<String>> {
    render_plan_lines_with_steps(plan, config_digest, true)
}

pub(in crate::lifecycle_plan) fn render_plan_lines_with_steps(
    plan: &PlanV1,
    config_digest: &str,
    include_unchanged_steps: bool,
) -> Result<Vec<String>> {
    let mut lines = vec![crate::colors::bold(&format!(
        "Security Plan · {}",
        plan.operation.as_str()
    ))];
    lines.push(format!(
        "  Preset snapshot  {}",
        plan.inputs.preset.as_hex()
    ));
    lines.push(format!("  Config snapshot  {config_digest}"));
    lines.push(format!("  State snapshot   {}", plan.inputs.state.as_hex()));
    let visible_steps = plan
        .steps
        .iter()
        .filter(|step| include_unchanged_steps || upgrade_step_has_review_content(step))
        .collect::<Vec<_>>();
    if include_unchanged_steps || !visible_steps.is_empty() {
        lines.push(format!("  {}", crate::colors::bold("Steps")));
        if visible_steps.is_empty() {
            lines.push(format!("    {}", crate::colors::dim("- none")));
        }
    }
    for step in visible_steps {
        let resource = step
            .resource
            .as_deref()
            .map(|value| format!(" · {}", escape_controls(value)))
            .unwrap_or_default();
        let diagnostics = if step.diagnostic_codes.is_empty() {
            String::new()
        } else {
            format!(" [{}]", escape_controls(&step.diagnostic_codes.join(", ")))
        };
        lines.push(format!(
            "    {} {}{}{}",
            styled_action_name(step.action),
            escape_controls(&step.target),
            resource,
            diagnostics
        ));
        lines.extend(stale_app_source_guidance(step, "      "));
    }
    lines.extend(render_code_boundaries(plan, "  ", true));
    lines.push(format!("  {}", crate::colors::bold("Required permissions")));
    if plan.permissions.required.is_empty() {
        lines.push(format!("    {}", crate::colors::dim("- none")));
    }
    for permission in plan.permissions.required.iter() {
        lines.push(format!("    - {}", permission_name(permission)));
    }
    if !plan.author_capabilities.is_empty() {
        lines.push(format!(
            "  {}",
            crate::colors::bold("Author capability statement (unverified)")
        ));
        for permission in plan.author_capabilities.iter() {
            lines.push(format!("    - {}", permission_name(permission)));
        }
    }
    for permission in plan.permissions.missing_declarations.iter() {
        lines.push(format!(
            "    {} {}",
            crate::colors::red("! missing declaration:"),
            permission_name(permission)
        ));
    }
    for code in &plan.permissions.uncomputable_codes {
        let code = escape_controls(code);
        lines.push(format!(
            "    {} {code}",
            crate::colors::red("! uncomputable:")
        ));
    }
    lines.push(format!(
        "  Fingerprint      {}",
        plan.fingerprint()?.as_hex()
    ));
    Ok(lines)
}
