//! Compatibility re-exports for Sys runtime state now owned by `shine-core`.

#[cfg(test)]
pub(super) use shine_core::runtime::SYS_MANIFEST_FILE;
pub(super) use shine_core::runtime::SysRunEntry;
#[cfg(test)]
pub(super) use shine_core::runtime::SysRunManifest;
