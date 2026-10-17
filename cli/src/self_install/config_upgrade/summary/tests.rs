use super::*;
use shine_core::lifecycle::{
    LifecycleEffect, LifecycleOperation, LifecycleOutcomeV1, LifecycleStatus,
};

fn result(rows: &[(&str, &str, LifecycleStatus, LifecycleEffect)]) -> LifecycleResultV1 {
    let mut result = LifecycleResultV1::new(LifecycleOperation::Upgrade, false);
    for (target, resource, status, effect) in rows {
        result.push(LifecycleOutcomeV1::new(
            *target,
            Some(*resource),
            *status,
            [*effect],
        ));
    }
    result
}

#[test]
fn target_app_summary_counts_categories_and_preserved_resources_without_auxiliary_work() {
    use LifecycleEffect::{CacheWritten, CodeExecuted, ResourceWritten, UserResourcePreserved};
    use LifecycleStatus::{Changed, Conflict, Failed, Preserved};
    let lifecycle = result(&[
        ("app/demo", "one", Changed, ResourceWritten),
        ("app/demo", "two", Changed, ResourceWritten),
        ("app/demo", "local", Preserved, UserResourcePreserved),
        ("app/demo", "conflict", Conflict, UserResourcePreserved),
        ("app/other", "preset-cache", Changed, CacheWritten),
        ("app/other", "purge", Changed, CacheWritten),
        ("app/other", "hook:post-install", Changed, CodeExecuted),
        (
            "app/other",
            "artifact:teardown",
            Conflict,
            UserResourcePreserved,
        ),
        ("app/demo", "hook:post-upgrade", Failed, CodeExecuted),
    ]);
    let hints = BTreeSet::from(["Restart demo".into()]);
    let report = apps::AppUpgradeReport {
        updated: 99,
        updated_categories: 99,
        failed: 0, // A non-fatal hook outcome must not become a fatal report failure.
        restart_hints: hints.clone(),
        ..Default::default()
    };
    assert_eq!(
        TargetUpgradeSummary::from_app(report, &lifecycle),
        TargetUpgradeSummary {
            updated: 1,
            user_modified: 2,
            restart_hints: hints,
            ..Default::default()
        }
    );
    assert_eq!(
        TargetUpgradeSummary::from_app(
            apps::AppUpgradeReport {
                failed: 3,
                ..Default::default()
            },
            &lifecycle
        )
        .failed,
        3
    );
}

#[test]
fn target_shell_summary_excludes_cache_only_categories_and_counts_path_separately() {
    use LifecycleEffect::{CacheWritten, ResourceWritten, UserResourcePreserved};
    use LifecycleStatus::{Changed, Conflict};
    let lifecycle = result(&[
        ("shell/demo/one", "one", Changed, ResourceWritten),
        ("shell/demo/two", "two", Changed, ResourceWritten),
        ("shell/cache", "preset-cache", Changed, CacheWritten),
        (
            "shell/demo/foreign",
            "foreign",
            Conflict,
            UserResourcePreserved,
        ),
    ]);
    let report = shells::ShellUpgradeReport {
        path_changed: true,
        link_conflicts: 99,
        ..Default::default()
    };
    assert_eq!(
        TargetUpgradeSummary::from_shell(&report, &lifecycle),
        TargetUpgradeSummary {
            updated: 2,
            link_conflicts: 1,
            ..Default::default()
        }
    );
    assert_eq!(
        TargetUpgradeSummary::from_shell(&shells::ShellUpgradeReport::default(), &lifecycle)
            .updated,
        1
    );
}

#[test]
fn target_sys_summary_keeps_resource_counts_and_authoritative_failure_total() {
    use LifecycleEffect::{ResourceWritten, UserResourcePreserved};
    use LifecycleStatus::{Changed, Conflict, Failed, Preserved};
    let lifecycle = result(&[
        ("sys/demo", "one", Changed, ResourceWritten),
        ("sys/demo", "two", Changed, ResourceWritten),
        ("sys/demo", "local", Preserved, UserResourcePreserved),
        ("sys/demo", "conflict", Conflict, UserResourcePreserved),
        ("sys/demo", "failed", Failed, ResourceWritten),
    ]);
    let report = shine_core::runtime::SysUpgradeReport {
        updated: 99,
        failed: 4,
        ..Default::default()
    };
    assert_eq!(
        TargetUpgradeSummary::from_sys(&report, &lifecycle),
        TargetUpgradeSummary {
            updated: 2,
            user_modified: 2,
            failed: 4,
            ..Default::default()
        }
    );
}
#[test]
fn config_upgrade_summary_parts_includes_only_nonzero_counts() {
    assert_eq!(
        config_upgrade_summary_parts(2, 0, 0),
        vec!["2 updated".to_string()]
    );
}

#[test]
fn config_upgrade_summary_parts_empty_when_all_zero() {
    assert!(config_upgrade_summary_parts(0, 0, 0).is_empty());
}

#[test]
fn config_upgrade_summary_parts_reports_actionable_counters() {
    assert_eq!(
        config_upgrade_summary_parts(1, 2, 3),
        vec![
            "1 updated".to_string(),
            "2 user-modified (kept)".to_string(),
            "3 link conflicts".to_string(),
        ]
    );
}

#[test]
fn app_upgrade_failure_count_uses_the_authoritative_report_total() {
    let report = apps::AppUpgradeReport {
        failed: 3,
        ..Default::default()
    };

    assert_eq!(app_upgrade_failure_count(&report), 3);
}
