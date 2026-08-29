//! Install/uninstall primitives shared by the `apps` and `sys` preset
//! domains: manifest tracking (`manifest`), file write/backup/admin-lock
//! logic (`file_ops`), and content transforms (`transforms`).
//!
//! This module is domain-neutral so `sys` doesn't need to reach into `apps`
//! for these primitives — both depend on `install_core` instead.

#[cfg(test)]
pub mod file_ops;
pub mod manifest;

pub use manifest::AppEntry;
#[cfg(test)]
pub use manifest::{AppInstallStrategy, AppManifest, hash_content};
#[cfg(test)]
pub use shine_core::install::transforms;
