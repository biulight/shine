//! Shell inspection-to-row projection; identity and decisions never depend on display text.
use super::{FileStatus, UpdateChange};
use crate::{colors, config::Config};
use anyhow::Result;
use shine_core::runtime::ShellFileInspection;

pub struct ShellRow {
    /// Shell preset category owning this command row. Lifecycle commands act
    /// on this category; `label` remains the command-level diagnostic target.
    pub category: String,
    pub(crate) command_name: String,
    pub(crate) file_status: FileStatus,
    pub symbol: String,
    pub label: String,
    pub status_sym: &'static str,
    pub status_text: &'static str,
    /// `true` when at least one of preset-file or bin-symlink exists.
    pub is_installed: bool,
    /// Existing launcher is outside Shine's ownership proof and must be
    /// preserved rather than reported as an applicable update.
    pub(crate) link_conflict: bool,
    pub(crate) preset_missing: bool,
    pub(crate) changes: Vec<UpdateChange>,
}

/// CLI presentation severity, retaining the existing symbol groups and tie behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShellRowStatus {
    NotInstalled,
    UpToDate,
    UpdateAvail,
    Partial,
    Attention,
}

impl ShellRowStatus {
    fn for_file(status: FileStatus, link_conflict: bool, preset_missing: bool) -> Self {
        if link_conflict || preset_missing {
            return Self::Attention;
        }
        match status {
            FileStatus::NotInstalled => Self::NotInstalled,
            FileStatus::UpToDate => Self::UpToDate,
            FileStatus::UpdateAvail => Self::UpdateAvail,
            FileStatus::Missing => Self::Attention,
            FileStatus::Partial
            | FileStatus::UserModified
            | FileStatus::GeneratorNotEvaluated
            | FileStatus::GeneratorEvaluationFailed
            | FileStatus::GeneratorTrustRequired => Self::Partial,
        }
    }

    pub(crate) fn symbol(self) -> &'static str {
        match self {
            Self::NotInstalled => "✗",
            Self::UpToDate => "✓",
            Self::UpdateAvail => "↑",
            Self::Partial => "~",
            Self::Attention => "!",
        }
    }

    pub(crate) fn priority(self) -> usize {
        match self {
            Self::NotInstalled => 0,
            Self::UpToDate => 1,
            Self::UpdateAvail => 2,
            Self::Partial => 3,
            Self::Attention => 4,
        }
    }
}

impl ShellRow {
    pub(crate) fn selector(&self) -> String {
        format!("{}/{}", self.category, self.command_name)
    }

    pub(crate) fn target(&self) -> String {
        // Preserve terminal-component normalization for non-canonical legacy receipt commands,
        // but take it from the command identity rather than a potentially customized label.
        let command = self
            .command_name
            .split('/')
            .next_back()
            .unwrap_or(&self.command_name);
        format!("shell/{}/{}", self.category, command)
    }

    pub(crate) fn status(&self) -> ShellRowStatus {
        ShellRowStatus::for_file(self.file_status, self.link_conflict, self.preset_missing)
    }
}

impl From<ShellFileInspection> for ShellRow {
    fn from(file: ShellFileInspection) -> Self {
        let status_sym =
            ShellRowStatus::for_file(file.status, file.link_conflict, file.preset_missing).symbol();
        Self {
            category: file.category.name.clone(),
            command_name: file.file.command_name.clone(),
            file_status: file.status,
            symbol: colors::symbol(status_sym),
            label: format!("{}/{}", file.category.name, file.file.command_name),
            status_sym,
            status_text: file.status_text,
            is_installed: file.installed,
            link_conflict: file.link_conflict,
            preset_missing: file.preset_missing,
            changes: file.changes,
        }
    }
}

/// Build Shell rows without changing inspection capture or observation order.
pub async fn build_shell_rows(config: &Config) -> Result<Vec<ShellRow>> {
    let inspections = crate::core_runtime::frontend_from_config(config)
        .await?
        .inspect_shells()
        .await
        .map_err(shine_core::frontend::FrontendServiceError::into_source)?
        .files;
    Ok(inspections.into_iter().map(ShellRow::from).collect())
}

#[cfg(test)]
mod tests;
