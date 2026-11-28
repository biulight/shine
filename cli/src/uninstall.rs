//! Shared CLI completion policy for App and Shell uninstall.
use anyhow::Result;
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
        (true, true) => "Uninstall preview · protected",
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
        || (!dry_run && incomplete(result))
    {
        return Err(crate::output::ReportedCommandFailure.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shine_core::lifecycle::{LifecycleOperation, LifecycleOutcomeV1};

    #[test]
    fn completion_distinguishes_protection_partial_success_and_preview_failures() {
        let outcome = |target, status| LifecycleOutcomeV1::new(target, None::<String>, status, []);
        assert!(
            !anyhow::anyhow!("command incomplete; see reported results")
                .is::<crate::output::ReportedCommandFailure>()
        );
        for status in [
            LifecycleStatus::Preserved,
            LifecycleStatus::Conflict,
            LifecycleStatus::Failed,
        ] {
            for partial in [false, true] {
                let mut result = LifecycleResultV1::new(LifecycleOperation::Uninstall, false);
                if partial {
                    result.push(outcome("app/removed", LifecycleStatus::Changed));
                }
                result.push(outcome("app/kept", status));
                assert_eq!(summary_label(&result, false), "Uninstall incomplete");
                let error = check_completion(&result, false).unwrap_err();
                assert!(error.is::<crate::output::ReportedCommandFailure>());
                assert_eq!(
                    check_completion(&result, true).is_err(),
                    status == LifecycleStatus::Failed
                );
            }
        }
        let mut complete = LifecycleResultV1::new(LifecycleOperation::Uninstall, false);
        complete.push(outcome("shell/sample/one", LifecycleStatus::Changed));
        assert_eq!(summary_label(&complete, false), "Uninstall complete");
        assert!(check_completion(&complete, false).is_ok());
        assert_eq!(summary_label(&complete, true), "Uninstall preview");
    }
}
