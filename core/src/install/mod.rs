//! Reusable install and content primitives owned by `shine-core`.

pub mod file_ops;
mod line_endings;
pub mod manifest;
pub mod transforms;

pub use file_ops::{
    InstallOutcome, UninstallOutcome, backup_path, install_bytes_with_host,
    uninstall_entry_with_host,
};
#[cfg(test)]
pub use file_ops::{install_bytes, uninstall_entry};
pub use line_endings::{eol_eq, normalize_eol};
pub use manifest::{AppEntry, AppInstallStrategy, AppManifest, hash_content};
pub use transforms::apply as apply_transforms;
