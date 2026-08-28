use crate::config::full_expand_with_home;
use crate::install_core::file_ops::{
    InstallOutcome, UninstallOutcome, install_bytes, install_bytes_admin, uninstall_entry,
    uninstall_entry_admin,
};
use crate::install_core::{AppEntry, AppInstallStrategy, apply_transforms, hash_content};
use crate::sys::resources::{
    DriverContext, ManagedFileReceipt, RECEIPT_VERSION, ResourceConflict, ResourceOutcome,
    SystemReceipt, config_string, optional_config_string,
};
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use utils::lifecycle::LifecycleEffect;

fn managed_file_desired(context: &DriverContext<'_>) -> Result<(PathBuf, Vec<u8>, Option<String>)> {
    let source = config_string(&context.item.config, "source")?;
    let target = config_string(&context.item.config, "target")?;
    let source = PathBuf::from(source);
    if source.is_absolute()
        || source
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("managed-file source must stay within its preset directory");
    }
    let source = context.preset_root.join(source);
    let canonical_root = std::fs::canonicalize(context.preset_root)
        .with_context(|| format!("reading preset root {}", context.preset_root.display()))?;
    let canonical_source =
        std::fs::canonicalize(&source).with_context(|| format!("reading {}", source.display()))?;
    if !canonical_source.starts_with(&canonical_root) {
        bail!("managed-file source must stay within its preset directory");
    }
    let target = PathBuf::from(
        full_expand_with_home(&target, &context.config.home_dir)
            .with_context(|| format!("expanding managed-file target `{target}`"))?,
    );
    if !target.is_absolute() {
        bail!("managed-file target must resolve to an absolute path");
    }
    let raw = std::fs::read(&canonical_source)
        .with_context(|| format!("reading {}", canonical_source.display()))?;
    let transforms = context
        .item
        .config
        .get("transforms")
        .and_then(toml::Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_string)
                        .context("managed-file transforms must be strings")
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    let content = if transforms.is_empty() {
        raw
    } else {
        apply_transforms(&transforms, &raw, context.env)?
    };
    Ok((
        target,
        content,
        optional_config_string(&context.item.config, "restart_hint"),
    ))
}

pub(in crate::sys) async fn managed_file_up_to_date(
    context: &DriverContext<'_>,
    previous: Option<&SystemReceipt>,
) -> Result<bool> {
    let Some(SystemReceipt::ManagedFile(previous)) = previous else {
        return Ok(false);
    };
    let (destination, content, _restart_hint) = managed_file_desired(context)?;
    if previous.destination != destination || !destination.exists() {
        return Ok(false);
    }
    let current = tokio::fs::read(&destination)
        .await
        .with_context(|| format!("reading {}", destination.display()))?;
    if hash_content(&current) != previous.content_hash {
        return Ok(false);
    }
    Ok(current == content)
}

pub(in crate::sys) async fn apply_managed_file(
    context: &DriverContext<'_>,
    previous: Option<&SystemReceipt>,
) -> Result<ResourceOutcome> {
    let (destination, content, restart_hint) = managed_file_desired(context)?;
    let previous = match previous {
        Some(SystemReceipt::ManagedFile(receipt)) => Some(receipt),
        Some(other) => bail!("managed-file received {:?} receipt", other.driver()),
        None => None,
    };
    if context.dry_run {
        return Ok(ResourceOutcome {
            changed: true,
            effects: vec![LifecycleEffect::ResourceWritePreviewed],
            detail: destination.display().to_string(),
            receipt: None,
            restart_hint,
        });
    }
    let mut effects = Vec::new();
    if let Some(previous) = previous {
        if previous.destination != destination {
            effects.extend(remove_managed_file(previous, false).await?.effects);
        } else if destination.exists() {
            let current = tokio::fs::read(&destination).await?;
            if hash_content(&current) != previous.content_hash {
                return Err(ResourceConflict::user_modified(format!(
                    "managed file {} was modified; keeping user content",
                    destination.display()
                ))
                .into());
            }
            if current == content {
                return Ok(ResourceOutcome {
                    changed: false,
                    effects: Vec::new(),
                    detail: destination.display().to_string(),
                    receipt: Some(SystemReceipt::ManagedFile(previous.clone())),
                    restart_hint: None,
                });
            }
        }
    }
    let is_managed = previous.is_some_and(|receipt| receipt.destination == destination);
    let outcome = if context.item.requires_admin {
        install_bytes_admin(&content, &destination, is_managed, false, true).await?
    } else {
        install_bytes(&content, &destination, is_managed, false, true).await?
    };
    let changed = !matches!(outcome, InstallOutcome::AlreadyManaged);
    if changed {
        effects.push(LifecycleEffect::ResourceWritten);
    }
    if matches!(outcome, InstallOutcome::BackedUpAndInstalled { .. }) {
        effects.push(LifecycleEffect::BackupCreated);
    }
    let backup = match outcome {
        InstallOutcome::BackedUpAndInstalled { backup, .. } => Some(backup),
        _ => previous.and_then(|receipt| receipt.backup.clone()),
    };
    let receipt = ManagedFileReceipt {
        version: RECEIPT_VERSION,
        destination: destination.clone(),
        backup,
        content_hash: hash_content(&content),
        privileged: context.item.requires_admin,
        restart_hint: restart_hint.clone(),
    };
    Ok(ResourceOutcome {
        changed,
        effects,
        detail: destination.display().to_string(),
        receipt: Some(SystemReceipt::ManagedFile(receipt)),
        restart_hint,
    })
}

pub(in crate::sys) async fn remove_managed_file(
    receipt: &ManagedFileReceipt,
    dry_run: bool,
) -> Result<ResourceOutcome> {
    let entry = AppEntry {
        source: "sys/managed-file".to_string(),
        destination: receipt.destination.clone(),
        backup: receipt.backup.clone(),
        content_hash: receipt.content_hash,
        install_strategy: AppInstallStrategy::Copy,
        uses_env: false,
        requires_admin: receipt.privileged,
    };
    let outcome = if receipt.privileged {
        uninstall_entry_admin(&entry, dry_run, false).await?
    } else {
        uninstall_entry(&entry, dry_run, false).await?
    };
    if matches!(outcome, UninstallOutcome::UserModified) {
        return Err(ResourceConflict::user_modified(format!(
            "managed file {} was modified; keeping user content",
            receipt.destination.display()
        ))
        .into());
    }
    let changed = !matches!(outcome, UninstallOutcome::NotFound);
    let effects = match outcome {
        UninstallOutcome::Removed | UninstallOutcome::ForceRemoved => {
            vec![LifecycleEffect::ResourceRemoved]
        }
        UninstallOutcome::RestoredBackup { .. } | UninstallOutcome::ForceRestoredBackup { .. } => {
            vec![LifecycleEffect::BackupRestored]
        }
        UninstallOutcome::DryRun => vec![LifecycleEffect::ResourceRemovePreviewed],
        UninstallOutcome::NotFound | UninstallOutcome::UserModified => Vec::new(),
    };
    Ok(ResourceOutcome {
        changed,
        effects,
        detail: receipt.destination.display().to_string(),
        receipt: None,
        restart_hint: receipt.restart_hint.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::sys::resources::{BuiltinDriver, SystemDriver};
    use crate::sys::{SysDriverKind, SysItem, SysItemMode};
    use std::collections::BTreeMap;
    use std::path::Path;

    async fn make_temp_dir() -> PathBuf {
        crate::test_support::make_temp_dir("shine-resource").await
    }

    fn managed_file_item(source: &str, target: &Path) -> SysItem {
        let mut config = toml::Table::new();
        config.insert(
            "source".to_string(),
            toml::Value::String(source.to_string()),
        );
        config.insert(
            "target".to_string(),
            toml::Value::String(target.display().to_string()),
        );
        config.insert(
            "restart_hint".to_string(),
            toml::Value::String("Restart sample".to_string()),
        );
        SysItem {
            id: "sample-file".to_string(),
            label: "Sample file".to_string(),
            description: String::new(),
            default: false,
            mode: SysItemMode::Managed,
            requires_admin: false,
            required_env: Vec::new(),
            driver: SysDriverKind::ManagedFile,
            config,
            detect: None,
            install: None,
            shell: Vec::new(),
        }
    }

    #[tokio::test]
    async fn managed_file_backs_up_converges_and_restores() {
        let dir = make_temp_dir().await;
        let source = dir.join("source.txt");
        let destination = dir.join("destination.txt");
        tokio::fs::write(&source, "desired").await.unwrap();
        tokio::fs::write(&destination, "original").await.unwrap();
        let config = Config::new_for_test(&dir);
        let item = managed_file_item("source.txt", &destination);
        let env = BTreeMap::new();
        let context = DriverContext {
            config: &config,
            os_id: "fakeos",
            item: &item,
            preset_root: &dir,
            env: &env,
            dry_run: false,
        };
        let driver = BuiltinDriver::new(SysDriverKind::ManagedFile);

        let installed = driver.apply(&context, None).await.unwrap();
        assert!(installed.changed);
        assert_eq!(
            tokio::fs::read_to_string(&destination).await.unwrap(),
            "desired"
        );
        assert_eq!(installed.restart_hint.as_deref(), Some("Restart sample"));
        let receipt = installed.receipt.unwrap();

        let unchanged = driver.apply(&context, Some(&receipt)).await.unwrap();
        assert!(!unchanged.changed);
        assert!(unchanged.restart_hint.is_none());

        tokio::fs::write(&destination, "user edit").await.unwrap();
        let error = driver.apply(&context, Some(&receipt)).await.unwrap_err();
        assert!(error.to_string().contains("modified"));

        tokio::fs::write(&destination, "desired").await.unwrap();
        let removed = driver
            .remove(Some(&context), &receipt, false)
            .await
            .unwrap();
        assert!(removed.changed);
        assert_eq!(
            tokio::fs::read_to_string(&destination).await.unwrap(),
            "original"
        );

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn managed_file_is_up_to_date_matches_apply_convergence() {
        let dir = make_temp_dir().await;
        let source = dir.join("source.txt");
        let destination = dir.join("destination.txt");
        tokio::fs::write(&source, "desired").await.unwrap();
        let config = Config::new_for_test(&dir);
        let item = managed_file_item("source.txt", &destination);
        let env = BTreeMap::new();
        let context = DriverContext {
            config: &config,
            os_id: "fakeos",
            item: &item,
            preset_root: &dir,
            env: &env,
            dry_run: false,
        };
        let driver = BuiltinDriver::new(SysDriverKind::ManagedFile);

        assert!(!driver.is_up_to_date(&context, None).await.unwrap());

        let installed = driver.apply(&context, None).await.unwrap();
        let receipt = installed.receipt.unwrap();
        assert!(
            driver
                .is_up_to_date(&context, Some(&receipt))
                .await
                .unwrap()
        );

        tokio::fs::write(&source, "changed").await.unwrap();
        assert!(
            !driver
                .is_up_to_date(&context, Some(&receipt))
                .await
                .unwrap()
        );

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
