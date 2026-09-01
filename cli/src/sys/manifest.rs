//! CLI filesystem adapter for Core-owned Sys manifest parsing.

use anyhow::Result;
use shine_core::runtime::SysManifest;

pub(super) fn parse_and_validate_manifest(content: &str) -> Result<SysManifest> {
    shine_core::runtime::parse_sys_manifest(content)
}
