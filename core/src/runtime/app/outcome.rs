//! Internal outcomes bind report status and required local error/backup details.
//! The public, frontend-private report DTO retains its existing fields.
use super::{AppFileAction, AppFileLifecycleReport, AppInstallStrategy, LifecycleStatus};
use std::path::PathBuf;

pub(super) enum AppFileOutcome {
    Installed,
    BackedUp(PathBuf),
    Unchanged,
    PreviewInstall,
    GeneratorPreserved(String),
    GeneratorFailed(String),
    Removed,
    Restored,
    ForceRemoved,
    ForceRestored,
    Missing,
    UserModified,
    PreviewRemove,
    Failed(String),
}

impl AppFileOutcome {
    pub(super) fn status(&self) -> LifecycleStatus {
        match self {
            Self::Installed
            | Self::BackedUp(_)
            | Self::Removed
            | Self::Restored
            | Self::ForceRemoved
            | Self::ForceRestored
            | Self::Missing => LifecycleStatus::Changed,
            Self::Unchanged => LifecycleStatus::Unchanged,
            Self::PreviewInstall | Self::PreviewRemove => LifecycleStatus::Previewed,
            Self::GeneratorPreserved(_) | Self::UserModified => LifecycleStatus::Preserved,
            Self::GeneratorFailed(_) | Self::Failed(_) => LifecycleStatus::Failed,
        }
    }

    /// Whether the report's ordinary error channel is populated.
    pub(super) fn has_error(&self) -> bool {
        matches!(self, Self::Failed(_))
    }

    fn action(&self) -> AppFileAction {
        match self {
            Self::Installed => AppFileAction::Installed,
            Self::BackedUp(_) => AppFileAction::BackedUp,
            Self::Unchanged => AppFileAction::Unchanged,
            Self::PreviewInstall => AppFileAction::PreviewInstall,
            Self::GeneratorPreserved(_) | Self::GeneratorFailed(_) => {
                AppFileAction::GeneratorPreserved
            }
            Self::Removed => AppFileAction::Removed,
            Self::Restored => AppFileAction::Restored,
            Self::ForceRemoved => AppFileAction::ForceRemoved,
            Self::ForceRestored => AppFileAction::ForceRestored,
            Self::Missing => AppFileAction::Missing,
            Self::UserModified => AppFileAction::UserModified,
            Self::PreviewRemove => AppFileAction::PreviewRemove,
            Self::Failed(_) => AppFileAction::Failed,
        }
    }
}

pub(super) struct AppFileReportContext {
    pub category: String,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub transforms: Vec<String>,
    pub backup: Option<PathBuf>,
    pub uninstall_strategy: Option<AppInstallStrategy>,
    pub restart_hint: Option<String>,
}

impl AppFileReportContext {
    pub(super) fn finish(self, outcome: AppFileOutcome) -> AppFileLifecycleReport {
        let status = outcome.status();
        let action = outcome.action();
        let mut backup = self.backup;
        let mut generator_error = None;
        let mut error = None;
        match outcome {
            AppFileOutcome::BackedUp(path) => backup = Some(path),
            AppFileOutcome::GeneratorPreserved(detail)
            | AppFileOutcome::GeneratorFailed(detail) => {
                generator_error = Some(detail);
            }
            AppFileOutcome::Failed(detail) => error = Some(detail),
            AppFileOutcome::Installed
            | AppFileOutcome::Unchanged
            | AppFileOutcome::PreviewInstall
            | AppFileOutcome::Removed
            | AppFileOutcome::Restored
            | AppFileOutcome::ForceRemoved
            | AppFileOutcome::ForceRestored
            | AppFileOutcome::Missing
            | AppFileOutcome::UserModified
            | AppFileOutcome::PreviewRemove => {}
        }
        AppFileLifecycleReport {
            category: self.category,
            source: self.source,
            destination: self.destination,
            transforms: self.transforms,
            backup,
            uninstall_strategy: self.uninstall_strategy,
            restart_hint: self.restart_hint,
            generator_error,
            error,
            status,
            action,
        }
    }
}

/// Successful explicit refresh decisions; other lifecycle actions are not refresh results.
pub(super) enum RefreshChange {
    Installed,
    Unchanged,
    UserModified,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> AppFileReportContext {
        AppFileReportContext {
            category: "demo".into(),
            source: "config.toml".into(),
            destination: "installed/config.toml".into(),
            transforms: vec!["template".into()],
            backup: Some("old.backup".into()),
            uninstall_strategy: None,
            restart_hint: Some("restart demo".into()),
        }
    }

    #[test]
    fn generator_preservation_keeps_install_and_upgrade_status_and_error_channels() {
        let preserved = context().finish(AppFileOutcome::GeneratorPreserved("unavailable".into()));
        let failed = context().finish(AppFileOutcome::GeneratorFailed("unavailable".into()));
        assert_eq!(preserved.status, LifecycleStatus::Preserved);
        assert_eq!(failed.status, LifecycleStatus::Failed);
        for report in [preserved, failed] {
            assert_eq!(report.action, AppFileAction::GeneratorPreserved);
            assert_eq!(report.generator_error.as_deref(), Some("unavailable"));
            assert!(report.error.is_none());
            assert_eq!(report.restart_hint.as_deref(), Some("restart demo"));
        }
    }

    #[test]
    fn backup_success_and_local_failure_keep_required_report_details() {
        let backed_up = context().finish(AppFileOutcome::BackedUp("new.backup".into()));
        assert_eq!(backed_up.action, AppFileAction::BackedUp);
        assert_eq!(backed_up.backup, Some(PathBuf::from("new.backup")));
        assert!(backed_up.error.is_none());
        let failed = context().finish(AppFileOutcome::Failed("write failed".into()));
        assert_eq!(failed.action, AppFileAction::Failed);
        assert_eq!(failed.status, LifecycleStatus::Failed);
        assert_eq!(failed.error.as_deref(), Some("write failed"));
        assert!(failed.generator_error.is_none());
        assert_eq!(failed.backup, Some(PathBuf::from("old.backup")));
    }
}
