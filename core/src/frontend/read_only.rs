//! The bounded capability supplied to AI-facing adapters.

use super::{
    CapabilityKindV1, FrontendDiagnosticV1, FrontendService, InspectionReportV1, InventoryReportV1,
    InventoryRequest, OperationStateReportV1, PlanReviewReportV1, ReviewRequest,
};
use crate::runtime::{
    FileSystemHost, FileSystemObservationHost, PrivilegedFileSystemHost, ProcessHost, SplitDnsHost,
    SplitDnsObservationHost,
};

/// No runtime accessor, generator evaluation, approval constructor or execution method.
///
/// ```compile_fail,E0599
/// use shine_core::{frontend::ReadOnlyFrontend, runtime::InMemoryHost};
/// fn cannot_get_runtime(view: ReadOnlyFrontend<'_, InMemoryHost>) {
///     view.runtime();
/// }
/// ```
///
/// ```compile_fail,E0599
/// use shine_core::{frontend::ReadOnlyFrontend, runtime::InMemoryHost};
/// fn cannot_gain_approval_authority(view: ReadOnlyFrontend<'_, InMemoryHost>) {
///     view.into_trusted();
/// }
/// ```
///
/// ```compile_fail,E0599
/// use shine_core::{frontend::{ReadOnlyFrontend, ApprovedOperation}, runtime::InMemoryHost};
/// fn cannot_apply(view: ReadOnlyFrontend<'_, InMemoryHost>, approved: ApprovedOperation) {
///     view.apply(approved);
/// }
/// ```
pub struct ReadOnlyFrontend<'a, H> {
    service: &'a FrontendService<H>,
}

impl<H> FrontendService<H> {
    pub fn read_only(&self) -> ReadOnlyFrontend<'_, H> {
        ReadOnlyFrontend { service: self }
    }
}

impl<H: FileSystemObservationHost> ReadOnlyFrontend<'_, H> {
    pub async fn inventory(
        &self,
        request: InventoryRequest,
    ) -> Result<InventoryReportV1, FrontendDiagnosticV1> {
        self.service
            .inventory(request)
            .await
            .map_err(|error| error.diagnostic().clone())
    }
}

impl<H: FileSystemObservationHost + SplitDnsObservationHost> ReadOnlyFrontend<'_, H> {
    pub async fn request_review(
        &self,
        request: &ReviewRequest,
    ) -> Result<PlanReviewReportV1, FrontendDiagnosticV1> {
        self.service
            .review(request)
            .await
            .map_err(|error| error.diagnostic().clone())
    }
    pub async fn operation_state(
        &self,
        kind: CapabilityKindV1,
    ) -> Result<OperationStateReportV1, FrontendDiagnosticV1> {
        self.service
            .operation_state(kind)
            .await
            .map_err(|error| error.diagnostic().clone())
    }
}

impl<H: FileSystemHost + PrivilegedFileSystemHost + ProcessHost> ReadOnlyFrontend<'_, H> {
    pub async fn inspect_apps(
        &self,
        categories: Vec<String>,
    ) -> Result<InspectionReportV1, FrontendDiagnosticV1> {
        self.service
            .inspect_apps(categories)
            .await
            .map(|inspection| inspection.report)
            .map_err(|error| error.diagnostic().clone())
    }
}

impl<H: FileSystemHost + PrivilegedFileSystemHost> ReadOnlyFrontend<'_, H> {
    pub async fn inspect_shells(&self) -> Result<InspectionReportV1, FrontendDiagnosticV1> {
        self.service
            .inspect_shells()
            .await
            .map(|inspection| inspection.report)
            .map_err(|error| error.diagnostic().clone())
    }
}

impl<H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost> ReadOnlyFrontend<'_, H> {
    pub async fn inspect_sys(
        &self,
        os_id: &str,
    ) -> Result<InspectionReportV1, FrontendDiagnosticV1> {
        self.service
            .inspect_sys(os_id)
            .await
            .map(|inspection| inspection.report)
            .map_err(|error| error.diagnostic().clone())
    }
}
