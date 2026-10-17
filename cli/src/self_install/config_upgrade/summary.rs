//! Pure domain-specific counters and the target-upgrade result; no approval authority.

use crate::{apps, colors, output, shells};
use shine_core::lifecycle::LifecycleResultV1;
use std::collections::BTreeSet;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct TargetUpgradeSummary {
    pub(super) updated: usize,
    pub(super) user_modified: usize,
    pub(super) link_conflicts: usize,
    pub(super) failed: usize,
    pub(super) restart_hints: BTreeSet<String>,
}

impl TargetUpgradeSummary {
    pub(super) fn from_app(report: apps::AppUpgradeReport, lifecycle: &LifecycleResultV1) -> Self {
        Self {
            updated: changed_app_categories(lifecycle),
            user_modified: preserved_app_resources(lifecycle),
            link_conflicts: 0,
            failed: app_upgrade_failure_count(&report),
            restart_hints: report.restart_hints,
        }
    }

    pub(super) fn from_shell(
        report: &shells::ShellUpgradeReport,
        lifecycle: &LifecycleResultV1,
    ) -> Self {
        Self {
            updated: changed_shell_categories(lifecycle) + usize::from(report.path_changed),
            user_modified: 0,
            link_conflicts: lifecycle.summary().conflicts,
            failed: 0,
            restart_hints: BTreeSet::new(),
        }
    }

    pub(super) fn from_sys(
        report: &shine_core::runtime::SysUpgradeReport,
        lifecycle: &LifecycleResultV1,
    ) -> Self {
        Self {
            updated: lifecycle.summary().changed,
            user_modified: lifecycle.summary().preserved + lifecycle.summary().conflicts,
            link_conflicts: 0,
            failed: report.failed,
            restart_hints: BTreeSet::new(),
        }
    }
}

pub(super) fn config_upgrade_summary_parts(
    updated: usize,
    user_modified: usize,
    link_conflicts: usize,
) -> Vec<String> {
    let mut parts = Vec::new();
    output::push_count(&mut parts, updated, colors::green, "updated");
    output::push_count(
        &mut parts,
        user_modified,
        colors::yellow,
        "user-modified (kept)",
    );
    output::push_count(&mut parts, link_conflicts, colors::yellow, "link conflicts");
    parts
}

pub(super) fn app_upgrade_failure_count(report: &apps::AppUpgradeReport) -> usize {
    report.failed
}

pub(super) fn changed_shell_categories(result: &shine_core::lifecycle::LifecycleResultV1) -> usize {
    result
        .outcomes
        .iter()
        .filter(|outcome| {
            outcome.status == shine_core::lifecycle::LifecycleStatus::Changed
                && outcome.target.starts_with("shell/")
                && outcome.effects.iter().any(|effect| {
                    !matches!(effect, shine_core::lifecycle::LifecycleEffect::CacheWritten)
                })
        })
        .filter_map(|outcome| outcome.target.split('/').nth(1))
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

fn is_app_auxiliary_resource(resource: Option<&str>) -> bool {
    matches!(
        resource,
        Some(
            "preset-cache"
                | "purge"
                | "hook:post-install"
                | "hook:post-upgrade"
                | "artifact:teardown"
        )
    )
}

pub(super) fn changed_app_categories(result: &shine_core::lifecycle::LifecycleResultV1) -> usize {
    result
        .outcomes
        .iter()
        .filter(|outcome| {
            outcome.status == shine_core::lifecycle::LifecycleStatus::Changed
                && outcome.target.starts_with("app/")
                && !is_app_auxiliary_resource(outcome.resource.as_deref())
        })
        .map(|outcome| outcome.target.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len()
}

pub(super) fn preserved_app_resources(result: &shine_core::lifecycle::LifecycleResultV1) -> usize {
    result
        .outcomes
        .iter()
        .filter(|outcome| {
            matches!(
                outcome.status,
                shine_core::lifecycle::LifecycleStatus::Preserved
                    | shine_core::lifecycle::LifecycleStatus::Conflict
            ) && outcome.target.starts_with("app/")
                && !is_app_auxiliary_resource(outcome.resource.as_deref())
        })
        .count()
}

#[cfg(test)]
mod tests;
