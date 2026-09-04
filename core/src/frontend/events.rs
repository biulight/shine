//! Explicit safe progress projection. Raw runtime events remain local-only.

use crate::plan::PlanV1;
use crate::runtime::{RuntimeEvent, RuntimeObserver, SysItemStatus};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const FRONTEND_EVENT_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrontendEventKindV1 {
    Section,
    Progress,
    Warning,
    ProcessOutputAvailable,
    Interaction,
    BootstrapSelection,
    BootstrapItemStarted,
    BootstrapItemFinished,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrontendEventStatusV1 {
    Installed,
    AlreadyInstalled,
    Skipped,
    Updated,
    NeedsAction,
    Completed,
    Failed,
}

impl From<SysItemStatus> for FrontendEventStatusV1 {
    fn from(status: SysItemStatus) -> Self {
        match status {
            SysItemStatus::Installed => Self::Installed,
            SysItemStatus::AlreadyInstalled => Self::AlreadyInstalled,
            SysItemStatus::Skipped => Self::Skipped,
            SysItemStatus::Updated => Self::Updated,
            SysItemStatus::NeedsAction => Self::NeedsAction,
            SysItemStatus::Completed => Self::Completed,
            SysItemStatus::Failed => Self::Failed,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FrontendEventV1 {
    pub schema_version: u32,
    /// Monotonic within this execution observer, not a durable global cursor.
    pub sequence: u64,
    pub kind: FrontendEventKindV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<FrontendEventStatusV1>,
}

pub trait FrontendEventSink {
    fn emit(&mut self, event: FrontendEventV1);
}

impl FrontendEventSink for Vec<FrontendEventV1> {
    fn emit(&mut self, event: FrontendEventV1) {
        self.push(event);
    }
}

/// The allow-list comes from the generated review Plan, never event payloads.
pub struct EventProjector {
    targets: BTreeSet<String>,
    sequence: u64,
}

impl EventProjector {
    pub fn for_plan(plan: &PlanV1) -> Self {
        Self {
            targets: plan
                .steps
                .iter()
                .map(|step| step.target.clone())
                .filter(|target| canonical_event_target(target))
                .collect(),
            sequence: 0,
        }
    }

    pub fn project(&mut self, event: &RuntimeEvent) -> FrontendEventV1 {
        use FrontendEventKindV1 as Kind;
        let (kind, target, status) = match event {
            RuntimeEvent::Section { .. } => (Kind::Section, None, None),
            RuntimeEvent::Progress { target, .. } => (Kind::Progress, Some(target.clone()), None),
            RuntimeEvent::Warning { target, .. } => (Kind::Warning, target.clone(), None),
            RuntimeEvent::ProcessOutput { target, .. } => {
                (Kind::ProcessOutputAvailable, Some(target.clone()), None)
            }
            RuntimeEvent::Interaction { target, .. } => {
                (Kind::Interaction, Some(target.clone()), None)
            }
            RuntimeEvent::SysBootstrapSelection { .. } => (Kind::BootstrapSelection, None, None),
            RuntimeEvent::SysBootstrapItemStart { item_id, .. } => (
                Kind::BootstrapItemStarted,
                Some(format!("sys/{item_id}")),
                None,
            ),
            RuntimeEvent::SysBootstrapOutcome(outcome) => (
                Kind::BootstrapItemFinished,
                Some(format!("sys/{}", outcome.item_id)),
                Some(outcome.status.into()),
            ),
        };
        let projected = FrontendEventV1 {
            schema_version: FRONTEND_EVENT_SCHEMA_VERSION,
            sequence: self.sequence,
            kind,
            target: target.filter(|target| self.targets.contains(target)),
            status,
        };
        self.sequence = self.sequence.saturating_add(1);
        projected
    }
}

fn canonical_event_target(target: &str) -> bool {
    let parts = target.split('/').collect::<Vec<_>>();
    let count = match parts.first().copied() {
        Some("app" | "sys") => 2,
        Some("shell") => 3,
        _ => return false,
    };
    parts.len() == count
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.len() <= 256
                && !matches!(*part, "." | "..")
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        })
}

/// Fan out safe events to a frontend and unchanged private events to local presentation.
pub struct ProjectedObserver<'a, O, S> {
    projector: EventProjector,
    local: &'a mut O,
    safe: &'a mut S,
}

impl<'a, O: RuntimeObserver, S: FrontendEventSink> ProjectedObserver<'a, O, S> {
    pub fn new(plan: &PlanV1, local: &'a mut O, safe: &'a mut S) -> Self {
        Self {
            projector: EventProjector::for_plan(plan),
            local,
            safe,
        }
    }
}

impl<O: RuntimeObserver, S: FrontendEventSink> RuntimeObserver for ProjectedObserver<'_, O, S> {
    fn emit(&mut self, event: RuntimeEvent) {
        self.safe.emit(self.projector.project(&event));
        self.local.emit(event);
    }
}
