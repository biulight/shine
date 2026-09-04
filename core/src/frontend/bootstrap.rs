//! One host-backed source capture for all frontend adapters.

use super::{FrontendService, FrontendServiceError};
use crate::runtime::{
    CoreRuntime, FileSystemHost, PresetSnapshotRequest, RuntimeContext, capture_preset_snapshot,
};

impl<H: FileSystemHost> FrontendService<H> {
    /// The distribution supplies resolved settings and embedded bytes; Core owns source capture.
    pub async fn capture(
        host: H,
        context: RuntimeContext,
        presets: PresetSnapshotRequest,
    ) -> Result<Self, FrontendServiceError> {
        let snapshot = capture_preset_snapshot(&host, presets)
            .await
            .map_err(|error| FrontendServiceError::new("frontend_capture_failed", error))?;
        Ok(Self::new(CoreRuntime::new(host, context, snapshot)))
    }
}

impl<H> FrontendService<H> {
    /// Optional opaque revision of distribution-owned configuration, never configuration bytes.
    pub fn with_configuration_revision(mut self, revision: Option<String>) -> Self {
        self.configuration_revision = revision;
        self
    }
}
