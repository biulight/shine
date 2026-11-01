use super::{
    permission_name, render_bootstrap_permissions, render_code_boundaries, render_plan_lines,
    render_scope_summary, styled_action_name,
};
use crate::output::escape_controls;
use anyhow::Result;
use shine_core::plan::{NetworkScopeV1, PermissionV1, PlanV1};

pub(in crate::lifecycle_plan) fn render_bootstrap_plan_lines(
    plan: &PlanV1,
    config_digest: &str,
    verbose: bool,
) -> Result<Vec<String>> {
    let scoped_required = shine_core::plan::PermissionSetV1::new(
        plan.permission_scopes
            .iter()
            .flat_map(|scope| scope.permissions.required.iter().cloned()),
    );
    let targets = plan
        .permission_scopes
        .iter()
        .map(|scope| scope.target.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    if targets.len() != plan.permission_scopes.len()
        || scoped_required != plan.permissions.required
        || plan
            .permission_scopes
            .iter()
            .filter_map(|scope| scope.target.as_ref())
            .any(|target| !plan.steps.iter().any(|step| &step.target == target))
    {
        return render_plan_lines(plan, config_digest);
    }
    let items = plan
        .steps
        .iter()
        .filter(|step| step.resource.as_deref() == Some("bootstrap"))
        .count();
    let profiles = plan
        .steps
        .iter()
        .filter(|step| step.target == "sys/profile")
        .count();
    let mut lines = vec![
        crate::colors::bold("Security Plan · sys-bootstrap"),
        format!(
            "  {items} bootstrap item{} · {profiles} profile configuration{}",
            if items == 1 { "" } else { "s" },
            if profiles == 1 { "" } else { "s" }
        ),
    ];
    let mut attention = Vec::new();
    if plan
        .permissions
        .required
        .contains(&PermissionV1::Administrator)
    {
        attention.push("Administrator access required");
    }
    if plan.permissions.required.contains(&PermissionV1::Network {
        scope: NetworkScopeV1::Any,
    }) {
        attention.push("Unrestricted network access required");
    }
    if !plan.is_ready() {
        attention.push("Plan is blocked; review the diagnostics below");
    }
    if plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .iter()
            .any(|code| code == "sys_bootstrap_profile_recovery_unsupported")
    }) {
        attention.push("Recovery is unsupported for the profile step");
    }
    if !attention.is_empty() {
        lines.push(format!("\n  {}", crate::colors::bold("Attention")));
        for message in attention {
            lines.push(format!("    {} {message}", crate::colors::yellow("!")));
        }
    }
    lines.extend(render_code_boundaries(plan, "  ", verbose));
    if verbose && !plan.author_capabilities.is_empty() {
        lines.push(format!(
            "  {}",
            crate::colors::bold("Author capability statement (unverified)")
        ));
        for permission in plan.author_capabilities.iter() {
            lines.push(format!("    - {}", permission_name(permission)));
        }
    }
    lines.push(format!("\n  {}", crate::colors::bold("Bootstrap items")));
    let mut item_index = 0;
    for step in &plan.steps {
        if step.target == "sys/profile" {
            lines.push(format!(
                "\n  {}",
                crate::colors::bold("Profile configuration")
            ));
            lines.push(format!(
                "\n    {}  {}",
                styled_action_name(step.action),
                escape_controls(step.resource.as_deref().unwrap_or("profile"))
            ));
        } else {
            item_index += 1;
            lines.push(format!(
                "\n    {item_index:02}  {}  {}",
                escape_controls(step.target.strip_prefix("sys/").unwrap_or(&step.target)),
                styled_action_name(step.action)
            ));
        }
        if let Some(scope) = plan
            .permission_scopes
            .iter()
            .find(|scope| scope.target.as_deref() == Some(step.target.as_str()))
        {
            lines.extend(if verbose {
                render_bootstrap_permissions(&scope.permissions, "        ")
            } else {
                render_scope_summary(plan, &scope.permissions, "        ")
            });
        } else {
            // Older/incomplete contracts must never look like an empty permission set.
            return render_plan_lines(plan, config_digest);
        }
        for code in &step.diagnostic_codes {
            let message = match code.as_str() {
                "sys_bootstrap_profile_recovery_unsupported" => {
                    "Recovery is unsupported for this profile step"
                }
                "sys_bootstrap_required_env_missing" => "Required environment input is missing",
                "sys_external_code_not_allowed" => "External preset code is not trusted",
                _ => code,
            };
            let suffix = if verbose && message != code {
                format!(" [{code}]")
            } else {
                String::new()
            };
            let message = escape_controls(message);
            let suffix = escape_controls(&suffix);
            lines.push(format!(
                "        {} {message}{suffix}",
                crate::colors::yellow("!")
            ));
        }
    }
    for scope in plan
        .permission_scopes
        .iter()
        .filter(|scope| scope.target.is_none())
    {
        lines.push(format!("\n  {}", crate::colors::bold("Shared changes")));
        lines.extend(if verbose {
            render_bootstrap_permissions(&scope.permissions, "    ")
        } else {
            render_scope_summary(plan, &scope.permissions, "    ")
        });
    }
    // Keep aggregate blockers visible even if a future planner adds a non-scoped diagnostic.
    for permission in plan.permissions.missing_declarations.iter() {
        lines.push(format!(
            "  ! Missing declaration: {}",
            permission_name(permission)
        ));
    }
    for code in &plan.permissions.uncomputable_codes {
        if !plan
            .permission_scopes
            .iter()
            .any(|scope| scope.permissions.uncomputable_codes.contains(code))
        {
            lines.push(format!(
                "  ! Uncomputable permissions: {}",
                escape_controls(code)
            ));
        }
    }
    if verbose {
        lines.push(format!("\n  {}", crate::colors::bold("Plan identity")));
        for (label, identity) in [
            ("Preset", plan.inputs.preset.as_hex()),
            ("Config", config_digest.to_string()),
            ("State", plan.inputs.state.as_hex()),
            ("Fingerprint", plan.fingerprint()?.as_hex()),
        ] {
            lines.push(crate::colors::dim(&format!("    {label:<12}{identity}")));
        }
    }
    if !verbose {
        lines.push(crate::colors::dim(
            "  Use --verbose for all paths, identities and diagnostic codes.",
        ));
    }
    Ok(lines)
}
