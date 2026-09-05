use super::tests::{app_plan_request, assert_observation_only, runtime};
use super::*;
use crate::lifecycle::LifecycleOperation;
use crate::runtime::*;
use std::future::Future;
use std::pin::Pin;

struct Human;

fn snapshot() -> PresetSnapshot {
    PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file("app/available/shine.toml", b"dest = '~/.config/available'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec())
        .file("app/available/config.toml", b"value = true\n".to_vec())
        .file("shell/tools/shine.toml", b"[[files]]\nsource = 'tool.sh'\ntarget = 'tool'\npermissions = { schema_version = 1 }\n".to_vec())
        .file("shell/tools/tool.sh", b"echo tool\n".to_vec())
        .file("sys/ubuntu/shine.toml", b"version = 2\n[[items]]\nid = 'managed'\nlabel = 'Managed'\nmode = 'managed'\ndriver = 'managed-file'\npermissions = { schema_version = 1 }\n[items.config]\nsource = 'managed.txt'\ntarget = '~/.config/managed'\n".to_vec())
        .file("sys/ubuntu/managed.txt", b"managed\n".to_vec()).build()
}
impl RuntimeInteraction for Human {
    fn confirm(&mut self, _: &'static str, default: bool) -> anyhow::Result<bool> {
        Ok(default)
    }
    fn authorize_admin<'a>(
        &'a mut self,
        _: usize,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send + 'a>> {
        Box::pin(async { Ok(true) })
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

fn service(host: InMemoryHost) -> FrontendService<InMemoryHost> {
    FrontendService::new(runtime(host, snapshot()))
        .with_configuration_revision(Some("config:one".into()))
}

fn same_json(left: &impl serde::Serialize, right: &impl serde::Serialize) {
    assert_eq!(
        serde_json::to_value(left).unwrap(),
        serde_json::to_value(right).unwrap()
    );
}

#[tokio::test]
async fn specialized_artifact_execution_retains_secret_versions_and_operation_identity() {
    let presets = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "app/demo/shine.toml",
            br#"dest = '~/.config/demo'
[artifact]
script = 'build.ts'
teardown = 'unbuild.ts'
runtime = 'bun'
env = ['TOKEN']
[permissions]
schema_version = 1
filesystem = [
  { access = ['execute'], base = 'preset', path = 'build.ts' },
  { access = ['execute'], base = 'preset', path = 'unbuild.ts' },
]
commands = ['bun']
environment = [{ name = 'TOKEN', sensitivity = 'secret' }]
[[files]]
source = 'config.toml'
"#
            .to_vec(),
        )
        .file("app/demo/config.toml", b"config".to_vec())
        .file("app/demo/build.ts", b"process.exit(0)".to_vec())
        .file("app/demo/unbuild.ts", b"process.exit(0)".to_vec())
        .build();
    for action in [AppArtifactAction::Apply, AppArtifactAction::Remove] {
        let mut outputs = Vec::new();
        for _ in 0..2 {
            let mut runtime = runtime(InMemoryHost::new(), presets.clone());
            runtime
                .context_mut_for_cli()
                .env
                .insert("TOKEN".into(), "private-plaintext".into());
            let mut input_versions = PlanningInputVersions::default();
            input_versions
                .insert_secret_version("TOKEN", OpaqueSecretVersion::new("vault-revision-9"));
            let request = ReviewRequest::AppArtifact(AppArtifactPlanRequest {
                category: "demo".into(),
                action,
                input_versions,
            });
            runtime.host().queue_process_output(Ok(ProcessOutput {
                exit_code: Some(0),
                stdout: Vec::new(),
                stderr: Vec::new(),
            }));
            let service = FrontendService::new(runtime);
            let readonly = service.read_only().request_review(&request).await.unwrap();
            let trusted = service.into_trusted();
            let review = trusted.review(request).await.unwrap();
            same_json(review.report(), &readonly);
            let approved = review.approve_after_human_confirmation().unwrap();
            let mut events = Vec::new();
            let execution = trusted
                .apply(
                    approved,
                    ExecutionOptions::default(),
                    &mut NullObserver,
                    &mut Human,
                    &mut events,
                )
                .await
                .unwrap();
            assert!(matches!(
                execution.report.result,
                ExecutionResultV1::AppSpecialized { .. }
            ));
            assert_eq!(execution.report.operation, readonly.plan.operation);
            let encoded = serde_json::to_string(&(&execution.report, &events)).unwrap();
            for private in ["private-plaintext", "vault-revision-9", "/home/test"] {
                assert!(!encoded.contains(private));
            }
            outputs.push(execution.report);
        }
        same_json(&outputs[0], &outputs[1]);
    }
}

#[tokio::test]
async fn specialized_profile_and_bootstrap_keep_distinct_results() {
    let presets = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "sys/ubuntu/shine.toml",
            br#"version = 2
[[items]]
id = 'tool'
label = 'Tool'
permissions = { schema_version = 1 }
detect = { kind = 'path', path = '$HOME/.tool-present' }
install = { kind = 'package', provider = 'homebrew', package = 'tool' }
[[items.shell]]
shells = ['bash']
phase = 'post'
path = '$HOME/.tool/bin'
"#
            .to_vec(),
        )
        .build();
    let requests = [
        ReviewRequest::SysBootstrap(SysBootstrapPlanRequest {
            os_id: "ubuntu".into(),
            item_ids: vec!["tool".into()],
            sys_shell: "bash".into(),
            force_profile: false,
            input_versions: PlanningInputVersions::default(),
        }),
        ReviewRequest::SysProfile(SysProfilePlanRequest {
            os_id: "ubuntu".into(),
            item_id: "tool".into(),
            enabled: true,
        }),
        ReviewRequest::SysProfile(SysProfilePlanRequest {
            os_id: "ubuntu".into(),
            item_id: "tool".into(),
            enabled: false,
        }),
    ];
    let mut runs = Vec::new();
    for _ in 0..2 {
        let host = InMemoryHost::new();
        host.put_file("/home/test/.tool-present", b"present".to_vec());
        let mut reports = Vec::new();
        for request in &requests {
            let mut runtime = runtime(host.clone(), presets.clone());
            runtime.context_mut_for_cli().shell = ShellType::Bash;
            let trusted = FrontendService::new(runtime).into_trusted();
            let review = trusted.review(request.clone()).await.unwrap();
            let operation = review.report().plan.operation;
            let approved = review.approve_after_human_confirmation().unwrap();
            let mut events = Vec::new();
            let execution = trusted
                .apply(
                    approved,
                    ExecutionOptions::default(),
                    &mut NullObserver,
                    &mut Human,
                    &mut events,
                )
                .await
                .unwrap();
            assert!(matches!(
                execution.report.result,
                ExecutionResultV1::SysSpecialized { .. }
            ));
            assert_eq!(execution.report.operation, operation);
            assert!(
                !serde_json::to_string(&(&execution.report, &events))
                    .unwrap()
                    .contains("/home/test")
            );
            reports.push(execution.report);
        }
        runs.push(reports);
    }
    same_json(&runs[0], &runs[1]);
}

#[tokio::test]
async fn refresh_consumes_the_reviewed_secret_input_versions() {
    let presets = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file("app/demo/shine.toml", br#"dest = '~/.config/demo'
[permissions]
schema_version = 1
filesystem = [{ access = ['execute'], base = 'preset', path = 'gen.ts' }]
commands = ['bun']
environment = [{ name = 'SOURCE', sensitivity = 'secret' }]
[[files]]
source = 'generated.txt'
generator = { script = 'gen.ts', runtime = 'bun', env = ['SOURCE'], when_env = 'SOURCE', auto = false }
"#.to_vec())
        .file("app/demo/generated.txt", b"fallback".to_vec())
        .file("app/demo/gen.ts", b"process.stdout.write('generated')".to_vec()).build();
    let mut reports = Vec::new();
    for _ in 0..2 {
        let mut runtime = runtime(InMemoryHost::new(), presets.clone());
        runtime
            .context_mut_for_cli()
            .env
            .insert("SOURCE".into(), "private-source".into());
        let destination = runtime
            .context()
            .home_dir
            .join(".config/demo/generated.txt");
        runtime.host().put_file(&destination, b"installed".to_vec());
        crate::install::AppManifest {
            entries: vec![crate::install::AppEntry {
                source: "app/demo/generated.txt".into(),
                destination,
                backup: None,
                content_hash: crate::install::hash_content(b"installed"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
            ..crate::install::AppManifest::default()
        }
        .save(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
        runtime.host().queue_process_output(Ok(ProcessOutput {
            exit_code: Some(0),
            stdout: b"generated".to_vec(),
            stderr: Vec::new(),
        }));
        let mut input_versions = PlanningInputVersions::default();
        input_versions.insert_secret_version("SOURCE", OpaqueSecretVersion::new("source-version"));
        let request = ReviewRequest::AppRefresh(AppRefreshPlanRequest {
            category: "demo".into(),
            file: None,
            force: false,
            input_versions,
        });
        let trusted = FrontendService::new(runtime).into_trusted();
        let review = trusted.review(request).await.unwrap();
        let approved = review.approve_after_human_confirmation().unwrap();
        let mut events = Vec::new();
        let execution = trusted
            .apply(
                approved,
                ExecutionOptions::default(),
                &mut NullObserver,
                &mut Human,
                &mut events,
            )
            .await
            .unwrap();
        assert_eq!(
            execution.report.operation,
            crate::plan::PlanOperationV1::AppRefresh
        );
        let OperationDetails::AppRefresh(local) = execution.details else {
            panic!("refresh details")
        };
        assert_eq!(local.lifecycle.summary().changed, 1);
        for private in ["private-source", "source-version", "/home/test"] {
            assert!(
                !serde_json::to_string(&(&execution.report, &events))
                    .unwrap()
                    .contains(private)
            );
        }
        reports.push(execution.report);
    }
    same_json(&reports[0], &reports[1]);
}

#[tokio::test]
async fn readonly_facade_returns_identical_reports_and_safe_errors_without_effects() {
    let service = service(InMemoryHost::new());
    let readonly = service.read_only();
    let since = service.runtime().host().operations().len();
    same_json(
        &readonly
            .inventory(InventoryRequest::all().with_sys_os_id("ubuntu"))
            .await
            .unwrap(),
        &service
            .inventory(InventoryRequest::all().with_sys_os_id("ubuntu"))
            .await
            .unwrap(),
    );
    same_json(
        &readonly.inspect_apps(Vec::new()).await.unwrap(),
        &service.inspect_apps(Vec::new()).await.unwrap().report,
    );
    same_json(
        &readonly.inspect_shells().await.unwrap(),
        &service.inspect_shells().await.unwrap().report,
    );
    same_json(
        &readonly.inspect_sys("ubuntu").await.unwrap(),
        &service.inspect_sys("ubuntu").await.unwrap().report,
    );
    for kind in [
        CapabilityKindV1::App,
        CapabilityKindV1::Shell,
        CapabilityKindV1::Sys,
    ] {
        same_json(
            &readonly.operation_state(kind).await.unwrap(),
            &service.operation_state(kind).await.unwrap(),
        );
    }
    same_json(
        &readonly
            .request_review(&ReviewRequest::App(app_plan_request()))
            .await
            .unwrap(),
        &service
            .review(&ReviewRequest::App(app_plan_request()))
            .await
            .unwrap(),
    );
    assert_observation_only(service.runtime().host(), since);
    service.runtime().host().put_file(
        service
            .runtime()
            .context()
            .shine_dir
            .join(APP_OPERATION_JOURNAL_FILE),
        b"SECRET=/private/path".to_vec(),
    );
    let diagnostic = readonly
        .operation_state(CapabilityKindV1::App)
        .await
        .unwrap_err();
    assert_eq!(
        serde_json::to_value(diagnostic).unwrap(),
        serde_json::json!({"code":"frontend_operation_state_failed","severity":"error"})
    );
}

#[tokio::test]
async fn shared_capture_matches_core_inventory() {
    let host = InMemoryHost::new();
    let context = runtime(host.clone(), snapshot()).context().clone();
    let request = PresetSnapshotRequest {
        source: PresetSnapshotSource::Embedded(vec![
            (
                "app/demo/shine.toml".into(),
                b"dest='~/.config/demo'\n[[files]]\nsource='config'\n".to_vec(),
            ),
            ("app/demo/config".into(), b"first".to_vec()),
        ]),
        overlay_root: None,
    };
    let expected = capture_preset_snapshot(&host, request.clone())
        .await
        .unwrap();
    let service = FrontendService::capture(host.clone(), context.clone(), request)
        .await
        .unwrap();
    let core = CoreRuntime::new(host, context, expected);
    same_json(
        &service
            .inventory(InventoryRequest::for_kind(CapabilityKindV1::App))
            .await
            .unwrap(),
        &FrontendService::new(core)
            .inventory(InventoryRequest::for_kind(CapabilityKindV1::App))
            .await
            .unwrap(),
    );
}

#[tokio::test]
async fn normal_lifecycle_cli_ui_and_core_results_agree() {
    for kind in [
        CapabilityKindV1::App,
        CapabilityKindV1::Shell,
        CapabilityKindV1::Sys,
    ] {
        let core = runtime(InMemoryHost::new(), snapshot());
        let cli_host = InMemoryHost::new();
        let ui_host = InMemoryHost::new();
        for operation in [
            LifecycleOperation::Install,
            LifecycleOperation::Upgrade,
            LifecycleOperation::Uninstall,
        ] {
            let request = match kind {
                CapabilityKindV1::App => ReviewRequest::App(AppPlanRequest {
                    operation,
                    ..app_plan_request()
                }),
                CapabilityKindV1::Shell => ReviewRequest::Shell(ShellPlanRequest {
                    operation,
                    target: Some("tools".into()),
                    force: false,
                    purge: false,
                    input_versions: PlanningInputVersions::default(),
                }),
                CapabilityKindV1::Sys => ReviewRequest::Sys(SysManagedPlanRequest {
                    operation,
                    os_id: "ubuntu".into(),
                    target: Some("managed".into()),
                    input_versions: PlanningInputVersions::default(),
                }),
            };
            let plan = FrontendService::new(CoreRuntime::new(
                core.host().clone(),
                core.context().clone(),
                core.presets().clone(),
            ))
            .review(&request)
            .await
            .unwrap()
            .plan;
            assert!(plan.is_ready(), "{kind:?} {operation:?}: {plan:?}");
            let approval = crate::plan::PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
            let expected = match request.clone() {
                ReviewRequest::App(request) => match operation {
                    LifecycleOperation::Install => {
                        core.install_apps_approved(
                            request,
                            &approval,
                            &mut NullObserver,
                            &mut Human,
                        )
                        .await
                        .unwrap()
                        .lifecycle
                    }
                    LifecycleOperation::Upgrade => {
                        core.upgrade_apps_approved(
                            request,
                            &approval,
                            AppApprovedUpgradeOptions::default(),
                            &mut NullObserver,
                            &mut Human,
                        )
                        .await
                        .unwrap()
                        .lifecycle
                    }
                    LifecycleOperation::Uninstall => {
                        core.uninstall_apps_approved(
                            request,
                            &approval,
                            &mut NullObserver,
                            &mut Human,
                        )
                        .await
                        .unwrap()
                        .lifecycle
                    }
                    _ => unreachable!(),
                },
                ReviewRequest::Shell(request) => match operation {
                    LifecycleOperation::Install => {
                        core.install_shells_approved(request, &approval)
                            .await
                            .unwrap()
                            .lifecycle
                    }
                    LifecycleOperation::Upgrade => {
                        core.upgrade_shells_approved(request, &approval)
                            .await
                            .unwrap()
                            .lifecycle
                    }
                    LifecycleOperation::Uninstall => {
                        core.uninstall_shells_approved(request, &approval)
                            .await
                            .unwrap()
                            .lifecycle
                    }
                    _ => unreachable!(),
                },
                ReviewRequest::Sys(request) => {
                    core.run_managed_sys_approved(request, &approval, &mut Human, &mut NullObserver)
                        .await
                        .unwrap()
                        .lifecycle
                }
                _ => unreachable!(),
            };
            let mut results = Vec::new();
            for host in [cli_host.clone(), ui_host.clone()] {
                let trusted = service(host.clone()).into_trusted();
                let review = trusted.review(request.clone()).await.unwrap();
                same_json(&review.report().plan, &plan);
                let approved = review.approve_after_human_confirmation().unwrap();
                let mut events = Vec::new();
                let execution = service(host)
                    .into_trusted()
                    .apply(
                        approved,
                        ExecutionOptions::default(),
                        &mut NullObserver,
                        &mut Human,
                        &mut events,
                    )
                    .await
                    .unwrap();
                let ExecutionResultV1::Lifecycle { result } = &execution.report.result else {
                    panic!("normal lifecycle result")
                };
                same_json(result, &expected);
                assert_eq!(events.first().unwrap().status, None);
                assert_eq!(
                    events.last().unwrap().status,
                    Some(FrontendEventStatusV1::Completed)
                );
                let encoded = serde_json::to_string(&execution.report).unwrap();
                assert!(!encoded.contains("/home/test"));
                let decoded: ExecutionReportV1 = serde_json::from_str(&encoded).unwrap();
                same_json(&decoded, &execution.report);
                results.push(execution.report);
            }
            same_json(&results[0], &results[1]);
        }
    }
}

#[tokio::test]
async fn configuration_and_live_state_changes_reject_before_effects() {
    for changed_config in [true, false] {
        let host = InMemoryHost::new();
        let review = service(host.clone())
            .into_trusted()
            .review(ReviewRequest::App(app_plan_request()))
            .await
            .unwrap();
        let approved = review.approve_after_human_confirmation().unwrap();
        let current = if changed_config {
            service(host.clone()).with_configuration_revision(Some("config:two".into()))
        } else {
            host.put_file(
                "/home/test/.config/available/config.toml",
                b"user change".to_vec(),
            );
            service(host.clone())
        };
        let since = host.operations().len();
        let mut events = Vec::new();
        let error = current
            .into_trusted()
            .apply(
                approved,
                ExecutionOptions::default(),
                &mut NullObserver,
                &mut Human,
                &mut events,
            )
            .await
            .err()
            .expect("stale approval rejected");
        assert_eq!(
            error.diagnostic().code,
            if changed_config {
                "frontend_configuration_changed"
            } else {
                "frontend_approval_stale"
            }
        );
        assert!(events.is_empty());
        assert_observation_only(&host, since);
    }
}

#[tokio::test]
async fn changed_source_or_permissions_in_fresh_capture_invalidates_approval() {
    for metadata_changed in [false, true] {
        let host = InMemoryHost::new();
        let approved = service(host.clone())
            .into_trusted()
            .review(ReviewRequest::App(app_plan_request()))
            .await
            .unwrap()
            .approve_after_human_confirmation()
            .unwrap();
        let metadata = if metadata_changed {
            "dest = '~/.config/available'\n[permissions]\nschema_version = 1\ncommands = ['new-command']\n[[files]]\nsource = 'config.toml'\n"
        } else {
            "dest = \"~/.config/available\"\n[[files]]\nsource = \"config.toml\"\n"
        };
        let presets = PresetSnapshot::builder(PresetSourceKind::External)
            .file("app/available/shine.toml", metadata.as_bytes().to_vec())
            .file(
                "app/available/config.toml",
                if metadata_changed {
                    b"value = true\n".to_vec()
                } else {
                    b"value = false\n".to_vec()
                },
            )
            .build();
        let current = FrontendService::new(runtime(host.clone(), presets))
            .with_configuration_revision(Some("config:one".into()))
            .into_trusted();
        let since = host.operations().len();
        let error = current
            .apply(
                approved,
                ExecutionOptions::default(),
                &mut NullObserver,
                &mut Human,
                &mut Vec::new(),
            )
            .await
            .err()
            .expect("source change invalidates review");
        assert_eq!(error.diagnostic().code, "frontend_approval_stale");
        assert_observation_only(&host, since);
    }
}
