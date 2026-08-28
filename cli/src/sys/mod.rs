mod bootstrap;
mod commands;
mod detect;
mod drivers;
mod execution;
mod managed;
mod manifest;
mod model;
mod profile;
mod profile_blocks;
mod profile_commands;
mod profile_compose;
mod render;
mod resources;
mod run_manifest;
mod selection;

use model::{
    LoadedSysPreset, ResolvedSelection, SYS_PROFILE_PHASES, SelectionSource,
    ShellProfileBlockPosition, SysDetection, SysDetectionProbe, SysDriverKind, SysInstall,
    SysInstalledRow, SysItem, SysItemMode, SysItemOutcome, SysItemStatus, SysManifest,
    SysPackageProvider, SysProfilePhase, SysShellIntegration, SysShellKind, SysUpdateRow,
    SysUpgradeReport,
};
use render::sys_init_theme;

use anyhow::{Context, Result};
use std::path::Path;

pub use commands::{handle_info, handle_init, handle_list, handle_status};
pub use detect::detect_os_id;
pub(crate) use managed::installed_managed;
pub use managed::{
    handle_apply, handle_uninstall, handle_upgrade_managed, handle_upgrade_managed_target,
    managed_updates,
};
pub(crate) use managed::{
    handle_upgrade_managed_target_with_result, handle_upgrade_managed_with_result,
};
pub use profile_commands::{handle_profile_disable, handle_profile_enable};

const SYS_TEMPLATE: &str = r#"# System bootstrap preset metadata for shine (schema v2).
version = 2
description = "My system bootstrap."
default_profile = "recommended"

[[items]]
id = "my-tool"
label = "My Tool"
description = "Install and configure my tool."
default = true
detect = { kind = "command", command = "my-tool", version_args = ["--version"] }
# Keep scripts inside this category. For Windows, use a .ps1 script instead.
install = { kind = "script", path = "install/my-tool.sh" }

[profiles.recommended]
items = ["my-tool"]
"#;

pub async fn handle_init_template(force: bool) -> Result<()> {
    let dir = std::env::current_dir().context("reading current directory")?;
    let (path, overwritten) =
        utils::init_template::write_shine_toml_template(&dir, force, SYS_TEMPLATE)?;
    if overwritten {
        println!("Updated sys preset template: {}", path.display());
    } else {
        println!("Created sys preset template: {}", path.display());
    }
    Ok(())
}

pub(crate) fn validate_preset_category(
    name: &str,
    root: &Path,
) -> std::result::Result<bool, crate::preset_validation::PresetValidationFailure> {
    manifest::validate_preset_category(name, root)?;
    Ok(true)
}
