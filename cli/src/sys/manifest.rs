//! CLI filesystem adapter for Core-owned Sys manifest parsing.

use anyhow::Result;
use utils::runtime::SysManifest;

pub(super) fn parse_and_validate_manifest(content: &str) -> Result<SysManifest> {
    utils::runtime::parse_sys_manifest(content)
}
