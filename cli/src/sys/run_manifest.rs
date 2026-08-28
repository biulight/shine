//! Compatibility re-exports for Sys runtime state now owned by `shine-core`.

#[cfg(test)]
pub(super) use utils::runtime::SYS_MANIFEST_FILE;
pub(super) use utils::runtime::{SysRunEntry, SysRunManifest};
