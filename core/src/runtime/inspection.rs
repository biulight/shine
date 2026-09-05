use super::{AppCategory, AppFile, ShellCategory, ShellFile};
use crate::install::AppEntry;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum InspectionFileStatus {
    NotInstalled,
    UpToDate,
    UpdateAvail,
    GeneratorNotEvaluated,
    GeneratorEvaluationFailed,
    GeneratorTrustRequired,
    Partial,
    UserModified,
    Missing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InspectionChange {
    ContentChanged,
    SourceRelocated {
        from: PathBuf,
        to: PathBuf,
    },
    DestinationRelocated {
        from: PathBuf,
        to: PathBuf,
    },
    NewFile {
        destination: PathBuf,
    },
    DeploymentChanged {
        field: &'static str,
        from: String,
        to: String,
    },
    CommandEntryMissing {
        path: PathBuf,
    },
    CommandEntryOutdated {
        path: PathBuf,
    },
    ManifestEntryMissing {
        target: String,
    },
}

impl InspectionChange {
    pub fn includes_content(changes: &[Self]) -> bool {
        changes.contains(&Self::ContentChanged)
    }
}

#[derive(Clone, Debug)]
pub struct AppFileInspection {
    pub category: AppCategory,
    pub file: AppFile,
    pub destination: Option<PathBuf>,
    pub status: InspectionFileStatus,
    pub manifest_entry: Option<AppEntry>,
    pub desired_content: Option<Vec<u8>>,
    pub current_content: Option<Vec<u8>>,
    pub changes: Vec<InspectionChange>,
    pub assessment_error: Option<String>,
    pub assessment_diagnostic: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub struct ShellFileInspection {
    pub category: ShellCategory,
    pub file: ShellFile,
    pub source_path: PathBuf,
    pub installed_source_path: PathBuf,
    pub rendered_path: PathBuf,
    pub link_path: PathBuf,
    pub link_target: Option<PathBuf>,
    pub desired_content: Option<Vec<u8>>,
    pub current_content: Option<Vec<u8>>,
    pub status: InspectionFileStatus,
    pub status_text: &'static str,
    pub installed: bool,
    pub link_conflict: bool,
    pub preset_missing: bool,
    pub changes: Vec<InspectionChange>,
}

#[derive(Clone, Debug, Default)]
pub struct DomainInspectionReport {
    pub app_files: Vec<AppFileInspection>,
    pub shell_files: Vec<ShellFileInspection>,
}

/// Validated local journal observation. Raw journal content never leaves its owning domain.
pub(crate) struct JournalInspection {
    pub operation_id: String,
    pub prepared_actions: u64,
    pub applied_actions: u64,
    pub receipt_committed_actions: u64,
    pub recovery_plan: crate::plan::PlanV1,
}
