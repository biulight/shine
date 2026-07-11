mod commands;
mod detect;
mod drivers;
mod execution;
mod manifest;
mod model;
mod profile;
mod render;
mod resources;
mod run_manifest;
mod selection;

use commands::{SYS_STATUS_PREFIX, SysAction};
use model::{
    LoadedSysPreset, ResolvedSelection, SYS_PROFILE_PHASES, SelectionSource,
    ShellProfileBlockPosition, SysDriverKind, SysInitCommand, SysItem, SysItemMode, SysItemOutcome,
    SysItemStatus, SysManifest, SysProfilePhase, SysUpdateRow, SysUpgradeReport,
};
use render::sys_init_theme;

pub use commands::{
    handle_apply, handle_info, handle_init, handle_list, handle_status, handle_uninstall,
    handle_upgrade_managed, managed_updates,
};
pub use detect::detect_os_id;
