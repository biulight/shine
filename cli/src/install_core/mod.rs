//! Install/uninstall primitives shared by the `apps` and `sys` preset
//! domains: manifest tracking (`manifest`), file write/backup/admin-lock
//! logic (`file_ops`), and content transforms (`transforms`).
//!
//! This module is domain-neutral so `sys` doesn't need to reach into `apps`
//! for these primitives — both depend on `install_core` instead.

pub mod file_ops;
pub mod manifest;

pub use manifest::{AppEntry, AppInstallStrategy, AppManifest, hash_content};
pub use utils::install::transforms;
pub use utils::install::{apply_transforms, eol_eq, normalize_eol};
