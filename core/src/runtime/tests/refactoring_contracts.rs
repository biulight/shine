//! Cross-module contracts shared by the six runtime refactorings.
use crate::frontend::FrontendService;
use crate::install::{AppEntry, AppInstallStrategy, AppManifest, hash_content};
use crate::lifecycle::{LifecycleOperation, LifecycleStatus};
use crate::plan::PlanApprovalV1;
use crate::runtime::*;
use std::{future::Future, pin::Pin};

struct Interaction;
impl RuntimeInteraction for Interaction {
    fn confirm(&mut self, _: &'static str, default: bool) -> anyhow::Result<bool> {
        Ok(default)
    }
    fn authorize_admin<'a>(
        &'a mut self,
        _: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        Box::pin(async { panic!("these user-file scenarios must not request privilege") })
    }
    fn select_many(
        &mut self,
        _: &'static str,
        _: &[String],
        defaults: &[String],
    ) -> anyhow::Result<Vec<String>> {
        Ok(defaults.to_vec())
    }
}

fn context() -> RuntimeContext {
    let root = std::env::temp_dir().join("shine-cross-refactor-memory");
    RuntimeContext::isolated(
        root.join("home"),
        root.join("state"),
        root.join("presets"),
        root.join("bin"),
        RuntimePlatform::current(),
    )
}

#[tokio::test]
async fn app_static_actions_and_failed_generated_assessments_keep_independent_receipts() {
    for operation in [LifecycleOperation::Install, LifecycleOperation::Upgrade] {
        let mut context = context();
        context.env.insert("ENABLE".into(), "1".into());
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("app/demo/shine.toml", br#"metadata_schema_version = 2
dest = '~/.config/demo'
[permissions]
schema_version = 1
environment = [{ name = 'ENABLE', sensitivity = 'plain' }]
[[files]]
source = 'a-static'
[[files]]
source = 'b-generated'
generator = { script = 'gen.ts', runtime = 'bun', env = [{ source = 'ENABLE', sensitivity = 'plain' }], when_env = 'ENABLE', auto = true }
"#.to_vec())
            .file("app/demo/a-static", b"next static".to_vec())
            .file("app/demo/b-generated", b"fallback".to_vec())
            .file("app/demo/gen.ts", b"throw new Error('failed');".to_vec()).build();
        let host = InMemoryHost::new();
        let destination = context.home_dir.join(".config/demo");
        host.put_file_with_mode(
            destination.join("a-static"),
            b"old static".to_vec(),
            0o100640,
        );
        host.put_file(
            destination.join("b-generated"),
            b"last good generated".to_vec(),
        );
        let entries = [
            ("a-static", b"old static".as_slice()),
            ("b-generated", b"last good generated"),
        ]
        .into_iter()
        .map(|(resource, bytes)| AppEntry {
            source: format!("app/demo/{resource}"),
            destination: destination.join(resource),
            backup: None,
            content_hash: hash_content(bytes),
            install_strategy: AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        })
        .collect();
        AppManifest {
            entries,
            ..Default::default()
        }
        .save(&host, &context.shine_dir)
        .await
        .unwrap();
        let runtime = CoreRuntime::new(host, context, snapshot);
        let before = runtime.host().operations().len();
        let frontend = FrontendService::new(runtime);
        let inspection = frontend.inspect_apps(Vec::new()).await.unwrap().files;
        let runtime = frontend.into_runtime();
        assert_eq!(inspection[0].status, InspectionFileStatus::UpdateAvail);
        assert_eq!(
            inspection[1].status,
            InspectionFileStatus::GeneratorNotEvaluated
        );
        assert!(
            runtime.host().operations()[before..]
                .iter()
                .all(|op| matches!(op, HostOperation::Read(_)))
        );
        let request = AppPlanRequest {
            operation,
            target: Some("demo".into()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: Default::default(),
        };
        let plan = runtime.plan_apps(request.clone()).await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        runtime.host().queue_process_output(Ok(ProcessOutput {
            exit_code: Some(1),
            stderr: b"must-not-leak".to_vec(),
            ..Default::default()
        }));
        let since = runtime.host().operations().len();
        let files = if operation == LifecycleOperation::Install {
            runtime
                .install_apps_approved(
                    request.clone(),
                    &approval,
                    &mut NullObserver,
                    &mut Interaction,
                )
                .await
                .unwrap()
                .files
        } else {
            let report = runtime
                .upgrade_apps_approved(
                    request.clone(),
                    &approval,
                    AppApprovedUpgradeOptions::default(),
                    &mut NullObserver,
                    &mut Interaction,
                )
                .await
                .unwrap();
            assert_eq!(report.failed, 1);
            report.files
        };
        assert_eq!(files[0].action, AppFileAction::Installed);
        assert_eq!(files[1].action, AppFileAction::GeneratorPreserved);
        assert_eq!(
            files[1].status,
            if operation == LifecycleOperation::Install {
                LifecycleStatus::Preserved
            } else {
                LifecycleStatus::Failed
            }
        );
        assert!(
            !files[1]
                .generator_error
                .as_deref()
                .unwrap()
                .contains("must-not-leak")
        );
        assert_eq!(
            runtime.host().operations()[since..]
                .iter()
                .filter(|op| matches!(op, HostOperation::Run { .. }))
                .count(),
            1
        );
        assert_eq!(
            runtime
                .host()
                .read(&destination.join("a-static"))
                .await
                .unwrap(),
            b"next static"
        );
        assert_eq!(
            runtime
                .host()
                .read(&destination.join("b-generated"))
                .await
                .unwrap(),
            b"last good generated"
        );
        let receipt = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap();
        assert_eq!(
            receipt
                .find_by_source("app/demo/a-static")
                .unwrap()
                .content_hash,
            hash_content(b"next static")
        );
        assert_eq!(
            receipt
                .find_by_source("app/demo/b-generated")
                .unwrap()
                .content_hash,
            hash_content(b"last good generated")
        );
        assert!(
            runtime
                .host()
                .metadata(&runtime.context().shine_dir.join(APP_OPERATION_JOURNAL_FILE))
                .await
                .is_err()
        );
        assert!(
            runtime
                .host()
                .metadata(&destination.join("a-static.shine.rollback"))
                .await
                .is_err()
        );
        let frontend = FrontendService::new(runtime);
        let after = frontend.inspect_apps(Vec::new()).await.unwrap().files;
        let runtime = frontend.into_runtime();
        assert_eq!(after[0].status, InspectionFileStatus::UpToDate);
        assert_eq!(after[1].status, InspectionFileStatus::GeneratorNotEvaluated);
        // Partial generator failure must not permit replay of the earlier static update approval.
        let since = runtime.host().operations().len();
        let replay = if operation == LifecycleOperation::Install {
            runtime
                .install_apps_approved(request, &approval, &mut NullObserver, &mut Interaction)
                .await
                .map(|_| ())
        } else {
            runtime
                .upgrade_apps_approved(
                    request,
                    &approval,
                    AppApprovedUpgradeOptions::default(),
                    &mut NullObserver,
                    &mut Interaction,
                )
                .await
                .map(|_| ())
        };
        assert!(replay.is_err());
        assert!(
            runtime.host().operations()[since..]
                .iter()
                .all(|op| matches!(
                    op,
                    HostOperation::Read(_) | HostOperation::AcquireOperationLock(_)
                ))
        );
    }
}

fn shell_snapshot(version: &str) -> PresetSnapshot {
    PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file("shell/demo/shine.toml", b"[[files]]\nsource = 'run.ts'\ntarget = 'run'\nruntime = 'bun'\ntransforms = ['template']\n[files.permissions]\nschema_version = 1\nenvironment = [{ name = 'VALUE', sensitivity = 'plain' }]\n".to_vec())
        .file("shell/demo/run.ts", format!("console.log('{version} @@VALUE@@');\n").into_bytes()).build()
}

#[tokio::test]
async fn shell_inspection_stays_read_only_across_interruption_recovery_and_fresh_retry() {
    let mut context = context();
    context
        .env
        .insert("VALUE".into(), "captured payload".into());
    let runtime = CoreRuntime::new(InMemoryHost::new(), context, shell_snapshot("old"));
    let install = ShellPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo/run".into()),
        force: false,
        purge: false,
        input_versions: Default::default(),
    };
    let approval =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    runtime
        .install_shells_approved(install.clone(), &approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.inspect_shells().await.unwrap()[0].status,
        InspectionFileStatus::UpToDate
    );
    let next = CoreRuntime::new(
        runtime.host().clone(),
        runtime.context().clone(),
        shell_snapshot("next"),
    );
    assert_eq!(
        next.inspect_shells().await.unwrap()[0].status,
        InspectionFileStatus::UpdateAvail
    );
    let upgrade = ShellPlanRequest {
        operation: LifecycleOperation::Upgrade,
        ..install
    };
    let plan = next.plan_shells(upgrade.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let journal = next.context().shine_dir.join(SHELL_OPERATION_JOURNAL_FILE);
    next.host().fail_write_after(&journal, 1);
    assert!(
        next.upgrade_shells_approved(upgrade.clone(), &approval)
            .await
            .is_err()
    );
    let journal_bytes = next.host().read(&journal).await.unwrap();
    assert!(!String::from_utf8_lossy(&journal_bytes).contains("captured payload"));
    let since = next.host().operations().len();
    assert_eq!(
        next.inspect_shells().await.unwrap()[0].status,
        InspectionFileStatus::UpdateAvail
    );
    assert!(
        next.host().operations()[since..]
            .iter()
            .all(|op| matches!(op, HostOperation::Read(_)))
    );
    assert_eq!(next.host().read(&journal).await.unwrap(), journal_bytes);
    let recovery = next.plan_shell_operation_recovery().await.unwrap();
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    next.recover_shell_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert!(next.host().metadata(&journal).await.is_err());
    assert_eq!(
        runtime.inspect_shells().await.unwrap()[0].status,
        InspectionFileStatus::UpToDate
    );
    assert_eq!(
        next.inspect_shells().await.unwrap()[0].status,
        InspectionFileStatus::UpdateAvail
    );
    let retry = next.plan_shells(upgrade.clone()).await.unwrap();
    let retry_approval = PlanApprovalV1::for_reviewed_plan(&retry).unwrap();
    next.upgrade_shells_approved(upgrade, &retry_approval)
        .await
        .unwrap();
    let row = next.inspect_shells().await.unwrap().remove(0);
    assert_eq!(row.status, InspectionFileStatus::UpToDate);
    assert!(row.changes.is_empty());
    assert_eq!(row.desired_content, row.current_content);
    assert_eq!(
        row.current_content.unwrap(),
        b"console.log('next captured payload');\n"
    );
    assert!(next.host().metadata(&journal).await.is_err());
}

#[tokio::test]
async fn shared_shell_rendering_keeps_cached_bytes_receipt_consumers_and_recovery_order() {
    use crate::action::{ActionIrV1, ActionKindV1};
    #[derive(serde::Deserialize)]
    struct Journal {
        action_ir: ActionIrV1,
    }
    let mut context = context();
    context.env.insert("VALUE".into(), "captured".into());
    let mut metadata = String::new();
    for command in ["first", "second"] {
        metadata.push_str(&format!("[[files]]\nsource = 'shared.ts'\ntarget = '{command}'\nruntime = 'bun'\ntransforms = ['template']\n[files.permissions]\nschema_version = 1\nenvironment = [{{ name = 'VALUE', sensitivity = 'plain' }}]\n"));
    }
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file("shell/demo/shine.toml", metadata.into_bytes())
        .file(
            "shell/demo/shared.ts",
            b"console.log('embedded @@VALUE@@');\n".to_vec(),
        )
        .build();
    let runtime = CoreRuntime::new(InMemoryHost::new(), context, snapshot);
    let cached = runtime.context().presets_dir.join("shell/demo/shared.ts");
    let cached_bytes = b"console.log('cached @@VALUE@@');\n";
    runtime
        .host()
        .put_file_with_mode(&cached, cached_bytes.to_vec(), 0o744);
    let rendered = runtime.shell_rendered_path("demo", std::path::Path::new("shared.ts"));
    let receipt = runtime.context().shine_dir.join("shell-manifest.toml");
    runtime.host().fail_write_after(&receipt, 0);
    let request = ShellPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".into()),
        force: false,
        purge: false,
        input_versions: Default::default(),
    };
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    assert!(
        runtime
            .install_shells_approved(request.clone(), &approval)
            .await
            .is_err()
    );
    let journal_path = runtime
        .context()
        .shine_dir
        .join(SHELL_OPERATION_JOURNAL_FILE);
    let bytes = runtime.host().read(&journal_path).await.unwrap();
    let journal: Journal = toml::from_slice(&bytes).unwrap();
    let mut cache_at = None;
    let mut rendered_at = None;
    for (index, action) in journal.action_ir.actions.iter().enumerate() {
        match &action.kind {
            ActionKindV1::ReplaceShellCache { receipts, .. } => {
                cache_at = Some(index);
                assert_eq!(
                    receipts
                        .iter()
                        .map(|r| r.target.as_str())
                        .collect::<Vec<_>>(),
                    ["shell/demo/first", "shell/demo/second"]
                );
                assert!(receipts.iter().all(|r| r.previous.is_none()
                    && r.desired.content_hash == hash_content(cached_bytes)));
            }
            ActionKindV1::ReplaceShellRenderedFile {
                receipts, desired, ..
            } => {
                rendered_at = Some(index);
                assert_eq!(
                    receipts
                        .iter()
                        .map(|r| r.target.as_str())
                        .collect::<Vec<_>>(),
                    ["shell/demo/first", "shell/demo/second"]
                );
                assert_eq!(
                    desired.content_hash,
                    hash_content(b"console.log('cached captured');\n")
                );
                assert_eq!(desired.unix_mode, Some(0o744));
            }
            _ => {}
        }
    }
    assert!(cache_at.unwrap() < rendered_at.unwrap());
    assert!(!String::from_utf8_lossy(&bytes).contains("cached captured"));
    assert_eq!(runtime.host().read(&cached).await.unwrap(), cached_bytes);
    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    runtime
        .recover_shell_operation_approved(&PlanApprovalV1::for_reviewed_plan(&recovery).unwrap())
        .await
        .unwrap();
    assert!(runtime.host().metadata(&rendered).await.is_err());
    assert_eq!(runtime.host().read(&cached).await.unwrap(), cached_bytes);
    let approval =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_shells(request.clone()).await.unwrap())
            .unwrap();
    runtime
        .install_shells_approved(request, &approval)
        .await
        .unwrap();
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(manifest.entries.len(), 2);
    assert!(
        manifest
            .entries
            .iter()
            .all(|entry| entry.content_hash == hash_content(cached_bytes))
    );
    assert_eq!(
        runtime.host().read(&rendered).await.unwrap(),
        b"console.log('cached captured');\n"
    );
    assert!(runtime.host().metadata(&journal_path).await.is_err());
}
