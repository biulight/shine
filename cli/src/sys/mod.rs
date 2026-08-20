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

use commands::{SYS_STATUS_PREFIX, SYS_UPDATE_PREFIX};
use managed::SysAction;
use model::{
    LoadedSysPreset, ResolvedSelection, SYS_PROFILE_PHASES, SelectionSource,
    ShellProfileBlockPosition, SysDetection, SysDetectionProbe, SysDriverKind, SysInitCommand,
    SysInstall, SysInstalledRow, SysItem, SysItemMode, SysItemOutcome, SysItemStatus, SysManifest,
    SysPackageProvider, SysProfilePhase, SysShellIntegration, SysShellKind, SysUpdateCheck,
    SysUpdateRow, SysUpdateState, SysUpgradeReport,
};
use render::sys_init_theme;

pub use commands::{handle_info, handle_init, handle_list, handle_status, handle_update};
pub use detect::detect_os_id;
pub(crate) use managed::installed_managed;
pub use managed::{
    handle_apply, handle_uninstall, handle_upgrade_managed, handle_upgrade_managed_target,
    managed_updates,
};
pub use profile_commands::{handle_profile_disable, handle_profile_enable};
