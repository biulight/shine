//! Permissions.

use super::*;

pub(super) fn add_shine_write_permission(
    context: &crate::runtime::RuntimeContext,
    permissions: &mut PermissionAccumulator,
    path: &Path,
    purpose: FilesystemPurposeV1,
    target: &str,
) {
    permissions.implicit_for(
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Write,
            path: review_path(context, path),
        },
        purpose,
        target,
    );
}

pub(super) fn add_app_typed_permissions(
    context: &crate::runtime::RuntimeContext,
    permissions: &mut PermissionAccumulator,
    file: &AppFile,
    destination: &Path,
    operation: LifecycleOperation,
) {
    permissions.implicit(PermissionV1::Filesystem {
        access: if operation == LifecycleOperation::Uninstall {
            FilesystemAccessV1::Remove
        } else {
            FilesystemAccessV1::Write
        },
        path: review_path(context, destination),
    });
    if file.requires_admin {
        permissions.implicit(PermissionV1::Administrator);
    }
}

pub(super) fn add_app_entry_permissions(
    context: &crate::runtime::RuntimeContext,
    permissions: &mut PermissionAccumulator,
    entry: &AppEntry,
    operation: LifecycleOperation,
) {
    permissions.implicit(PermissionV1::Filesystem {
        access: if operation == LifecycleOperation::Uninstall {
            FilesystemAccessV1::Remove
        } else {
            FilesystemAccessV1::Write
        },
        path: review_path(context, &entry.destination),
    });
    if operation == LifecycleOperation::Uninstall
        && let Some(backup) = &entry.backup
    {
        permissions.implicit(PermissionV1::Filesystem {
            access: FilesystemAccessV1::Write,
            path: review_path(context, &entry.destination),
        });
        permissions.implicit_for(
            PermissionV1::Filesystem {
                access: FilesystemAccessV1::Remove,
                path: review_path(context, backup),
            },
            FilesystemPurposeV1::Recovery,
            review_path(context, &entry.destination),
        );
    }
    if entry.requires_admin {
        permissions.implicit(PermissionV1::Administrator);
    }
}

pub(super) fn add_shell_typed_permissions(
    context: &crate::runtime::RuntimeContext,
    permissions: &mut PermissionAccumulator,
    path: &Path,
    operation: LifecycleOperation,
    purpose: FilesystemPurposeV1,
    target: &str,
) {
    permissions.implicit_for(
        PermissionV1::Filesystem {
            access: if operation == LifecycleOperation::Uninstall {
                FilesystemAccessV1::Remove
            } else {
                FilesystemAccessV1::Write
            },
            path: review_path(context, path),
        },
        purpose,
        target,
    );
}

pub(super) fn add_shell_profile_permissions(
    context: &crate::runtime::RuntimeContext,
    permissions: &mut PermissionAccumulator,
    paths: &[PathBuf],
) {
    let managed_profile =
        crate::runtime::managed_shell_profile_path(&context.shine_dir, context.shell);
    for path in paths {
        let rollback = managed_file_rollback_path(path);
        for (access, effect) in [
            (FilesystemAccessV1::Write, path),
            (FilesystemAccessV1::Remove, path),
            (FilesystemAccessV1::Write, &rollback),
            (FilesystemAccessV1::Remove, &rollback),
        ] {
            permissions.implicit_for(
                PermissionV1::Filesystem {
                    access,
                    path: review_path(context, effect),
                },
                if effect == &rollback {
                    FilesystemPurposeV1::Recovery
                } else if path == &managed_profile {
                    FilesystemPurposeV1::Installation
                } else {
                    FilesystemPurposeV1::UserTarget
                },
                if path == &managed_profile {
                    "shell/profile".to_string()
                } else {
                    review_path(context, path)
                },
            );
        }
    }
}

pub(super) fn add_shine_receipt_permission(
    context: &crate::runtime::RuntimeContext,
    permissions: &mut PermissionAccumulator,
    file: &str,
    operation: LifecycleOperation,
) {
    permissions.implicit_for(
        PermissionV1::Filesystem {
            access: if operation == LifecycleOperation::Uninstall {
                FilesystemAccessV1::Remove
            } else {
                FilesystemAccessV1::Write
            },
            path: review_path(context, &context.shine_dir.join(file)),
        },
        FilesystemPurposeV1::Maintenance,
        "installation state",
    );
}

pub(super) fn add_app_journal_permissions(
    context: &crate::runtime::RuntimeContext,
    permissions: &mut PermissionAccumulator,
) {
    let path = review_path(
        context,
        &context
            .shine_dir
            .join(crate::runtime::APP_OPERATION_JOURNAL_FILE),
    );
    permissions.implicit_for(
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Write,
            path: path.clone(),
        },
        FilesystemPurposeV1::Maintenance,
        "installation state",
    );
    permissions.implicit_for(
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Remove,
            path,
        },
        FilesystemPurposeV1::Maintenance,
        "installation state",
    );
}

pub(super) fn add_shell_journal_permissions(
    context: &crate::runtime::RuntimeContext,
    permissions: &mut PermissionAccumulator,
) {
    let path = review_path(
        context,
        &context
            .shine_dir
            .join(crate::runtime::SHELL_OPERATION_JOURNAL_FILE),
    );
    permissions.implicit_for(
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Write,
            path: path.clone(),
        },
        FilesystemPurposeV1::Maintenance,
        "installation state",
    );
    permissions.implicit_for(
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Remove,
            path,
        },
        FilesystemPurposeV1::Maintenance,
        "installation state",
    );
}

pub(super) fn add_app_backup_creation_permissions(
    context: &crate::runtime::RuntimeContext,
    permissions: &mut PermissionAccumulator,
    destination: &Path,
    backup: &Path,
) {
    permissions.implicit(PermissionV1::Filesystem {
        access: FilesystemAccessV1::Remove,
        path: review_path(context, destination),
    });
    permissions.implicit_for(
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Write,
            path: review_path(context, backup),
        },
        FilesystemPurposeV1::Recovery,
        review_path(context, destination),
    );
}

pub(super) fn add_app_update_permissions(
    context: &crate::runtime::RuntimeContext,
    permissions: &mut PermissionAccumulator,
    destination: &Path,
    rollback: &Path,
) {
    for (access, path) in [
        (FilesystemAccessV1::Remove, destination),
        (FilesystemAccessV1::Write, rollback),
        (FilesystemAccessV1::Remove, rollback),
    ] {
        permissions.implicit_for(
            PermissionV1::Filesystem {
                access,
                path: review_path(context, path),
            },
            if path == rollback {
                FilesystemPurposeV1::Recovery
            } else {
                FilesystemPurposeV1::UserTarget
            },
            review_path(context, destination),
        );
    }
}

pub(super) fn add_app_relocation_permissions(
    context: &crate::runtime::RuntimeContext,
    permissions: &mut PermissionAccumulator,
    previous: &AppEntry,
    previous_present: bool,
    desired_destination: &Path,
    rollback: &Path,
    desired_requires_admin: bool,
) {
    permissions.implicit(PermissionV1::Filesystem {
        access: FilesystemAccessV1::Write,
        path: review_path(context, desired_destination),
    });
    if previous_present {
        for (access, path) in [
            (FilesystemAccessV1::Remove, previous.destination.as_path()),
            (FilesystemAccessV1::Write, rollback),
            (FilesystemAccessV1::Remove, rollback),
        ] {
            permissions.implicit(PermissionV1::Filesystem {
                access,
                path: review_path(context, path),
            });
        }
        if matches!(
            previous.install_strategy,
            crate::install::AppInstallStrategy::JsonMerge { .. }
        ) {
            permissions.implicit(PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: review_path(context, &previous.destination),
            });
        }
        if let Some(backup) = &previous.backup {
            permissions.implicit(PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: review_path(context, &previous.destination),
            });
            permissions.implicit_for(
                PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Remove,
                    path: review_path(context, backup),
                },
                FilesystemPurposeV1::Recovery,
                review_path(context, &previous.destination),
            );
        }
    }
    if (previous_present && previous.requires_admin) || desired_requires_admin {
        permissions.implicit(PermissionV1::Administrator);
    }
}

pub(super) fn add_sys_receipt_permissions(
    context: &crate::runtime::RuntimeContext,
    permissions: &mut PermissionAccumulator,
    receipt: &SystemReceipt,
    operation: LifecycleOperation,
) {
    match receipt {
        SystemReceipt::ManagedFile(receipt) => {
            permissions.implicit(PermissionV1::Filesystem {
                access: if operation == LifecycleOperation::Uninstall {
                    FilesystemAccessV1::Remove
                } else {
                    FilesystemAccessV1::Write
                },
                path: review_path(context, &receipt.destination),
            });
            if receipt.privileged {
                permissions.implicit(PermissionV1::Administrator);
            }
        }
        SystemReceipt::SplitDns(_) => {
            permissions.implicit(PermissionV1::Administrator);
            permissions.implicit(PermissionV1::System {
                capability: "split-dns".to_string(),
                resource: Some("private-domain".to_string()),
            });
        }
        SystemReceipt::Script { .. } => {
            permissions
                .uncomputable
                .insert("sys_managed_driver_uncomputable".to_string());
        }
    }
}
