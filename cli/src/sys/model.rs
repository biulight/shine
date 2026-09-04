//! Compatibility exports while Sys command presentation remains in the CLI.

#[cfg(test)]
pub(crate) use shine_core::runtime::SysInstalledRow;
pub(super) use shine_core::runtime::{
    LoadedSysPreset, ResolvedSelection, SysDetection, SysDetectionProbe, SysDriverKind, SysInstall,
    SysItem, SysItemMode, SysItemOutcome, SysItemStatus, SysPackageProvider,
};
pub use shine_core::runtime::{SysUpdateRow, SysUpgradeReport};
