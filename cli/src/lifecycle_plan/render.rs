//! Pure terminal projection of reviewed Plans; never creates approval.

mod bootstrap;
mod code_boundaries;
mod compact;
mod detailed;
mod labels;
mod permissions;

pub(super) use bootstrap::*;
pub(super) use code_boundaries::*;
pub(super) use compact::*;
pub(super) use detailed::*;
#[cfg(test)]
pub(super) use labels::{PlanActionTone, plan_action_tone};
pub(crate) use labels::{permission_name, styled_action_name};
pub(super) use labels::{stale_app_source_guidance, style_plan_action};
pub(crate) use permissions::permission_group;
pub(super) use permissions::{
    render_bootstrap_permissions, render_compact_permissions, render_scope_summary,
};
