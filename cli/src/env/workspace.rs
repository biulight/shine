mod broker_snapshot;
mod cache;
mod compilation;
mod dotenv;
mod model;
mod run;
mod sealing;
pub(crate) use broker_snapshot::declared_secrets_from_source;
pub use broker_snapshot::decrypt_broker_snapshot;
use broker_snapshot::merge_broker_values;
pub use broker_snapshot::snapshot_for_broker;
use cache::cache_path;
use cache::read_valid_cache;
use cache::write_cache;
use compilation::CapturedSources;
use compilation::capture_sources;
use compilation::compile_captured_sources;
use compilation::compile_export_sources;
#[cfg(test)]
use compilation::compile_source_layers;
use compilation::decrypt_source_payload;
use compilation::involves_hybrid;
use compilation::snapshot_input_hash;
#[cfg(test)]
use dotenv::dotenv_modes;
pub use dotenv::handle_export;
pub use dotenv::handle_init_from_dotenv;
#[cfg(test)]
use dotenv::init_from_dotenv_at;
#[cfg(test)]
use dotenv::parse_dotenv;
#[cfg(test)]
use dotenv::render_dotenv;
#[cfg(test)]
use dotenv::render_source;
use model::CacheFile;
use model::CachedMode;
use model::Encryption;
use model::SecretPayload;
use model::SecretState;
use model::SourceFile;
pub use model::Workspace;
pub use model::WorkspaceEnv;
use model::parse_source;
use model::parse_workspace;
pub(crate) use model::validate_broker_mode;
use model::validate_mode;
pub use run::handle_run;
use run::load_run_values_effective;
#[cfg(test)]
use run::merge_explicit;
pub(super) use run::resolve_explicit_values;
#[cfg(test)]
use run::run_command;
#[cfg(test)]
use sealing::SealLock;
pub use sealing::handle_seal;
#[cfg(test)]
use sealing::replace_sealed_file;
use sealing::resolve_seal_encryption;

pub mod file_groups;
mod local_cache;
mod sources;
mod values;
use sources::{Binding, effective_bindings};
pub(super) use values::SensitiveValues;

use super::broker::{SourceSnapshot, WorkspaceSnapshot};
use crate::commands::EnvWorkspaceExportFormat;
use crate::persist::{
    atomic_write, atomic_write_new, atomic_write_private, atomic_write_private_new,
};
use crate::secret::{BackendKind, EncryptRecipients};
use crate::{config::Config, secret};
use anyhow::{Context, Result, bail};
use dialoguer::Password;
use directories::BaseDirs;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::{Path, PathBuf},
};
use tokio::process::Command;
use toml_edit::{DocumentMut, value};
use zeroize::{Zeroize, Zeroizing};

const WORKSPACE_FILE: &str = "shine.workspace.toml";
const WORKSPACE_FORMAT_VERSION: u32 = 2;
const ENV_SOURCE_FORMAT_VERSION: u32 = 1;
const SECRET_PAYLOAD_VERSION: u32 = 1;
const CACHE_FORMAT_VERSION: u32 = 1;

async fn find_workspace_optional(explicit: Option<&Path>) -> Result<Option<PathBuf>> {
    if let Some(path) = explicit {
        return Ok(Some(absolute_from_current(path)?));
    }
    let current = std::env::current_dir().context("reading current directory")?;
    Ok(current
        .ancestors()
        .map(|directory| directory.join(WORKSPACE_FILE))
        .find(|path| path.is_file()))
}

async fn load_workspace(path: &Path) -> Result<Workspace> {
    let contents = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    parse_workspace(path, &contents)
}

fn clean_recipients(recipients: &[String]) -> Vec<String> {
    recipients
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn resolve_sources(workspace_path: &Path, files: &[String], mode: &str) -> Result<Vec<PathBuf>> {
    let root = workspace_path
        .parent()
        .context("workspace path has no parent directory")?;
    files
        .iter()
        .map(|file| {
            if file.contains("{mode}") && mode.is_empty() {
                bail!("cannot expand {file} without a mode");
            }
            let expanded = file.replace("{mode}", mode);
            let path = PathBuf::from(expanded);
            Ok(if path.is_absolute() {
                path
            } else {
                root.join(path)
            })
        })
        .collect()
}

pub(super) fn command_builder(
    command: &[OsString],
    values: &BTreeMap<String, String>,
    override_process_env: bool,
    explicit: &BTreeMap<String, String>,
) -> Result<Command> {
    let (program, args) = command
        .split_first()
        .context("a command is required after --")?;
    let mut child = Command::new(program);
    child.args(args);
    for (key, value) in values {
        if override_process_env || std::env::var_os(key).is_none() {
            child.env(key, value);
        }
    }
    child.envs(explicit);
    Ok(child)
}

pub(super) fn finish_command_status(status: std::process::ExitStatus) -> Result<()> {
    if status.success() {
        return Ok(());
    }
    if let Some(code) = status.code() {
        std::process::exit(code);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        std::process::exit(128 + status.signal().unwrap_or(1));
    }
    #[cfg(not(unix))]
    std::process::exit(1);
}

fn absolute_from_current(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .context("reading current directory")?
            .join(path))
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod hybrid_snapshot_tests;
