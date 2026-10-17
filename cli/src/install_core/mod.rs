//! CLI re-exports of Core App manifest types, plus test-only coverage of
//! Core-owned file operations. Runtime transforms and ownership live in Core.

#[cfg(test)]
pub mod file_ops;
pub mod manifest;

pub use manifest::AppEntry;
#[cfg(test)]
pub use manifest::{AppInstallStrategy, hash_content};
