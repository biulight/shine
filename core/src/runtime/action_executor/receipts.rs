//! App receipts.

use super::*;

pub(super) async fn load_app_manifest_receipts(
    host: &impl FileSystemObservationHost,
    shine_dir: &Path,
) -> Result<(AppManifest, Option<Vec<u8>>)> {
    let bytes = read_optional(host, &shine_dir.join("app-manifest.toml")).await?;
    let mut manifest: AppManifest = bytes
        .as_deref()
        .map(toml::from_slice)
        .transpose()
        .context("failed to parse app manifest")?
        .unwrap_or_default();
    match manifest.schema_version {
        0 => manifest.schema_version = APP_MANIFEST_SCHEMA_VERSION,
        APP_MANIFEST_SCHEMA_VERSION => {}
        version => bail!(
            "app manifest schema version {version} is newer than this Shine supports ({APP_MANIFEST_SCHEMA_VERSION})"
        ),
    }
    Ok((manifest, bytes))
}

pub(super) fn matching_app_receipt(
    manifest: &AppManifest,
    action: &crate::action::DeclarativeActionV1,
) -> bool {
    let source = action_source_identity(action);
    manifest
        .find_by_source(&source)
        .is_some_and(|entry| match &action.kind {
            ActionKindV1::CreateManagedFile {
                destination,
                desired_hash,
                requires_admin,
            } => {
                entry.destination == *destination
                    && entry.content_hash == *desired_hash
                    && entry.backup.is_none()
                    && entry.install_strategy == AppInstallStrategy::Copy
                    && entry.requires_admin == *requires_admin
            }
            ActionKindV1::CreateManagedFileWithBackup {
                destination,
                backup,
                desired_hash,
                requires_admin,
                ..
            } => {
                entry.destination == *destination
                    && entry.content_hash == *desired_hash
                    && entry.backup.as_ref() == Some(backup)
                    && entry.install_strategy == AppInstallStrategy::Copy
                    && entry.requires_admin == *requires_admin
            }
            ActionKindV1::UpdateManagedFile {
                destination,
                previous_backup,
                desired_hash,
                requires_admin,
                ..
            } => {
                entry.destination == *destination
                    && entry.content_hash == *desired_hash
                    && entry.backup == *previous_backup
                    && entry.install_strategy == AppInstallStrategy::Copy
                    && entry.requires_admin == *requires_admin
            }
            ActionKindV1::RelocateManagedFile {
                desired_destination,
                desired_hash,
                desired_uses_env,
                desired_requires_admin,
                ..
            } => {
                entry.destination == *desired_destination
                    && entry.content_hash == *desired_hash
                    && entry.backup.is_none()
                    && entry.install_strategy == AppInstallStrategy::Copy
                    && entry.uses_env == *desired_uses_env
                    && entry.requires_admin == *desired_requires_admin
            }
            ActionKindV1::RelocateManagedJson {
                desired_destination,
                desired_managed_hash,
                desired_managed_keys,
                desired_uses_env,
                ..
            } => {
                entry.destination == *desired_destination
                    && entry.content_hash == *desired_managed_hash
                    && entry.backup.is_none()
                    && entry.install_strategy
                        == AppInstallStrategy::JsonMerge {
                            managed_keys: desired_managed_keys.clone(),
                        }
                    && entry.uses_env == *desired_uses_env
                    && !entry.requires_admin
            }
            ActionKindV1::MergeManagedJson {
                destination,
                desired_managed_hash,
                managed_keys,
                ..
            } => {
                entry.destination == *destination
                    && entry.content_hash == *desired_managed_hash
                    && entry.backup.is_none()
                    && entry.install_strategy
                        == AppInstallStrategy::JsonMerge {
                            managed_keys: managed_keys.clone(),
                        }
                    && !entry.requires_admin
            }
            ActionKindV1::RemoveManagedFile { .. } => false,
            ActionKindV1::RemoveManagedFileWithBackup { .. } => false,
            ActionKindV1::ForceRemoveManagedFile { .. } => false,
            ActionKindV1::RemoveManagedJson { .. } => false,
            ActionKindV1::CreateShellLauncher { .. } => false,
            ActionKindV1::UpdateShellLauncher { .. } => false,
            ActionKindV1::RemoveShellLauncher { .. } => false,
            ActionKindV1::RemoveLegacyShellLauncher { .. } => false,
            ActionKindV1::ReplaceShellSnapshot { .. } => false,
            ActionKindV1::ReplaceShellCache { .. } => false,
            ActionKindV1::RemoveShellCache { .. } => false,
            ActionKindV1::RemoveShellSnapshot { .. } => false,
            ActionKindV1::ReconcileShellProfile { .. } => false,
            ActionKindV1::ReconcileSysSplitDns { .. } => false,
            ActionKindV1::ReconcileSysProfileBlocks { .. } => false,
            ActionKindV1::ReplaceShellRenderedFile { .. } => false,
            ActionKindV1::RemoveShellRenderedFile { .. } => false,
            ActionKindV1::OpaqueExecution { .. } => false,
        })
}

pub(super) fn matching_previous_app_receipt(
    manifest: &AppManifest,
    action: &crate::action::DeclarativeActionV1,
) -> bool {
    if let ActionKindV1::RelocateManagedFile {
        previous_destination,
        previous_backup,
        previous_hash,
        previous_uses_env,
        previous_requires_admin,
        ..
    } = &action.kind
    {
        let source = action_source_identity(action);
        return manifest.find_by_source(&source).is_some_and(|entry| {
            entry.destination == *previous_destination
                && entry.content_hash == *previous_hash
                && entry.backup.as_ref() == previous_backup.as_ref().map(|backup| &backup.path)
                && entry.install_strategy == AppInstallStrategy::Copy
                && entry.uses_env == *previous_uses_env
                && entry.requires_admin == *previous_requires_admin
        });
    }
    if let ActionKindV1::RelocateManagedJson {
        previous_destination,
        previous_receipt_hash,
        previous_managed_keys,
        previous_uses_env,
        ..
    } = &action.kind
    {
        let source = action_source_identity(action);
        return manifest.find_by_source(&source).is_some_and(|entry| {
            entry.destination == *previous_destination
                && entry.content_hash == *previous_receipt_hash
                && entry.backup.is_none()
                && entry.install_strategy
                    == AppInstallStrategy::JsonMerge {
                        managed_keys: previous_managed_keys.clone(),
                    }
                && entry.uses_env == *previous_uses_env
                && !entry.requires_admin
        });
    }
    if let ActionKindV1::MergeManagedJson {
        destination,
        previous_receipt_hash: Some(previous_receipt_hash),
        managed_keys,
        ..
    } = &action.kind
    {
        let source = action_source_identity(action);
        return manifest.find_by_source(&source).is_some_and(|entry| {
            entry.destination == *destination
                && entry.content_hash == *previous_receipt_hash
                && entry.backup.is_none()
                && entry.install_strategy
                    == AppInstallStrategy::JsonMerge {
                        managed_keys: managed_keys.clone(),
                    }
                && !entry.requires_admin
        });
    }
    if let ActionKindV1::RemoveManagedJson {
        destination,
        receipt_managed_hash,
        managed_keys,
        uses_env,
        ..
    } = &action.kind
    {
        let source = action_source_identity(action);
        return manifest.find_by_source(&source).is_some_and(|entry| {
            entry.destination == *destination
                && entry.content_hash == *receipt_managed_hash
                && entry.backup.is_none()
                && entry.install_strategy
                    == AppInstallStrategy::JsonMerge {
                        managed_keys: managed_keys.clone(),
                    }
                && entry.uses_env == *uses_env
                && !entry.requires_admin
        });
    }
    let (destination, previous_backup, original_hash, uses_env, requires_admin) = match &action.kind
    {
        ActionKindV1::UpdateManagedFile {
            destination,
            previous_backup,
            original_hash,
            requires_admin,
            ..
        } => (
            destination,
            previous_backup.as_ref(),
            original_hash,
            None,
            *requires_admin,
        ),
        ActionKindV1::RemoveManagedFile {
            destination,
            original_hash,
            uses_env,
            requires_admin,
            ..
        } => (
            destination,
            None,
            original_hash,
            Some(*uses_env),
            *requires_admin,
        ),
        ActionKindV1::RemoveManagedFileWithBackup {
            destination,
            backup,
            managed_hash,
            uses_env,
            requires_admin,
            ..
        } => (
            destination,
            Some(backup),
            managed_hash,
            Some(*uses_env),
            *requires_admin,
        ),
        ActionKindV1::ForceRemoveManagedFile {
            destination,
            persistent_backup,
            receipt_hash,
            uses_env,
            requires_admin,
            ..
        } => (
            destination,
            persistent_backup.as_ref().map(|backup| &backup.path),
            receipt_hash,
            Some(*uses_env),
            *requires_admin,
        ),
        ActionKindV1::MergeManagedJson { .. }
        | ActionKindV1::RelocateManagedJson { .. }
        | ActionKindV1::RemoveManagedJson { .. } => {
            return false;
        }
        _ => return false,
    };
    let source = action_source_identity(action);
    manifest.find_by_source(&source).is_some_and(|entry| {
        entry.destination == *destination
            && entry.content_hash == *original_hash
            && entry.backup.as_ref() == previous_backup
            && entry.install_strategy == AppInstallStrategy::Copy
            && entry.requires_admin == requires_admin
            && uses_env.is_none_or(|uses_env| entry.uses_env == uses_env)
    })
}

pub(super) fn previous_removed_app_receipt(
    action: &crate::action::DeclarativeActionV1,
) -> Result<crate::install::AppEntry> {
    let (destination, backup, original_hash, uses_env, requires_admin) = match &action.kind {
        ActionKindV1::RemoveManagedFile {
            destination,
            original_hash,
            uses_env,
            requires_admin,
            ..
        } => (destination, None, original_hash, uses_env, requires_admin),
        ActionKindV1::RemoveManagedFileWithBackup {
            destination,
            backup,
            managed_hash,
            uses_env,
            requires_admin,
            ..
        } => (
            destination,
            Some(backup.clone()),
            managed_hash,
            uses_env,
            requires_admin,
        ),
        ActionKindV1::ForceRemoveManagedFile {
            destination,
            persistent_backup,
            receipt_hash,
            uses_env,
            requires_admin,
            ..
        } => (
            destination,
            persistent_backup.as_ref().map(|backup| backup.path.clone()),
            receipt_hash,
            uses_env,
            requires_admin,
        ),
        ActionKindV1::RemoveManagedJson {
            destination,
            receipt_managed_hash,
            managed_keys,
            uses_env,
            ..
        } => {
            return Ok(crate::install::AppEntry {
                source: action_source_identity(action),
                destination: destination.clone(),
                backup: None,
                content_hash: *receipt_managed_hash,
                install_strategy: AppInstallStrategy::JsonMerge {
                    managed_keys: managed_keys.clone(),
                },
                uses_env: *uses_env,
                requires_admin: false,
            });
        }
        _ => bail!("only a managed-file removal has a restorable previous receipt"),
    };
    Ok(crate::install::AppEntry {
        source: action_source_identity(action),
        destination: destination.clone(),
        backup,
        content_hash: *original_hash,
        install_strategy: AppInstallStrategy::Copy,
        uses_env: *uses_env,
        requires_admin: *requires_admin,
    })
}

pub(super) fn removed_app_receipt_committed(
    manifest: &AppManifest,
    action: &crate::action::DeclarativeActionV1,
) -> bool {
    let (destination, backup, rollback) = match &action.kind {
        ActionKindV1::RemoveManagedFile {
            destination,
            rollback,
            ..
        } => (destination, None, rollback),
        ActionKindV1::RemoveManagedFileWithBackup {
            destination,
            backup,
            rollback,
            ..
        } => (destination, Some(backup), rollback),
        ActionKindV1::ForceRemoveManagedFile {
            destination,
            persistent_backup,
            rollback,
            ..
        } => (
            destination,
            persistent_backup.as_ref().map(|backup| &backup.path),
            rollback,
        ),
        ActionKindV1::RemoveManagedJson {
            destination,
            rollback,
            ..
        } => (destination, None, rollback),
        _ => return false,
    };
    let source = action_source_identity(action);
    manifest.find_by_source(&source).is_none()
        && manifest.find_by_dest(destination).is_none()
        && manifest.find_by_dest(rollback).is_none()
        && backup.is_none_or(|backup| manifest.find_by_dest(backup).is_none())
}

pub(super) fn removal_receipt_conflict(
    manifest: &AppManifest,
    action: &crate::action::DeclarativeActionV1,
    receipt_committed: bool,
) -> bool {
    let rollback = match &action.kind {
        ActionKindV1::RemoveManagedFile { rollback, .. }
        | ActionKindV1::RemoveManagedFileWithBackup { rollback, .. }
        | ActionKindV1::ForceRemoveManagedFile { rollback, .. }
        | ActionKindV1::RemoveManagedJson { rollback, .. } => rollback,
        _ => return true,
    };
    if receipt_committed {
        return !removed_app_receipt_committed(manifest, action);
    }
    !(matching_previous_app_receipt(manifest, action)
        || removed_app_receipt_committed(manifest, action))
        || manifest.find_by_dest(rollback).is_some()
}

pub(super) fn conflicting_app_receipt(
    manifest: &AppManifest,
    action: &crate::action::DeclarativeActionV1,
) -> bool {
    if let ActionKindV1::RelocateManagedFile {
        previous_destination,
        previous_backup,
        previous_rollback,
        desired_destination,
        ..
    } = &action.kind
    {
        if !matching_app_receipt(manifest, action)
            && !matching_previous_app_receipt(manifest, action)
        {
            return true;
        }
        let source = action_source_identity(action);
        return manifest.entries.iter().any(|entry| {
            entry.source != source
                && std::iter::once(entry.destination.as_path())
                    .chain(entry.backup.iter().map(PathBuf::as_path))
                    .any(|claimed| {
                        claimed == previous_destination
                            || claimed == desired_destination
                            || claimed == previous_rollback
                            || previous_backup
                                .as_ref()
                                .is_some_and(|backup| claimed == backup.path)
                    })
        });
    }
    if let ActionKindV1::RelocateManagedJson {
        previous_destination,
        previous_rollback,
        desired_destination,
        ..
    } = &action.kind
    {
        if !matching_app_receipt(manifest, action)
            && !matching_previous_app_receipt(manifest, action)
        {
            return true;
        }
        let source = action_source_identity(action);
        return manifest.entries.iter().any(|entry| {
            entry.source != source
                && std::iter::once(entry.destination.as_path())
                    .chain(entry.backup.iter().map(PathBuf::as_path))
                    .any(|claimed| {
                        claimed == previous_destination
                            || claimed == desired_destination
                            || claimed == previous_rollback
                    })
        });
    }
    if matching_app_receipt(manifest, action) {
        return false;
    }
    if matches!(
        action.kind,
        ActionKindV1::UpdateManagedFile { .. } | ActionKindV1::MergeManagedJson { .. }
    ) {
        if !matching_previous_app_receipt(manifest, action) {
            if !matches!(
                action.kind,
                ActionKindV1::MergeManagedJson {
                    previous_receipt_hash: None,
                    ..
                }
            ) {
                return true;
            }
            let source = action_source_identity(action);
            let destination = match &action.kind {
                ActionKindV1::MergeManagedJson { destination, .. } => destination,
                _ => unreachable!(),
            };
            if manifest.find_by_source(&source).is_some()
                || manifest.find_by_dest(destination).is_some()
            {
                return true;
            }
        }
        let rollback = match &action.kind {
            ActionKindV1::UpdateManagedFile { rollback, .. }
            | ActionKindV1::MergeManagedJson { rollback, .. } => rollback,
            _ => unreachable!(),
        };
        return manifest.find_by_dest(rollback).is_some();
    }
    let source = action_source_identity(action);
    if manifest.find_by_source(&source).is_some() {
        return true;
    }
    match &action.kind {
        ActionKindV1::CreateManagedFile { destination, .. } => {
            manifest.find_by_dest(destination).is_some()
        }
        ActionKindV1::CreateManagedFileWithBackup {
            destination,
            backup,
            ..
        } => {
            manifest.find_by_dest(destination).is_some() || manifest.find_by_dest(backup).is_some()
        }
        ActionKindV1::UpdateManagedFile { .. }
        | ActionKindV1::RelocateManagedFile { .. }
        | ActionKindV1::RelocateManagedJson { .. } => false,
        ActionKindV1::MergeManagedJson {
            destination,
            rollback,
            ..
        } => {
            manifest.find_by_dest(destination).is_some()
                || manifest.find_by_dest(rollback).is_some()
        }
        ActionKindV1::RemoveManagedFile { .. }
        | ActionKindV1::RemoveManagedFileWithBackup { .. }
        | ActionKindV1::ForceRemoveManagedFile { .. }
        | ActionKindV1::RemoveManagedJson { .. } => true,
        ActionKindV1::CreateShellLauncher { .. }
        | ActionKindV1::UpdateShellLauncher { .. }
        | ActionKindV1::RemoveShellLauncher { .. }
        | ActionKindV1::RemoveLegacyShellLauncher { .. }
        | ActionKindV1::ReplaceShellSnapshot { .. }
        | ActionKindV1::ReplaceShellCache { .. }
        | ActionKindV1::RemoveShellCache { .. }
        | ActionKindV1::RemoveShellSnapshot { .. }
        | ActionKindV1::ReconcileShellProfile { .. }
        | ActionKindV1::ReconcileSysSplitDns { .. }
        | ActionKindV1::ReconcileSysProfileBlocks { .. }
        | ActionKindV1::ReplaceShellRenderedFile { .. }
        | ActionKindV1::RemoveShellRenderedFile { .. }
        | ActionKindV1::OpaqueExecution { .. } => false,
    }
}

pub(super) fn is_app_removal_action(kind: &ActionKindV1) -> bool {
    matches!(
        kind,
        ActionKindV1::RemoveManagedFile { .. }
            | ActionKindV1::RemoveManagedFileWithBackup { .. }
            | ActionKindV1::ForceRemoveManagedFile { .. }
            | ActionKindV1::RemoveManagedJson { .. }
    )
}

pub(super) fn app_removal_plan_authorizes(
    plan: &PlanV1,
    action: &crate::action::DeclarativeActionV1,
    forced: bool,
) -> bool {
    plan.steps.iter().any(|step| {
        step.target == action.target
            && step.resource.as_deref() == Some(action.resource.as_str())
            && step.action == PlanActionV1::Remove
            && match plan.operation {
                PlanOperationV1::Uninstall => {
                    !forced || step.kind == Some(PlanStepKindV1::AppForcedRemoval)
                }
                PlanOperationV1::Upgrade => {
                    !forced && step.kind == Some(PlanStepKindV1::AppStalePrune)
                }
                _ => false,
            }
    })
}

pub(super) fn action_source_identity(action: &crate::action::DeclarativeActionV1) -> String {
    format!(
        "{}/{}",
        action.target.trim_end_matches('/'),
        action.resource.trim_start_matches('/')
    )
}
