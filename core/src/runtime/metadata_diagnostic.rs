//! Stable metadata diagnostic identity, independent of names and error prose.

use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MetadataDiagnosticKind {
    MissingReference,
    DuplicateCommand,
    DuplicateTarget,
    BunDependencyPolicy,
    MissingMetadata,
    InvalidMetadata,
}

impl MetadataDiagnosticKind {
    pub(super) const fn code(self) -> &'static str {
        match self {
            Self::MissingReference => "missing_reference",
            Self::DuplicateCommand => "duplicate_command",
            Self::DuplicateTarget => "duplicate_target",
            Self::BunDependencyPolicy => "bun_dependency_policy",
            Self::MissingMetadata => "missing_metadata",
            Self::InvalidMetadata => "invalid_metadata",
        }
    }
}

#[derive(Debug)]
pub(super) struct MetadataDiagnosticError {
    pub(super) kind: MetadataDiagnosticKind,
    message: String,
}

impl MetadataDiagnosticError {
    pub(super) fn new(kind: MetadataDiagnosticKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub(super) fn wrap(kind: MetadataDiagnosticKind, error: anyhow::Error) -> anyhow::Error {
        Self::new(kind, format!("{error:#}")).into()
    }
}

impl fmt::Display for MetadataDiagnosticError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for MetadataDiagnosticError {}
