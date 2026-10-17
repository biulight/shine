//! Refresh phase contracts through the approved production entry point.
use super::*;
use crate::runtime::{
    AppRefreshPlanRequest, HostOperation, InMemoryHost, NullObserver, PresetSnapshot,
    PresetSourceKind, ProcessOutput, RuntimeContext, RuntimePlatform,
};
use std::{future::Future, pin::Pin};

struct Interaction {
    allow: bool,
    calls: usize,
}
impl RuntimeInteraction for Interaction {
    fn confirm(&mut self, _: &'static str, default: bool) -> Result<bool> {
        Ok(default)
    }
    fn authorize_admin<'a>(
        &'a mut self,
        _: usize,
    ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>> {
        self.calls += 1;
        Box::pin(async { Ok(self.allow) })
    }
    fn select_many(
        &mut self,
        _: &'static str,
        _: &[String],
        defaults: &[String],
    ) -> Result<Vec<String>> {
        Ok(defaults.to_vec())
    }
}
async fn runtime(admin: bool) -> CoreRuntime<InMemoryHost> {
    let root = std::env::temp_dir().join("shine-refresh-phases");
    let mut context = RuntimeContext::isolated(
        root.join("home"),
        root.join("state"),
        root.join("presets"),
        root.join("bin"),
        RuntimePlatform::current(),
    );
    context.env.insert("ENABLE".into(), "1".into());
    let mut metadata = String::from(
        "metadata_schema_version = 2\ndest = '~/.config/demo'\npost_upgrade = { script = 'hook.sh' }\n[permissions]\nschema_version = 2\nopaque_code = 'unrestricted'\nenvironment = [{ name = 'ENABLE', sensitivity = 'plain' }]\n",
    );
    for source in ["first", "second"] {
        metadata.push_str(&format!("[[files]]\nsource = '{source}'\nrequires_admin = {admin}\ngenerator = {{ script = 'generate.sh', when_env = 'ENABLE', env = ['ENABLE'], auto = false }}\n"));
    }
    let runtime = CoreRuntime::new(
        InMemoryHost::new(),
        context,
        PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("app/demo/shine.toml", metadata.into_bytes())
            .file("app/demo/first", b"fallback".to_vec())
            .file("app/demo/second", b"fallback".to_vec())
            .file("app/demo/generate.sh", b"#!/bin/sh\nexit 0\n".to_vec())
            .file("app/demo/hook.sh", b"#!/bin/sh\nexit 0\n".to_vec())
            .build(),
    );
    let mut manifest = AppManifest::default();
    for source in ["first", "second"] {
        let destination = runtime.context().home_dir.join(".config/demo").join(source);
        runtime.host().put_file(&destination, b"old".to_vec());
        manifest.upsert(AppEntry {
            source: format!("app/demo/{source}"),
            destination,
            backup: None,
            content_hash: hash_content(b"old"),
            install_strategy: AppInstallStrategy::Copy,
            uses_env: true,
            requires_admin: admin,
        });
    }
    manifest
        .save(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    runtime
}
fn request() -> AppRefreshPlanRequest {
    AppRefreshPlanRequest {
        category: "demo".into(),
        file: None,
        force: false,
        input_versions: Default::default(),
    }
}
#[tokio::test]
async fn refresh_continues_after_file_failure_saves_once_and_runs_hooks_after_receipt() {
    let runtime = runtime(false).await;
    runtime.host().queue_process_output(Ok(ProcessOutput {
        exit_code: Some(1),
        stderr: b"private diagnostic".to_vec(),
        ..Default::default()
    }));
    runtime.host().queue_process_output(Ok(ProcessOutput {
        exit_code: Some(0),
        stdout: b"next".to_vec(),
        ..Default::default()
    }));
    runtime.host().queue_process_output(Ok(ProcessOutput {
        exit_code: Some(0),
        ..Default::default()
    }));
    let request = request();
    let plan = runtime.plan_app_refresh(request.clone()).await.unwrap();
    assert!(plan.is_ready(), "{plan:#?}");
    let approval = PlanApprovalV1::for_reviewed_plan(
        &runtime.plan_app_refresh(request.clone()).await.unwrap(),
    )
    .unwrap();
    let since = runtime.host().operations().len();
    let report = runtime
        .refresh_app_generators_approved(
            request,
            &approval,
            &mut NullObserver,
            &mut Interaction {
                allow: true,
                calls: 0,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        report
            .files
            .iter()
            .map(|file| file.status)
            .collect::<Vec<_>>(),
        [LifecycleStatus::Failed, LifecycleStatus::Changed]
    );
    assert!(!format!("{report:?}").contains("private diagnostic"));
    let destination = runtime.context().home_dir.join(".config/demo");
    assert_eq!(
        runtime
            .host()
            .read(&destination.join("first"))
            .await
            .unwrap(),
        b"old"
    );
    assert_eq!(
        runtime
            .host()
            .read(&destination.join("second"))
            .await
            .unwrap(),
        b"next"
    );
    let receipt = runtime.context().shine_dir.join("app-manifest.toml");
    let operations = runtime.host().operations();
    let trace = &operations[since..];
    let writes = trace
        .iter()
        .enumerate()
        .filter(|(_, op)| matches!(op, HostOperation::Write(path) if path == &receipt))
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    assert_eq!(writes.len(), 1);
    let runs = trace
        .iter()
        .enumerate()
        .filter(|(_, op)| matches!(op, HostOperation::Run { .. }))
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    assert_eq!(runs.len(), 3);
    assert!(runs[1] < writes[0] && writes[0] < runs[2]);
}
#[tokio::test]
async fn refresh_admin_denial_and_failed_final_receipt_do_not_run_hooks() {
    for deny in [true, false] {
        let runtime = runtime(deny).await;
        let receipt = runtime.context().shine_dir.join("app-manifest.toml");
        let before = runtime.host().read(&receipt).await.unwrap();
        let request = request();
        let plan = runtime.plan_app_refresh(request.clone()).await.unwrap();
        assert!(plan.is_ready(), "{plan:#?}");
        let approval = PlanApprovalV1::for_reviewed_plan(
            &runtime.plan_app_refresh(request.clone()).await.unwrap(),
        )
        .unwrap();
        if !deny {
            runtime.host().fail_write_after(&receipt, 0);
        }
        for _ in 0..2 {
            runtime.host().queue_process_output(Ok(ProcessOutput {
                exit_code: Some(0),
                stdout: b"next".to_vec(),
                ..Default::default()
            }));
        }
        let since = runtime.host().operations().len();
        let mut interaction = Interaction {
            allow: !deny,
            calls: 0,
        };
        assert!(
            runtime
                .refresh_app_generators_approved(
                    request,
                    &approval,
                    &mut NullObserver,
                    &mut interaction
                )
                .await
                .is_err()
        );
        assert_eq!(interaction.calls, usize::from(deny));
        assert_eq!(runtime.host().read(&receipt).await.unwrap(), before);
        assert!(
            runtime
                .app_operation_journal_bytes()
                .await
                .unwrap()
                .is_none()
        );
        assert!(runtime.plan_app_operation_recovery().await.is_err());
        assert_eq!(
            runtime.host().process_requests().len(),
            if deny { 0 } else { 2 }
        );
        for source in ["first", "second"] {
            assert_eq!(
                runtime
                    .host()
                    .read(&runtime.context().home_dir.join(".config/demo").join(source))
                    .await
                    .unwrap(),
                if deny {
                    b"old".as_slice()
                } else {
                    b"next".as_slice()
                }
            );
        }
        if deny {
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
}

#[tokio::test]
async fn refresh_selection_preserves_declared_order_and_requires_exact_receipt_owner() {
    for foreign in [false, true] {
        let runtime = runtime(false).await;
        let mut manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap();
        // Receipt order must not decide generator order.
        manifest.entries.reverse();
        if foreign {
            manifest
                .entries
                .iter_mut()
                .find(|entry| entry.source == "app/demo/first")
                .unwrap()
                .source = "app/other/first".into();
        }
        manifest
            .save(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap();
        let receipt = runtime.context().shine_dir.join("app-manifest.toml");
        let before = runtime.host().read(&receipt).await.unwrap();
        let since = runtime.host().operations().len();
        let plan = runtime.plan_app_refresh(request()).await.unwrap();
        let selected = plan
            .steps
            .iter()
            .filter_map(|step| step.resource.as_deref())
            .filter(|resource| resource.starts_with("generator:"))
            .collect::<Vec<_>>();
        assert_eq!(
            selected,
            if foreign {
                vec!["generator:second"]
            } else {
                vec!["generator:first", "generator:second"]
            }
        );
        if foreign {
            let mut targeted = request();
            targeted.file = Some("first".into());
            assert_eq!(
                runtime
                    .plan_app_refresh(targeted)
                    .await
                    .unwrap_err()
                    .to_string(),
                "app 'demo' generated file is not installed: first; run `shine install app/demo` to install it before refreshing"
            );
        }
        assert!(
            runtime.host().operations()[since..]
                .iter()
                .all(|operation| matches!(operation, HostOperation::Read(_)))
        );
        assert_eq!(runtime.host().read(&receipt).await.unwrap(), before);
        for _ in 0..if foreign { 1 } else { 2 } {
            runtime.host().queue_process_output(Ok(ProcessOutput {
                exit_code: Some(0),
                stdout: b"next".to_vec(),
                ..Default::default()
            }));
        }
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        let report = runtime
            .refresh_app_generators_approved(
                request(),
                &approval,
                &mut NullObserver,
                &mut Interaction {
                    allow: true,
                    calls: 0,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            report
                .files
                .iter()
                .map(|file| file.source.to_str().unwrap())
                .collect::<Vec<_>>(),
            if foreign {
                vec!["second"]
            } else {
                vec!["first", "second"]
            }
        );
        let after = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap();
        for file in &report.files {
            assert_eq!(
                after.find_by_dest(&file.destination).unwrap().content_hash,
                hash_content(b"next")
            );
        }
        if foreign {
            assert_eq!(
                after.find_by_source("app/other/first"),
                manifest.find_by_source("app/other/first")
            );
            assert_eq!(
                runtime
                    .host()
                    .read(&runtime.context().home_dir.join(".config/demo/first"))
                    .await
                    .unwrap(),
                b"old"
            );
        }
    }
}

#[tokio::test]
async fn refresh_revalidates_receipts_after_waiting_for_the_lifecycle_lock() {
    let runtime = runtime(true).await;
    let plan = runtime.plan_app_refresh(request()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let guard = runtime.acquire_app_lifecycle_operation().await.unwrap();
    let mut observer = NullObserver;
    let mut interaction = Interaction {
        allow: true,
        calls: 0,
    };
    let since = runtime.host().operations().len();
    let mut pending = Box::pin(runtime.refresh_app_generators_approved(
        request(),
        &approval,
        &mut observer,
        &mut interaction,
    ));
    std::future::poll_fn(|cx| {
        assert!(pending.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    assert_eq!(
        runtime.host().operations().len(),
        since,
        "no state capture before acquiring the lock"
    );
    let mut manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    manifest.entries[0].content_hash = hash_content(b"changed while waiting");
    manifest
        .save(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    let receipt = runtime.context().shine_dir.join("app-manifest.toml");
    let changed = runtime.host().read(&receipt).await.unwrap();
    let since = runtime.host().operations().len();
    drop(guard);
    let error = pending.await.unwrap_err();
    assert_eq!(
        error.downcast_ref::<crate::plan::PlanApprovalError>(),
        Some(&crate::plan::PlanApprovalError::PlanChanged)
    );
    assert_eq!(interaction.calls, 0);
    assert!(runtime.host().process_requests().is_empty());
    assert!(
        runtime.host().operations()[since..]
            .iter()
            .all(|operation| matches!(
                operation,
                HostOperation::Read(_) | HostOperation::AcquireOperationLock(_)
            ))
    );
    assert_eq!(runtime.host().read(&receipt).await.unwrap(), changed);
}

#[tokio::test]
async fn refresh_preserves_unrelated_receipts_added_while_waiting_for_the_lock() {
    let runtime = runtime(false).await;
    let plan = runtime.plan_app_refresh(request()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let guard = runtime.acquire_app_lifecycle_operation().await.unwrap();
    let mut observer = NullObserver;
    let mut interaction = Interaction {
        allow: true,
        calls: 0,
    };
    let mut pending = Box::pin(runtime.refresh_app_generators_approved(
        request(),
        &approval,
        &mut observer,
        &mut interaction,
    ));
    std::future::poll_fn(|cx| {
        assert!(pending.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    let mut manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    let unrelated = AppEntry {
        source: "app/other/config".into(),
        destination: runtime.context().home_dir.join(".config/other/config"),
        backup: Some(runtime.context().home_dir.join(".config/other/config.bak")),
        content_hash: hash_content(b"unrelated"),
        install_strategy: AppInstallStrategy::Copy,
        uses_env: false,
        requires_admin: false,
    };
    runtime
        .host()
        .put_file(&unrelated.destination, b"unrelated".to_vec());
    manifest.upsert(unrelated.clone());
    manifest
        .save(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(runtime.plan_app_refresh(request()).await.unwrap(), plan);
    for _ in 0..2 {
        runtime.host().queue_process_output(Ok(ProcessOutput {
            exit_code: Some(0),
            stdout: b"next".to_vec(),
            ..Default::default()
        }));
    }
    drop(guard);
    let report = pending.await.unwrap();
    assert!(
        report
            .files
            .iter()
            .all(|file| file.status == LifecycleStatus::Changed)
    );
    let after = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(after.find_by_source(&unrelated.source), Some(&unrelated));
    assert_eq!(
        runtime.host().read(&unrelated.destination).await.unwrap(),
        b"unrelated"
    );
    for source in ["first", "second"] {
        assert_eq!(
            after
                .find_by_source(&format!("app/demo/{source}"))
                .unwrap()
                .content_hash,
            hash_content(b"next")
        );
    }
    // Completion releases the guard after receipt persistence and hooks.
    let mut next = Box::pin(runtime.acquire_app_lifecycle_operation());
    std::future::poll_fn(|cx| {
        assert!(matches!(
            next.as_mut().poll(cx),
            std::task::Poll::Ready(Ok(_))
        ));
        std::task::Poll::Ready(())
    })
    .await;
}

#[tokio::test]
async fn refresh_waiting_on_lock_never_recovers_an_interrupted_operation_implicitly() {
    // Obtain a real journal from an approved static install whose receipt save fails.
    let original = runtime(false).await;
    let interrupted = CoreRuntime::new(InMemoryHost::new(), original.context().clone(),
        PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("app/other/shine.toml", b"dest = '~/.config/other'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config'\n".to_vec())
            .file("app/other/config", b"interrupted".to_vec()).build());
    let install = crate::runtime::AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("other".into()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: Default::default(),
    };
    let approval =
        PlanApprovalV1::for_reviewed_plan(&interrupted.plan_apps(install.clone()).await.unwrap())
            .unwrap();
    interrupted
        .host()
        .fail_write_after(interrupted.context().shine_dir.join("app-manifest.toml"), 0);
    assert!(
        interrupted
            .install_apps_approved(
                install,
                &approval,
                &mut NullObserver,
                &mut Interaction {
                    allow: true,
                    calls: 0
                }
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("failed to write app manifest")
    );
    let journal_path = original
        .context()
        .shine_dir
        .join(crate::runtime::APP_OPERATION_JOURNAL_FILE);
    let journal = interrupted.host().read(&journal_path).await.unwrap();
    let resource = original.context().home_dir.join(".config/other/config");
    let content = interrupted.host().read(&resource).await.unwrap();
    let mut future: toml::Value = toml::from_slice(&journal).unwrap();
    future["schema_version"] = toml::Value::Integer(999);

    for (journal, valid) in [
        (journal, true),
        (b"corrupt journal".to_vec(), false),
        (toml::to_string(&future).unwrap().into_bytes(), false),
    ] {
        let runtime = runtime(true).await;
        let plan = runtime.plan_app_refresh(request()).await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        let receipt = runtime.context().shine_dir.join("app-manifest.toml");
        let before = runtime.host().read(&receipt).await.unwrap();
        let guard = runtime.acquire_app_lifecycle_operation().await.unwrap();
        let mut observer = NullObserver;
        let mut interaction = Interaction {
            allow: true,
            calls: 0,
        };
        let mut pending = Box::pin(runtime.refresh_app_generators_approved(
            request(),
            &approval,
            &mut observer,
            &mut interaction,
        ));
        std::future::poll_fn(|cx| {
            assert!(pending.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        runtime.host().put_file(&journal_path, journal.clone());
        runtime.host().put_file(&resource, content.clone());
        let since = runtime.host().operations().len();
        drop(guard);
        let error = pending.await.unwrap_err();
        if valid {
            assert_eq!(
                error.downcast_ref::<crate::plan::PlanApprovalError>(),
                Some(&crate::plan::PlanApprovalError::PlanNotReady)
            );
            let blocked = runtime.plan_app_refresh(request()).await.unwrap();
            assert!(!blocked.is_ready());
            assert!(blocked.steps.iter().any(|step| {
                step.diagnostic_codes
                    .iter()
                    .any(|code| code == "app_recovery_required")
            }));
        } else {
            assert!(runtime.plan_app_refresh(request()).await.is_err());
        }
        assert_eq!(interaction.calls, 0);
        assert!(runtime.host().process_requests().is_empty());
        assert!(
            runtime.host().operations()[since..]
                .iter()
                .all(|op| matches!(
                    op,
                    HostOperation::Read(_) | HostOperation::AcquireOperationLock(_)
                ))
        );
        assert_eq!(runtime.host().read(&journal_path).await.unwrap(), journal);
        assert_eq!(runtime.host().read(&resource).await.unwrap(), content);
        assert_eq!(runtime.host().read(&receipt).await.unwrap(), before);
        if valid {
            // Only separately approved recovery may remove the interrupted resource/journal.
            let recovery = runtime.plan_app_operation_recovery().await.unwrap();
            let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
            let report = runtime
                .recover_app_operation_approved(&approval)
                .await
                .unwrap();
            assert_eq!(report.rolled_back_actions.len(), 1);
            assert!(runtime.host().read(&resource).await.is_err());
            assert!(runtime.host().read(&journal_path).await.is_err());
            assert_eq!(runtime.host().read(&receipt).await.unwrap(), before);
            assert!(
                runtime
                    .plan_app_refresh(request())
                    .await
                    .unwrap()
                    .is_ready()
            );
        }
    }
}
