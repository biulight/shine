//! Private policy persistence; locks cover the final read, recheck and write.
use super::model::{POLICY_VERSION, PolicyStore, RemoteEnrollmentPlan};
use super::policy::validate_store;
use crate::config::Config;
use anyhow::{Context, Result, bail};
use shine_core::runtime::{FileSystemHost, RealHost};
use std::path::{Path, PathBuf};

const POLICY_FILE: &str = "ssh-secret-broker.toml";

pub fn policy_path(config: &Config) -> PathBuf {
    config.shine_dir().join(POLICY_FILE)
}

pub async fn load_store(config: &Config) -> Result<PolicyStore> {
    load_store_from(&policy_path(config)).await
}

pub async fn load_stores(config: &Config, overrides: &[PathBuf]) -> Result<PolicyStore> {
    let mut merged = load_store(config).await?;
    for path in overrides {
        let mut extra = load_store_from(path).await?;
        merged.policies.append(&mut extra.policies);
    }
    validate_store(&merged)?;
    Ok(merged)
}

pub async fn load_store_from(path: &Path) -> Result<PolicyStore> {
    validate_policy_file(path, true).await?;
    let contents = match tokio::fs::read_to_string(path).await {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PolicyStore {
                version: POLICY_VERSION,
                policies: Vec::new(),
            });
        }
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let store: PolicyStore = toml::from_str(&contents)
        .map_err(crate::config::config_parse_error)
        .with_context(|| format!("parsing {}", path.display()))?;
    validate_store(&store)?;
    Ok(store)
}

pub(super) async fn save_store(config: &Config, store: &PolicyStore) -> Result<()> {
    validate_store(store)?;
    let path = policy_path(config);
    validate_policy_file(&path, true).await?;
    let contents = toml::to_string_pretty(store).context("serializing SSH secret broker policy")?;
    crate::persist::atomic_write_private(&path, contents.as_bytes()).await?;
    validate_policy_file(&path, false).await
}

async fn validate_policy_file(path: &Path, allow_missing: bool) -> Result<()> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(());
        }
        Err(error) => return Err(error).with_context(|| format!("inspecting {}", path.display())),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        bail!(
            "SSH secret broker policy must be a regular file, not a symlink: {}",
            path.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let expected_uid = unsafe { libc::geteuid() };
        if metadata.uid() != expected_uid {
            bail!(
                "SSH secret broker policy is not owned by the current user: {}",
                path.display()
            );
        }
        if metadata.mode() & 0o077 != 0 {
            bail!(
                "SSH secret broker policy permissions are too broad (expected 0600): {}",
                path.display()
            );
        }
    }
    Ok(())
}

pub async fn apply_remote_enrollment(
    config: &Config,
    plan: &RemoteEnrollmentPlan,
) -> Result<String> {
    let _guard = RealHost
        .acquire_operation_lock(&config.shine_dir().join("ssh-secret-broker.lock"))
        .await?;
    let mut store = load_store(config).await?;
    if let Some(previous) = &plan.previous {
        let current = store
            .policies
            .iter_mut()
            .find(|policy| policy.name == plan.name)
            .with_context(|| format!("policy {} no longer exists", plan.name))?;
        if current != previous {
            bail!(
                "policy {} changed after review; inspect and retry",
                plan.name
            );
        }
        *current = plan.candidate.clone();
    } else {
        if store.policies.iter().any(|item| item.name == plan.name) {
            bail!(
                "policy {} was created after review; inspect it explicitly",
                plan.name
            );
        }
        store.policies.push(plan.candidate.clone());
    }
    save_store(config, &store).await?;
    Ok(plan.name.clone())
}
