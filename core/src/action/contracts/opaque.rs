//! Resource-specific validation and exact executable permission rules.
use super::*;

pub(super) fn validate(
    kind: &ActionKindV1,
    rollback: &RollbackSupportV1,
) -> Result<(), ActionIrError> {
    match (kind, rollback) {
        (
            ActionKindV1::OpaqueExecution { capability, .. },
            RollbackSupportV1::Unsupported { reason_code },
        ) => {
            validate_identity("opaque capability", capability)?;
            validate_identity("rollback reason", reason_code)
        }
        (ActionKindV1::OpaqueExecution { .. }, _) => Err(ActionIrError::Invalid(
            "opaque execution must declare rollback as unsupported".to_string(),
        )),
        _ => Err(ActionIrError::Invalid(
            "action routed to an incompatible resource contract".to_string(),
        )),
    }
}

pub(super) fn permissions(
    kind: &ActionKindV1,
    _path_identity: &impl Fn(&Path) -> String,
    _required: &mut PermissionSetV1,
    uncomputable_codes: &mut BTreeSet<String>,
) {
    match kind {
        ActionKindV1::OpaqueExecution { .. } => {
            uncomputable_codes.insert("opaque_action_permissions_uncomputable".to_string());
        }
        _ => {
            uncomputable_codes.insert("action_resource_permissions_uncomputable".to_string());
        }
    }
}
