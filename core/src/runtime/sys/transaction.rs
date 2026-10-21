//! Managed Sys transaction preparation from fresh resource observations.
use super::*;
impl<H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost> CoreRuntime<H> {
    pub(super) async fn prepare_sys_apply_transaction(
        &self,
        os_id: &str,
        item: &SysItem,
        previous_entry: Option<&SysRunEntry>,
    ) -> Result<Option<PreparedSysTransaction>> {
        let previous_receipt = previous_entry.and_then(|entry| entry.receipt.as_ref());
        let target = format!("sys/{}", item.id);
        match item.driver {
            SysDriverKind::ManagedFile => {
                let desired = self.managed_file_desired(os_id, item)?;
                let desired_hash = hash_content(&desired.content);
                let previous = match previous_receipt {
                    Some(SystemReceipt::ManagedFile(receipt)) => Some(receipt),
                    Some(other) => bail!("managed-file received {:?} receipt", other.driver()),
                    None => None,
                };
                let desired_receipt = ManagedFileReceipt {
                    version: RECEIPT_VERSION,
                    destination: desired.destination.clone(),
                    backup: previous
                        .filter(|receipt| receipt.destination == desired.destination)
                        .and_then(|receipt| receipt.backup.clone()),
                    content_hash: desired_hash,
                    privileged: item.requires_admin,
                    restart_hint: desired.restart_hint.clone(),
                };
                let action_identity = format!("{os_id}:{}:{desired_hash}", item.id);
                let action = if let Some(previous) = previous {
                    let current = read_regular_sys_file(
                        &self.host,
                        &previous.destination,
                        "reading managed Sys source",
                    )
                    .await?;
                    let Some((current, mode)) = current else {
                        bail!(
                            "managed Sys file {} is missing; uninstall it before reapplying",
                            previous.destination.display()
                        );
                    };
                    if hash_content(&current) != previous.content_hash {
                        return Err(ResourceConflict::user_modified(format!(
                            "managed file {} was modified; keeping user content",
                            previous.destination.display()
                        ))
                        .into());
                    }
                    if previous.destination == desired.destination {
                        if current == desired.content
                            && previous.content_hash == desired_hash
                            && previous.privileged == item.requires_admin
                        {
                            return Ok(None);
                        }
                        DeclarativeActionV1::update_managed_file(
                            format!("update:{action_identity}"),
                            &target,
                            desired.destination.display().to_string(),
                            ManagedFileUpdateSpecV1 {
                                destination: desired.destination.clone(),
                                previous_backup: previous.backup.clone(),
                                original_mode: mode,
                                original_hash: previous.content_hash,
                                desired_hash,
                                requires_admin: item.requires_admin,
                            },
                        )
                    } else {
                        if self.host.metadata(&desired.destination).await.is_ok() {
                            bail!(
                                "managed Sys relocation destination {} must be absent",
                                desired.destination.display()
                            );
                        }
                        let previous_backup = match &previous.backup {
                            Some(path) => {
                                let (bytes, backup_mode) = read_regular_sys_file(
                                    &self.host,
                                    path,
                                    "reading managed Sys persistent backup",
                                )
                                .await?
                                .with_context(|| {
                                    format!("managed Sys backup {} is missing", path.display())
                                })?;
                                Some(ManagedFileRelocationBackupV1 {
                                    path: path.clone(),
                                    mode: backup_mode,
                                    hash: hash_content(&bytes),
                                })
                            }
                            None => None,
                        };
                        DeclarativeActionV1::relocate_managed_file(
                            format!("relocate:{action_identity}"),
                            &target,
                            desired.destination.display().to_string(),
                            ManagedFileRelocationSpecV1 {
                                previous_destination: previous.destination.clone(),
                                previous_backup,
                                desired_destination: desired.destination.clone(),
                                previous_present: true,
                                previous_mode: mode,
                                previous_hash: previous.content_hash,
                                desired_hash,
                                previous_uses_env: false,
                                desired_uses_env: false,
                                previous_requires_admin: previous.privileged,
                                desired_requires_admin: item.requires_admin,
                            },
                        )
                    }
                } else {
                    match read_regular_sys_file(
                        &self.host,
                        &desired.destination,
                        "reading managed Sys destination",
                    )
                    .await?
                    {
                        None => DeclarativeActionV1::create_managed_file(
                            format!("create:{action_identity}"),
                            &target,
                            desired.destination.display().to_string(),
                            desired.destination.clone(),
                            desired_hash,
                            item.requires_admin,
                        ),
                        Some((original, _)) => {
                            let backup = crate::install::backup_path(&desired.destination);
                            if self.host.metadata(&backup).await.is_ok() {
                                bail!(
                                    "managed Sys backup path {} is already occupied",
                                    backup.display()
                                );
                            }
                            DeclarativeActionV1::create_managed_file_with_backup(
                                format!("create-with-backup:{action_identity}"),
                                &target,
                                desired.destination.display().to_string(),
                                ManagedFileCreationWithBackupSpecV1 {
                                    destination: desired.destination.clone(),
                                    backup: backup.clone(),
                                    original_hash: hash_content(&original),
                                    desired_hash,
                                    requires_admin: item.requires_admin,
                                },
                            )
                        }
                    }
                };
                let creates_backup = matches!(
                    &action.kind,
                    crate::action::ActionKindV1::CreateManagedFileWithBackup { .. }
                );
                let backup = match &action.kind {
                    crate::action::ActionKindV1::CreateManagedFileWithBackup { backup, .. } => {
                        Some(backup.clone())
                    }
                    _ => desired_receipt.backup.clone(),
                };
                let desired_entry = SysRunEntry {
                    os_id: os_id.to_string(),
                    item_id: item.id.clone(),
                    label: item.label.clone(),
                    status: SysItemStatus::Updated,
                    detail: desired.destination.display().to_string(),
                    updated_at: self.context.captured_unix_time.to_string(),
                    managed: true,
                    profile_enabled: false,
                    receipt: Some(SystemReceipt::ManagedFile(ManagedFileReceipt {
                        backup,
                        ..desired_receipt
                    })),
                };
                let outcome_receipt = desired_entry.receipt.clone();
                Ok(Some(PreparedSysTransaction {
                    action_ir: ActionIrV1::new(
                        format!("sys:{}:{action_identity}", self.context.captured_unix_time),
                        vec![action],
                    ),
                    receipt: SysReceiptTransitionV1 {
                        previous: previous_entry.cloned().map(Box::new),
                        desired: Some(Box::new(desired_entry)),
                    },
                    desired_content: Some(desired.content),
                    outcome: ResourceOutcome {
                        changed: true,
                        effects: if creates_backup {
                            vec![
                                LifecycleEffect::BackupCreated,
                                LifecycleEffect::ResourceWritten,
                            ]
                        } else {
                            vec![LifecycleEffect::ResourceWritten]
                        },
                        detail: desired.destination.display().to_string(),
                        receipt: outcome_receipt,
                        restart_hint: desired.restart_hint,
                    },
                }))
            }
            SysDriverKind::SplitDns => {
                let mut desired =
                    split_dns_receipt(&self.split_dns_item_request(os_id, item, false)?)?;
                let desired_request = split_dns_host_request(&desired);
                let state = self.host.inspect_split_dns(&desired_request).await?;
                if state.exists && !split_dns_owned(&state, &desired) {
                    bail!(
                        "split DNS destination {} exists but is not owned by shine",
                        desired.resource
                    );
                }
                desired.content_hash = Some(hash_content(&desired_request.content));
                if previous_receipt.is_some_and(|receipt| {
                    matches!(receipt, SystemReceipt::SplitDns(previous) if previous == &desired)
                }) && split_dns_state_matches(&state, &desired_request)
                {
                    return Ok(None);
                }
                let previous = match previous_receipt {
                    Some(SystemReceipt::SplitDns(receipt)) => {
                        Some(split_dns_action_state(receipt)?)
                    }
                    Some(other) => bail!("split-dns received {:?} receipt", other.driver()),
                    None => None,
                };
                let desired_state = split_dns_action_state(&desired)?;
                let desired_entry = SysRunEntry {
                    os_id: os_id.to_string(),
                    item_id: item.id.clone(),
                    label: item.label.clone(),
                    status: SysItemStatus::Updated,
                    detail: format!("{} -> {}", desired.domain, desired.servers.join(", ")),
                    updated_at: self.context.captured_unix_time.to_string(),
                    managed: true,
                    profile_enabled: false,
                    receipt: Some(SystemReceipt::SplitDns(desired.clone())),
                };
                let outcome_receipt = desired_entry.receipt.clone();
                let action = DeclarativeActionV1::reconcile_sys_split_dns(
                    format!("split-dns:{os_id}:{}", item.id),
                    &target,
                    desired.resource.clone(),
                    previous,
                    Some(desired_state),
                );
                Ok(Some(PreparedSysTransaction {
                    action_ir: ActionIrV1::new(
                        format!(
                            "sys:{}:{os_id}:{}",
                            self.context.captured_unix_time, item.id
                        ),
                        vec![action],
                    ),
                    receipt: SysReceiptTransitionV1 {
                        previous: previous_entry.cloned().map(Box::new),
                        desired: Some(Box::new(desired_entry)),
                    },
                    desired_content: None,
                    outcome: ResourceOutcome {
                        changed: true,
                        effects: vec![LifecycleEffect::ResourceWritten],
                        detail: format!("{} -> {}", desired.domain, desired.servers.join(", ")),
                        receipt: outcome_receipt,
                        restart_hint: None,
                    },
                }))
            }
            SysDriverKind::Script => bail!("script is not a built-in system resource driver"),
        }
    }

    pub(super) async fn prepare_sys_removal_transaction(
        &self,
        previous_entry: &SysRunEntry,
    ) -> Result<Option<PreparedSysTransaction>> {
        let receipt = previous_entry
            .receipt
            .as_ref()
            .context("managed Sys removal requires a receipt")?;
        let target = format!("sys/{}", previous_entry.item_id);
        let (action, effects, detail) = match receipt {
            SystemReceipt::ManagedFile(receipt) => {
                let Some((current, mode)) = read_regular_sys_file(
                    &self.host,
                    &receipt.destination,
                    "reading managed Sys removal destination",
                )
                .await?
                else {
                    return Ok(None);
                };
                if hash_content(&current) != receipt.content_hash {
                    return Err(ResourceConflict::user_modified(format!(
                        "managed file {} was modified; keeping user content",
                        receipt.destination.display()
                    ))
                    .into());
                }
                let action = if let Some(backup) = &receipt.backup {
                    let (backup_bytes, backup_mode) = read_regular_sys_file(
                        &self.host,
                        backup,
                        "reading managed Sys persistent backup",
                    )
                    .await?
                    .with_context(|| {
                        format!("managed Sys backup {} is missing", backup.display())
                    })?;
                    DeclarativeActionV1::remove_managed_file_with_backup(
                        format!("remove-with-backup:{}", previous_entry.item_id),
                        &target,
                        receipt.destination.display().to_string(),
                        ManagedFileRemoveWithBackupSpecV1 {
                            destination: receipt.destination.clone(),
                            backup: backup.clone(),
                            managed_mode: mode,
                            managed_hash: receipt.content_hash,
                            backup_mode,
                            backup_hash: hash_content(&backup_bytes),
                            uses_env: false,
                            requires_admin: receipt.privileged,
                        },
                    )
                } else {
                    DeclarativeActionV1::remove_managed_file(
                        format!("remove:{}", previous_entry.item_id),
                        &target,
                        receipt.destination.display().to_string(),
                        ManagedFileRemoveSpecV1 {
                            destination: receipt.destination.clone(),
                            original_mode: mode,
                            original_hash: receipt.content_hash,
                            uses_env: false,
                            requires_admin: receipt.privileged,
                        },
                    )
                };
                let effects = if receipt.backup.is_some() {
                    vec![LifecycleEffect::BackupRestored]
                } else {
                    vec![LifecycleEffect::ResourceRemoved]
                };
                (action, effects, receipt.destination.display().to_string())
            }
            SystemReceipt::SplitDns(receipt) => {
                let state = split_dns_action_state(receipt)?;
                let request = split_dns_host_request(receipt);
                let current = self.host.inspect_split_dns(&request).await?;
                if !current.exists {
                    return Ok(None);
                }
                if !split_dns_owned(&current, receipt) {
                    return Err(ResourceConflict::user_modified(format!(
                        "split DNS resource {} was modified; keeping user content",
                        receipt.resource
                    ))
                    .into());
                }
                (
                    DeclarativeActionV1::reconcile_sys_split_dns(
                        format!("remove-split-dns:{}", previous_entry.item_id),
                        &target,
                        receipt.resource.clone(),
                        Some(state),
                        None,
                    ),
                    vec![LifecycleEffect::ResourceRemoved],
                    format!("remove split DNS for {}", receipt.domain),
                )
            }
            SystemReceipt::Script { .. } => {
                bail!("script receipt is not a managed system resource")
            }
        };
        Ok(Some(PreparedSysTransaction {
            action_ir: ActionIrV1::new(
                format!(
                    "sys-remove:{}:{}:{}",
                    self.context.captured_unix_time, previous_entry.os_id, previous_entry.item_id
                ),
                vec![action],
            ),
            receipt: SysReceiptTransitionV1 {
                previous: Some(Box::new(previous_entry.clone())),
                desired: None,
            },
            desired_content: None,
            outcome: ResourceOutcome {
                changed: true,
                effects,
                detail,
                receipt: None,
                restart_hint: match receipt {
                    SystemReceipt::ManagedFile(receipt) => receipt.restart_hint.clone(),
                    _ => None,
                },
            },
        }))
    }
}
