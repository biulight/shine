//! Named, local receipt handoffs; persisted V1 contracts and validation stay unchanged.
use super::*;

pub(crate) struct ShellReceiptTransition {
    pub target: String,
    pub previous: Option<ShellManifestEntry>,
    pub desired: ShellManifestEntry,
}
impl ShellReceiptTransition {
    pub(super) fn contract(&self) -> ShellReceiptTransitionV1 {
        ShellReceiptTransitionV1 {
            target: self.target.clone(),
            previous: self.previous.as_ref().map(receipt_contract).map(Box::new),
            desired: Box::new(receipt_contract(&self.desired)),
        }
    }
}

pub(crate) struct ShellReceiptRemoval {
    pub target: String,
    pub previous: ShellManifestEntry,
}
impl ShellReceiptRemoval {
    pub(super) fn contract(&self) -> ShellReceiptRemovalV1 {
        ShellReceiptRemovalV1 {
            target: self.target.clone(),
            previous: Box::new(receipt_contract(&self.previous)),
        }
    }
}
