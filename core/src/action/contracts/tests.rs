use super::*;

#[test]
fn resource_contracts_preserve_exact_permissions_and_reject_other_rollback_kinds() {
    let destination = std::env::temp_dir().join("shine-action-contract");
    let rollback = managed_file_rollback_path(&destination);
    let filesystem = |access, path: &Path| PermissionV1::Filesystem {
        access,
        path: format!("resource:{}", path.display()),
    };
    let cases = [
        (
            ActionKindV1::CreateManagedFile {
                destination: destination.clone(),
                desired_hash: 1,
                requires_admin: true,
            },
            RollbackSupportV1::RemoveCreatedIfUnchanged,
            vec![
                filesystem(FilesystemAccessV1::Write, &destination),
                PermissionV1::Administrator,
            ],
            false,
        ),
        (
            ActionKindV1::MergeManagedJson {
                destination: destination.clone(),
                rollback: rollback.clone(),
                original_mode: None,
                original_hash: None,
                previous_receipt_hash: None,
                desired_managed_hash: 1,
                managed_keys: vec!["owned".into()],
            },
            RollbackSupportV1::RestoreJsonKeysIfUnchanged,
            vec![
                filesystem(FilesystemAccessV1::Write, &destination),
                filesystem(FilesystemAccessV1::Remove, &destination),
            ],
            false,
        ),
        (
            ActionKindV1::RemoveLegacyShellLauncher {
                resources: vec![ShellLauncherRemovalResourceV1 {
                    previous: ShellLauncherResourceV1::Symlink {
                        destination: destination.clone(),
                        target: destination.with_extension("sh"),
                    },
                    rollback: rollback.clone(),
                }],
            },
            RollbackSupportV1::RestoreRemovedLauncherIfUnchanged,
            vec![
                filesystem(FilesystemAccessV1::Remove, &destination),
                filesystem(FilesystemAccessV1::Write, &rollback),
                filesystem(FilesystemAccessV1::Remove, &rollback),
            ],
            false,
        ),
        (
            ActionKindV1::ReconcileSysSplitDns {
                previous: None,
                desired: Some(SysSplitDnsStateV1 {
                    os_id: "test".into(),
                    item_id: "dns".into(),
                    domain: "example.test".into(),
                    servers: vec!["1.1.1.1".into()],
                    resource: destination.clone(),
                    content_hash: 1,
                }),
            },
            RollbackSupportV1::RestorePreviousSysSplitDnsIfUnchanged,
            vec![PermissionV1::Administrator],
            false,
        ),
        (
            ActionKindV1::OpaqueExecution {
                capability: "app-hook".into(),
                provenance: ActionProvenanceV1::External,
                requires_administrator: false,
            },
            RollbackSupportV1::Unsupported {
                reason_code: "opaque_action_not_reversible".into(),
            },
            vec![],
            true,
        ),
    ];
    let rollbacks = cases
        .iter()
        .map(|(_, rollback, _, _)| rollback.clone())
        .collect::<Vec<_>>();
    for (kind, rollback, expected_permissions, opaque) in cases {
        let action = DeclarativeActionV1 {
            action_id: "action".into(),
            target: "demo".into(),
            resource: "resource".into(),
            kind,
            rollback: rollback.clone(),
        };
        let ir = ActionIrV1::new("operation", vec![action.clone()]);
        ir.validate().unwrap();
        let requirements =
            ir.permission_requirements(|path| format!("resource:{}", path.display()));
        let mut expected = PermissionSetV1::default();
        for permission in expected_permissions {
            expected.insert(permission);
        }
        assert_eq!(requirements.required, expected);
        assert_eq!(
            requirements.uncomputable_codes,
            if opaque {
                BTreeSet::from(["opaque_action_permissions_uncomputable".to_string()])
            } else {
                BTreeSet::new()
            }
        );
        for candidate in &rollbacks {
            let mut candidate_action = action.clone();
            candidate_action.rollback = candidate.clone();
            assert_eq!(
                ActionIrV1::new("operation", vec![candidate_action])
                    .validate()
                    .is_ok(),
                *candidate == rollback
            );
        }
    }
}
