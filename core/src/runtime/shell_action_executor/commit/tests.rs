use super::*;
use crate::runtime::{
    HostOperation, InMemoryHost, PresetSnapshot, PresetSourceKind, RuntimeContext, RuntimePlatform,
};

struct Fixture {
    runtime: CoreRuntime<InMemoryHost>,
    execution: ShellOperationExecutionV1,
    journal_path: PathBuf,
    cache: PathBuf,
    rendered: PathBuf,
    launcher: PathBuf,
}

async fn fixture(update: bool) -> Fixture {
    let home = std::env::temp_dir().join("shine-shell-commit-dispatch");
    let shine = home.join("state");
    let context = RuntimeContext::isolated(
        home,
        shine.clone(),
        shine.join("presets"),
        shine.join("bin"),
        RuntimePlatform::Linux,
    );
    let runtime = CoreRuntime::new(
        InMemoryHost::new(),
        context,
        PresetSnapshot::builder(PresetSourceKind::Embedded).build(),
    );
    let cache = shine.join("presets/shell/demo/run.sh");
    let rendered = shine.join("rendered/shell/demo/run.sh");
    let launcher = shine.join("bin/run");
    let identity = |bytes: &[u8]| ShellFileIdentityV1 {
        content_hash: hash_content(bytes),
        unix_mode: Some(0o644),
    };
    for path in [&cache, &rendered] {
        runtime.host().put_file(path, b"desired".to_vec());
        runtime
            .host()
            .put_file(managed_file_rollback_path(path), b"previous".to_vec());
    }
    runtime
        .host()
        .put_file(&launcher, b"desired-launcher".to_vec());
    let entry = ShellManifestEntry {
        launcher_format: None,
        launcher_config_dir: None,
        category: "demo".into(),
        command: "run".into(),
        mode: super::super::super::ExternalShellMode::Snapshot,
        source_path: cache.clone(),
        rendered_path: rendered.clone(),
        runtime: "native".into(),
        bun_dependencies: None,
        dependency_hash: None,
        transforms: vec!["template".into()],
        env: Vec::new(),
        needs_source: false,
        content_hash: hash_content(b"desired"),
    };
    let receipt = receipt_contract(&entry);
    let receipts = || {
        vec![ShellReceiptTransitionV1 {
            target: "shell/demo/run".into(),
            previous: None,
            desired: Box::new(receipt.clone()),
        }]
    };
    let resource = |bytes: &[u8]| ShellLauncherResourceV1::File {
        destination: launcher.clone(),
        desired_hash: hash_content(bytes),
        unix_mode: Some(0o644),
    };
    let launcher_action = if update {
        let rollback = managed_file_rollback_path(&launcher);
        runtime
            .host()
            .put_file(&rollback, b"previous-launcher".to_vec());
        let mut previous = receipt.clone();
        previous.content_hash = hash_content(b"previous");
        DeclarativeActionV1::update_shell_launcher(
            "launcher",
            "shell/demo/run",
            "launcher",
            previous,
            receipt.clone(),
            vec![ShellLauncherUpdateResourceV1 {
                previous: resource(b"previous-launcher"),
                desired: resource(b"desired-launcher"),
                rollback,
            }],
        )
    } else {
        DeclarativeActionV1::create_shell_launcher(
            "launcher",
            "shell/demo/run",
            "launcher",
            receipt.clone(),
            vec![resource(b"desired-launcher")],
        )
    };
    let actions = vec![
        DeclarativeActionV1::replace_shell_cache(
            "cache",
            "shell/demo",
            "preset-cache",
            ShellCacheReplacementSpecV1 {
                files: vec![ShellCacheFileReplacementV1 {
                    destination: cache.clone(),
                    rollback: managed_file_rollback_path(&cache),
                    previous: Some(identity(b"previous")),
                    desired: identity(b"desired"),
                }],
                receipts: receipts(),
            },
        ),
        DeclarativeActionV1::replace_shell_rendered_file(
            "rendered",
            "shell/demo/run",
            "rendered-output",
            ShellRenderedFileReplacementSpecV1 {
                destination: rendered.clone(),
                previous: Some(identity(b"previous")),
                desired: identity(b"desired"),
                receipts: receipts(),
            },
        ),
        launcher_action,
    ];
    let action_ir = ActionIrV1::new("shell-commit-dispatch", actions);
    action_ir.validate().unwrap();
    let permissions =
        action_ir.permission_requirements(|path| review_path(runtime.context(), path));
    assert!(permissions.uncomputable_codes.is_empty());
    let plan = PlanV1::new(
        PlanOperationV1::Install,
        PlanInputsV1 {
            preset: SnapshotDigestV1::builder("fixture-preset").finish(),
            state: SnapshotDigestV1::builder("fixture-state").finish(),
        },
        Vec::new(),
        permissions.required.clone(),
        &permissions.required,
        Vec::<String>::new(),
    );
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let mut journal = ShellOperationJournalV1::new(action_ir, approval);
    journal.applied = journal
        .action_ir
        .actions
        .iter()
        .map(|action| action.action_id.clone())
        .collect();
    save_shell_operation_journal(runtime.host(), &shine, &journal)
        .await
        .unwrap();
    let manifest = ShellManifest {
        entries: vec![entry],
        ..Default::default()
    };
    runtime.host().put_file(
        shine.join("shell-manifest.toml"),
        toml::to_string_pretty(&manifest).unwrap().into_bytes(),
    );
    let execution = ShellOperationExecutionV1 {
        operation_id: journal.action_ir.operation_id,
        _operation_guard: runtime.host().acquire_privileged_operation().await.unwrap(),
    };
    Fixture {
        runtime,
        execution,
        journal_path: shine.join(SHELL_OPERATION_JOURNAL_FILE),
        cache,
        rendered,
        launcher,
    }
}

#[tokio::test]
async fn mixed_shell_commit_keeps_markers_and_journal_until_all_cleanup_succeeds() {
    for update in [false, true] {
        let f = fixture(update).await;
        f.runtime
            .mark_shell_launcher_receipt_committed(&f.execution)
            .await
            .unwrap();
        let (journal, _) =
            load_shell_operation_journal(f.runtime.host(), &f.runtime.context().shine_dir)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(journal.receipt_committed, ["cache", "rendered"]);
        // Creation and update commit from matching receipts without a positive marker.
        let mut invalid_marker = journal.clone();
        invalid_marker.receipt_committed.push("launcher".into());
        assert!(invalid_marker.validate().is_err());

        let cache_rollback = managed_file_rollback_path(&f.cache);
        let rendered_rollback = managed_file_rollback_path(&f.rendered);
        f.runtime.host().fail_remove_after(&rendered_rollback, 0);
        assert!(
            f.runtime
                .commit_shell_launcher_operation(&f.execution)
                .await
                .is_err()
        );
        assert!(f.runtime.host().metadata(&cache_rollback).await.is_err());
        assert_eq!(
            f.runtime.host().read(&rendered_rollback).await.unwrap(),
            b"previous"
        );
        assert!(f.runtime.host().read(&f.journal_path).await.is_ok());
        for path in [&f.cache, &f.rendered] {
            assert_eq!(f.runtime.host().read(path).await.unwrap(), b"desired");
        }
        assert_eq!(
            f.runtime.host().read(&f.launcher).await.unwrap(),
            b"desired-launcher"
        );

        // The retry tolerates already-cleaned cache rollback and completes the rest.
        f.runtime
            .commit_shell_launcher_operation(&f.execution)
            .await
            .unwrap();
        assert!(f.runtime.host().metadata(&rendered_rollback).await.is_err());
        assert!(f.runtime.host().metadata(&f.journal_path).await.is_err());
        if update {
            assert!(
                f.runtime
                    .host()
                    .metadata(&managed_file_rollback_path(&f.launcher))
                    .await
                    .is_err()
            );
        }
        assert!(
            matches!(f.runtime.host().operations().last(), Some(HostOperation::Remove(path)) if path == &f.journal_path)
        );
    }
}

#[tokio::test]
async fn shell_commit_checks_every_resource_before_any_rollback_cleanup() {
    let f = fixture(false).await;
    f.runtime
        .mark_shell_launcher_receipt_committed(&f.execution)
        .await
        .unwrap();
    let journal = f.runtime.host().read(&f.journal_path).await.unwrap();
    f.runtime
        .host()
        .put_file(&f.rendered, b"user-modified".to_vec());
    let since = f.runtime.host().operations().len();
    assert!(
        f.runtime
            .mark_shell_launcher_receipt_committed(&f.execution)
            .await
            .is_err()
    );
    assert!(
        f.runtime
            .commit_shell_launcher_operation(&f.execution)
            .await
            .is_err()
    );
    assert!(
        f.runtime.host().operations()[since..]
            .iter()
            .all(|op| matches!(op, HostOperation::Read(_)))
    );
    assert_eq!(
        f.runtime.host().read(&f.journal_path).await.unwrap(),
        journal
    );
    assert_eq!(
        f.runtime.host().read(&f.rendered).await.unwrap(),
        b"user-modified"
    );
    for path in [&f.cache, &f.rendered] {
        assert_eq!(
            f.runtime
                .host()
                .read(&managed_file_rollback_path(path))
                .await
                .unwrap(),
            b"previous"
        );
    }
}

#[tokio::test]
async fn shell_commit_entry_points_reject_foreign_actions_without_mutation() {
    use crate::action::{ActionProvenanceV1, RollbackSupportV1, SysSplitDnsStateV1};
    for family in ["app", "sys", "opaque"] {
        let f = fixture(false).await;
        let foreign = match family {
            "app" => DeclarativeActionV1::create_managed_file(
                "foreign",
                "app/demo",
                "file",
                f.runtime.context().home_dir.join("foreign"),
                1,
                false,
            ),
            "sys" => DeclarativeActionV1::reconcile_sys_split_dns(
                "foreign",
                "sys/demo",
                "dns",
                None,
                Some(SysSplitDnsStateV1 {
                    os_id: "test".into(),
                    item_id: "demo".into(),
                    domain: "example.test".into(),
                    servers: vec!["127.0.0.1".into()],
                    resource: f.runtime.context().home_dir.join("dns"),
                    content_hash: 1,
                }),
            ),
            _ => DeclarativeActionV1 {
                action_id: "foreign".into(),
                target: "app/demo".into(),
                resource: "hook".into(),
                kind: ActionKindV1::OpaqueExecution {
                    capability: "hook".into(),
                    provenance: ActionProvenanceV1::External,
                    requires_administrator: false,
                },
                rollback: RollbackSupportV1::Unsupported {
                    reason_code: "opaque_execution".into(),
                },
            },
        };
        let (mut journal, _) =
            load_shell_operation_journal(f.runtime.host(), &f.runtime.context().shine_dir)
                .await
                .unwrap()
                .unwrap();
        journal.applied.push(foreign.action_id.clone());
        journal.action_ir.actions.push(foreign);
        journal.action_ir.validate().unwrap();
        // Model an on-disk journal that passes Action IR validation but crosses domains.
        let bytes = toml::to_string_pretty(&journal).unwrap().into_bytes();
        f.runtime.host().put_file(&f.journal_path, bytes.clone());
        let since = f.runtime.host().operations().len();
        let marker = f
            .runtime
            .mark_shell_launcher_receipt_committed(&f.execution)
            .await
            .unwrap_err();
        let commit = f
            .runtime
            .commit_shell_launcher_operation(&f.execution)
            .await
            .unwrap_err();
        let review = f.runtime.plan_shell_operation_recovery().await.unwrap_err();
        assert_eq!(
            marker.to_string(),
            "Shell operation journal contains an unsupported action"
        );
        assert_eq!(commit.to_string(), marker.to_string());
        assert_eq!(review.to_string(), marker.to_string());
        assert!(
            f.runtime.host().operations()[since..]
                .iter()
                .all(|op| matches!(op, HostOperation::Read(_)))
        );
        assert_eq!(f.runtime.host().read(&f.journal_path).await.unwrap(), bytes);
        assert_eq!(
            f.runtime
                .host()
                .read(&managed_file_rollback_path(&f.cache))
                .await
                .unwrap(),
            b"previous"
        );
    }
}
