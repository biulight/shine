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
