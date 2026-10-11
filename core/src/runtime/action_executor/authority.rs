//! App authority.

use super::*;

pub(super) fn validate_app_action_authority<'a>(
    context: &crate::runtime::RuntimeContext,
    plan: &PlanV1,
    approval: &PlanApprovalV1,
    action_ir: &'a ActionIrV1,
) -> Result<&'a crate::action::DeclarativeActionV1> {
    approval.validate(plan)?;
    action_ir.validate()?;
    let requirements = action_ir.permission_requirements(|path| review_path(context, path));
    if !requirements.uncomputable_codes.is_empty() {
        bail!("action permissions are not fully computable");
    }
    if requirements
        .required
        .iter()
        .any(|permission| !approval.approved_permissions.contains(permission))
    {
        bail!("action permission was not included in the approved security Plan");
    }
    let [action] = action_ir.actions.as_slice() else {
        bail!("an App managed-resource operation accepts exactly one action");
    };
    Ok(action)
}
