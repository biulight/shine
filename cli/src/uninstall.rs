//! Shared CLI completion policy for App and Shell uninstall.
use anyhow::{Result, bail};
use shine_core::lifecycle::{LifecycleResultV1, LifecycleStatus};

pub(crate) fn incomplete(result: &LifecycleResultV1) -> bool {
    result.outcomes.iter().any(|outcome| {
        matches!(
            outcome.status,
            LifecycleStatus::Preserved | LifecycleStatus::Conflict | LifecycleStatus::Failed
        )
    })
}

pub(crate) fn summary_label(result: &LifecycleResultV1, dry_run: bool) -> &'static str {
    match (dry_run, incomplete(result)) {
        (true, true) => "Preview · protection would prevent complete uninstall",
        (true, false) => "Uninstall preview",
        (false, true) => "Uninstall incomplete",
        (false, false) => "Uninstall complete",
    }
}

pub(crate) fn check_completion(result: &LifecycleResultV1, dry_run: bool) -> Result<()> {
    if result
        .outcomes
        .iter()
        .any(|outcome| outcome.status == LifecycleStatus::Failed)
    {
        bail!("uninstall incomplete: one or more operations failed; see details above");
    }
    if !dry_run && incomplete(result) {
        bail!(
            "uninstall incomplete: protected or conflicting resources remain; see guidance above"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shine_core::lifecycle::{LifecycleOperation, LifecycleOutcomeV1};

    #[test]
    fn completion_distinguishes_protection_partial_success_and_preview_failures() {
        for status in [
            LifecycleStatus::Preserved,
            LifecycleStatus::Conflict,
            LifecycleStatus::Failed,
        ] {
            for partial in [false, true] {
                let mut result = LifecycleResultV1::new(LifecycleOperation::Uninstall, false);
                if partial {
                    result.push(LifecycleOutcomeV1::new(
                        "app/removed",
                        None::<String>,
                        LifecycleStatus::Changed,
                        [],
                    ));
                }
                result.push(LifecycleOutcomeV1::new(
                    "app/kept",
                    None::<String>,
                    status,
                    [],
                ));
                assert_eq!(summary_label(&result, false), "Uninstall incomplete");
                assert!(check_completion(&result, false).is_err());
                assert_eq!(
                    check_completion(&result, true).is_err(),
                    status == LifecycleStatus::Failed
                );
            }
        }
        let mut complete = LifecycleResultV1::new(LifecycleOperation::Uninstall, false);
        complete.push(LifecycleOutcomeV1::new(
            "shell/sample/one",
            None::<String>,
            LifecycleStatus::Changed,
            [],
        ));
        assert_eq!(summary_label(&complete, false), "Uninstall complete");
        assert!(check_completion(&complete, false).is_ok());
        assert_eq!(summary_label(&complete, true), "Uninstall preview");
    }
}
