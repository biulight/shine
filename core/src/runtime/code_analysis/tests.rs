use super::*;
use crate::lifecycle::LifecycleOperation;
use crate::plan::PlanApprovalV1;
use crate::runtime::*;

fn runtime(code: &str, permissions: &str, enabled: bool) -> CoreRuntime<InMemoryHost> {
    let root = std::env::temp_dir().join("shine-code-analysis-plan");
    let mut context = RuntimeContext::isolated(
        root.join("home"),
        root.join("state"),
        root.join("presets"),
        root.join("bin"),
        RuntimePlatform::Linux,
    );
    if enabled {
        context.env.insert("ENABLE".into(), "yes".into());
    }
    CoreRuntime::new(InMemoryHost::new(),context,PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file("app/demo/shine.toml",format!("metadata_schema_version=2\ndescription='demo'\ndest='~/.demo'\n[permissions]\nschema_version=1\nenvironment=[{{name='ENABLE',sensitivity='plain'}}]\n{permissions}\n[[files]]\nsource='config'\ngenerator={{script='gen.ts',runtime='bun',env=[{{ source='ENABLE', sensitivity='plain' }}],when_env='ENABLE',auto=false}}\n").into_bytes())
        .file("app/demo/config",b"fallback".to_vec()).file("app/demo/gen.ts",code.as_bytes().to_vec()).build())
}
fn request(operation: LifecycleOperation) -> AppPlanRequest {
    AppPlanRequest {
        operation,
        target: Some("demo".into()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    }
}

#[tokio::test]
async fn plan_prepares_once_and_locked_planning_only_consumes() {
    let runtime = runtime(
        "fetch('https://example.org');",
        "network=[{scope='host',host='example.org'}]",
        true,
    );
    let missing = runtime
        .plan_apps_prepared(request(LifecycleOperation::Install))
        .await
        .unwrap();
    assert!(!missing.is_ready());
    assert_eq!(runtime.analysis.0.lock().unwrap().sources.parsed_files(), 0);
    let prepared = runtime
        .plan_apps(request(LifecycleOperation::Install))
        .await
        .unwrap();
    assert!(prepared.is_ready(), "{prepared:?}");
    assert_eq!(runtime.analysis.0.lock().unwrap().sources.parsed_files(), 1);
    let checked = runtime
        .plan_apps_prepared(request(LifecycleOperation::Install))
        .await
        .unwrap();
    assert_eq!(prepared, checked);
    runtime
        .plan_apps(request(LifecycleOperation::Install))
        .await
        .unwrap();
    assert_eq!(runtime.analysis.0.lock().unwrap().sources.parsed_files(), 1);
    PlanApprovalV1::for_reviewed_plan(&prepared).unwrap();
}

#[tokio::test]
async fn shadowed_and_replaced_builtins_do_not_block_lifecycle_approval() {
    for code in [
        "export {}; function* fetch(url) { yield url; } fetch('https://fake.invalid');",
        "import fs from 'node:fs'; const alias = fs; alias.readFileSync = () => 'ok'; fs.readFileSync('config');",
    ] {
        let runtime = runtime(code, "", true);
        let plan = runtime
            .plan_apps(request(LifecycleOperation::Install))
            .await
            .unwrap();
        assert!(plan.is_ready(), "{plan:?}");
        assert!(PlanApprovalV1::for_reviewed_plan(&plan).is_ok());
        let diagnostics = &plan.code_analysis[0].scope.report.diagnostics;
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code == "code_analysis_call_unknown")
        );
        assert!(!diagnostics.iter().any(Diagnostic::blocking));
    }
}

#[tokio::test]
async fn preserved_builtin_calls_still_block_lifecycle_approval() {
    for code in [
        "import fs from 'node:fs'; const read = fs.readFileSync; const alias = fs; alias.readFileSync = () => 'ok'; read('private');",
        "import fs from 'node:fs'; const alias = fs; alias.custom = 1; fs.readFileSync('private');",
        "import fs from 'node:fs'; function run() { const read = original; read('private'); } const original = fs.readFileSync; run();",
    ] {
        let runtime = runtime(code, "", true);
        let plan = runtime
            .plan_apps(request(LifecycleOperation::Install))
            .await
            .unwrap();
        assert!(!plan.is_ready(), "{plan:?}");
        assert!(PlanApprovalV1::for_reviewed_plan(&plan).is_err());
        assert!(
            plan.code_analysis[0]
                .scope
                .report
                .diagnostics
                .iter()
                .any(|d| d.kind == DiagnosticKind::Difference)
        );
    }
}

#[tokio::test]
async fn definite_difference_blocks_approval_with_gaps_retained() {
    let runtime = runtime(
        "fetch('https://example.org'); Bun.spawn(['curl']);",
        "",
        true,
    );
    let plan = runtime
        .plan_apps(request(LifecycleOperation::Install))
        .await
        .unwrap();
    assert!(!plan.is_ready());
    assert!(PlanApprovalV1::for_reviewed_plan(&plan).is_err());
    let diagnostics = &plan.code_analysis[0].scope.report.diagnostics;
    assert!(
        diagnostics
            .iter()
            .any(|d| d.kind == DiagnosticKind::Difference)
    );
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == "code_analysis_subprocess_effects")
    );
}

#[tokio::test]
async fn untriggered_generators_do_not_parse_invalid_source() {
    let runtime = runtime("const broken = ;", "", false);
    let plan = runtime
        .plan_apps(request(LifecycleOperation::Install))
        .await
        .unwrap();
    assert!(plan.code_analysis.is_empty());
    assert_eq!(runtime.analysis.0.lock().unwrap().sources.parsed_files(), 0);
    let runtime = super::tests::runtime("const broken = ;", "", true);
    let plan = runtime
        .plan_apps(request(LifecycleOperation::Upgrade))
        .await
        .unwrap();
    assert!(plan.code_analysis.is_empty());
    assert_eq!(runtime.analysis.0.lock().unwrap().sources.parsed_files(), 0);
}

#[tokio::test]
async fn declarations_and_new_source_invalidate_approved_evidence() {
    let first = runtime(
        "fetch('https://example.org');",
        "network=[{scope='host',host='example.org'}]",
        true,
    );
    let plan = first
        .plan_apps(request(LifecycleOperation::Install))
        .await
        .unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let mut changed = runtime(
        "fetch('https://changed.example');",
        "network=[{scope='host',host='example.org'}]",
        true,
    );
    changed.analysis = first.analysis.clone();
    let unchecked = changed
        .plan_apps_prepared(request(LifecycleOperation::Install))
        .await
        .unwrap();
    assert!(!unchecked.is_ready());
    assert!(approval.validate(&unchecked).is_err());
    let checked = changed
        .plan_apps(request(LifecycleOperation::Install))
        .await
        .unwrap();
    assert!(!checked.is_ready());
    assert_eq!(changed.analysis.0.lock().unwrap().sources.parsed_files(), 2);
    let mut declaration = runtime("fetch('https://example.org');", "", true);
    declaration.analysis = first.analysis.clone();
    assert!(
        !declaration
            .plan_apps(request(LifecycleOperation::Install))
            .await
            .unwrap()
            .is_ready()
    );
    assert_eq!(first.analysis.0.lock().unwrap().sources.parsed_files(), 2);
}

#[tokio::test]
async fn lint_checks_entries_skipped_by_runtime_trigger_conditions() {
    let runtime = runtime("const broken = ;", "", false);
    let category = runtime.app_categories(Some("demo")).unwrap().remove(0);
    let scopes = runtime.lint_app_code(&category).await.unwrap();
    assert_eq!(scopes.len(), 1);
    assert!(!scopes[0].report.allowed);
    assert!(
        scopes[0]
            .report
            .diagnostics
            .iter()
            .any(|d| d.code == "code_analysis_parse_failure")
    );
}

#[tokio::test]
async fn author_tools_share_diagnostics_and_negative_fixtures_pass_without_execution() {
    let live = runtime("fetch('https://example.org');", "", true);
    let source = InMemoryHost::new();
    for (path, bytes) in live.presets().files() {
        source.put_file(format!("/repo/{path}"), bytes.clone());
    }
    source.put_file("/repo/app/demo/shine.test.toml",br#"
schema_version = 1
[[cases]]
name = "triggered-difference"
platform = "linux"
[cases.host]
environment = ["ENABLE"]
[cases.expect]
valid = true
ready = false
analysis_diagnostic_codes = ["code_analysis_declaration_difference", "code_analysis_declaration_uncorroborated"]
[[cases]]
name = "not-triggered"
platform = "windows"
[cases.expect]
valid = true
analysis_diagnostic_codes = []
"#.to_vec());
    let lint = lint_preset_path(&source, Path::new("/repo"), Path::new("app/demo")).await;
    assert!(!lint.valid, "{lint:?}");
    assert!(lint.complete);
    assert!(!lint.analyzed_entries.is_empty());
    assert!(
        lint.diagnostics
            .iter()
            .any(|d| d.code == "code_analysis_declaration_difference")
    );
    let fixture = test_preset_path(&source, Path::new("/repo"), Path::new("app/demo")).await;
    assert!(fixture.valid, "{fixture:?}");
    assert!(fixture.complete);
    assert_eq!(fixture.schema_version, 2);
    assert!(fixture.cases.iter().all(|c| c.passed));
    let plan = plan_preset_path(
        &source,
        Path::new("/repo"),
        Path::new("app/demo"),
        RuntimePlatform::Linux,
    )
    .await;
    assert!(plan.complete);
    assert!(
        plan.plans.iter().all(|p| p.code_analysis.is_empty()),
        "ordinary author input has no ENABLE"
    );
    let json = serde_json::to_string(&plan).unwrap();
    assert!(!json.contains("shine.code-analysis.v1"));
    assert!(!json.contains("/repo"));
}

#[tokio::test]
async fn author_terminal_budget_is_incomplete_instead_of_expected_blocking() {
    let live = runtime("fetch('https://example.org');", "", true);
    let source = InMemoryHost::new();
    for (path, bytes) in live.presets().files() {
        source.put_file(format!("/repo/{path}"), bytes.clone());
    }
    let scope = crate::runtime::validation::load_preset_source_scope(
        &source,
        Path::new("/repo"),
        Path::new("app/demo"),
    )
    .await
    .unwrap();
    let mut state =
        crate::runtime::authoring::PresetAuthoringSyntheticState::empty(InMemoryHost::new());
    state.environment.insert("ENABLE".into(), "yes".into());
    let analysis = CodeAnalysisSession::default();
    analysis.0.lock().unwrap().sources = Session::new(
        analysis::Limits {
            command_files: 0,
            ..Default::default()
        },
        analysis.1.clone(),
    );
    let report = crate::runtime::authoring::plan_preset_source_scope_with_analysis(
        scope,
        RuntimePlatform::Linux,
        state,
        analysis,
    )
    .await;
    assert!(!report.complete);
    assert!(!report.valid);
    assert!(!report.ready);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "code_analysis_command_budget")
    );
}

#[tokio::test]
async fn fixture_terminal_failure_preserves_completed_and_marks_remaining_cases() {
    let live = runtime("export {};", "", true);
    let source = InMemoryHost::new();
    for (path, bytes) in live.presets().files() {
        source.put_file(format!("/repo/{path}"), bytes.clone());
    }
    source.put_file(
        "/repo/app/demo/shine.test.toml",
        br#"
schema_version = 1
[[cases]]
name = "completed"
platform = "linux"
[cases.expect]
valid = true
[[cases]]
name = "budget"
platform = "linux"
[cases.host]
environment = ["ENABLE"]
[cases.expect]
valid = false
ready = false
[[cases]]
name = "remaining"
platform = "linux"
"#
        .to_vec(),
    );
    let analysis = CodeAnalysisSession::default();
    analysis.0.lock().unwrap().sources = Session::new(
        analysis::Limits {
            command_files: 0,
            ..Default::default()
        },
        analysis.1.clone(),
    );
    let report = crate::runtime::fixture::test_preset_path_with_analysis(
        &source,
        Path::new("/repo"),
        Path::new("app/demo"),
        analysis,
    )
    .await;
    assert!(!report.complete);
    assert!(!report.valid);
    assert_eq!(report.cases.len(), 1);
    assert!(report.cases[0].passed);
    assert_eq!(report.unfinished_cases, ["budget", "remaining"]);
    assert_eq!(report.diagnostics, ["code_analysis_command_budget"]);
}

#[tokio::test]
async fn explicit_inspection_blocks_bad_file_but_evaluates_independent_file() {
    for bad in ["fetch('https://undeclared.example');", "const broken = ;"] {
        let base = runtime(bad, "", true);
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                br#"metadata_schema_version=2
 dest='~/.demo'
 [permissions]
 schema_version=1
 environment=[{name='ENABLE',sensitivity='plain'}]
 [[files]]
 source='bad'
 generator={script='bad.ts',runtime='bun',env=[{ source='ENABLE', sensitivity='plain' }],when_env='ENABLE'}
 [[files]]
 source='good'
 generator={script='good.ts',runtime='bun',env=[{ source='ENABLE', sensitivity='plain' }],when_env='ENABLE'}
 "#
                .to_vec(),
            )
            .file("app/demo/bad", b"bad fallback".to_vec())
            .file("app/demo/good", b"good fallback".to_vec())
            .file("app/demo/bad.ts", bad.as_bytes().to_vec())
            .file("app/demo/good.ts", b"console.log('good')".to_vec())
            .build();
        let runtime = CoreRuntime::new(InMemoryHost::new(), base.context().clone(), snapshot);
        runtime.host().queue_process_output(Ok(ProcessOutput {
            exit_code: Some(0),
            stdout: b"good".to_vec(),
            ..Default::default()
        }));
        let files = runtime
            .inspect_apps_with_options(
                AppInspectionOptions {
                    run_generators: true,
                    ..Default::default()
                },
                &mut NullObserver,
            )
            .await
            .unwrap();
        let bad = files
            .iter()
            .find(|f| {
                f.code_analysis
                    .as_ref()
                    .is_some_and(|s| s.script.as_deref() == Some("bad.ts"))
            })
            .unwrap();
        assert!(bad.desired_content.is_none());
        assert!(!bad.code_analysis.as_ref().unwrap().report.allowed);
        assert!(
            files
                .iter()
                .any(|f| f.desired_content.as_deref() == Some(b"good"))
        );
        let operations = runtime.host().operations();
        let runs = operations
            .iter()
            .filter_map(|op| {
                if let HostOperation::Run { args, .. } = op {
                    Some(args)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(runs.len(), 1);
        assert!(runs[0].iter().any(|a| a.ends_with("good.ts")));
        assert!(!operations.iter().any(|op|matches!(op,HostOperation::Write(path) if !path.starts_with(runtime.context().shine_dir.join("runtime/app")))));
    }
}

#[tokio::test]
async fn interrupted_generator_terminates_command_and_cleans_snapshot() {
    let runtime = runtime("console.log('ok')", "", true);
    runtime.host().queue_process_output(Ok(ProcessOutput {
        interrupted: true,
        exit_code: None,
        ..Default::default()
    }));
    let error = runtime
        .inspect_apps_with_options(
            AppInspectionOptions {
                run_generators: true,
                ..Default::default()
            },
            &mut NullObserver,
        )
        .await
        .unwrap_err();
    assert!(error.is::<TerminalFailure>(), "{error:#}");
    let operations = runtime.host().operations();
    assert!(operations.iter().any(|op|matches!(op,HostOperation::RemoveDirectory(path) if path.starts_with(runtime.context().shine_dir.join("runtime/app")))));
    let runs = operations
        .iter()
        .filter(|op| matches!(op, HostOperation::Run { .. }))
        .count();
    assert!(
        runtime
            .inspect_apps_with_options(
                AppInspectionOptions {
                    run_generators: true,
                    ..Default::default()
                },
                &mut NullObserver
            )
            .await
            .is_err()
    );
    assert_eq!(
        runtime
            .host()
            .operations()
            .iter()
            .filter(|op| matches!(op, HostOperation::Run { .. }))
            .count(),
        runs
    );
}

#[tokio::test]
async fn launcher_rejects_missing_evidence_before_snapshot_write() {
    let runtime = runtime("console.log('ok')", "", true);
    let error = runtime
        .run_app_script(
            "demo",
            "app/demo/gen.ts",
            ArtifactRuntime::Bun,
            "generator:config",
            &[],
            ProcessRequest::default(),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("code_analysis_evidence_missing"));
    assert!(runtime.host().operations().is_empty());
}

#[tokio::test]
async fn dropped_preparation_latches_cancellation_before_another_entry() {
    let code = "const x = 1;\n".repeat(50_000);
    let runtime = runtime(&code, "", true);
    let category = runtime.app_categories(Some("demo")).unwrap().remove(0);
    let binding = runtime
        .generator_analysis(
            &category,
            "config",
            category.files[0].generator.as_ref().unwrap(),
        )
        .unwrap();
    let requested = [binding];
    let mut preparation = Box::pin(runtime.prepare_code_analysis(&requested));
    std::future::poll_fn(|cx| {
        assert!(std::future::Future::poll(preparation.as_mut(), cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    drop(preparation);
    assert!(
        runtime
            .analysis
            .1
            .load(std::sync::atomic::Ordering::Acquire)
    );
    let error = runtime.prepare_code_analysis(&requested).await.unwrap_err();
    assert!(error.is::<TerminalFailure>());
    assert!(runtime.host().operations().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn real_process_cancellation_cleans_and_forced_abort_never_reuses_snapshot() {
    let root = std::env::temp_dir().join(format!("shine-analysis-cancel-{}", uuid::Uuid::new_v4()));
    let context = RuntimeContext::isolated(
        root.join("home"),
        root.join("state"),
        root.join("presets"),
        root.join("bin"),
        RuntimePlatform::current(),
    );
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "app/demo/wait.sh",
            b"#!/bin/sh\nprintf ready > ready\nsleep 30\n".to_vec(),
        )
        .build();
    let invocation_root = context.shine_dir.join("runtime/app/demo");
    let mut retained = None;
    for forced in [true, false] {
        let runtime = Arc::new(CoreRuntime::new(
            RealHost,
            context.clone(),
            snapshot.clone(),
        ));
        let category = runtime.app_categories(Some("demo")).unwrap().remove(0);
        let binding = runtime
            .app_analysis_binding(
                &category,
                "test",
                Some("wait.sh"),
                ArtifactRuntime::Native,
                &[],
            )
            .unwrap();
        runtime.prepare_code_analysis(&[binding]).await.unwrap();
        let child = runtime.clone();
        let task = tokio::spawn(async move {
            child
                .run_app_script(
                    "demo",
                    "app/demo/wait.sh",
                    ArtifactRuntime::Native,
                    "test",
                    &[],
                    ProcessRequest::default(),
                )
                .await
        });
        let invocation = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(entries) = std::fs::read_dir(&invocation_root) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if retained.as_ref() != Some(&path) && path.join("source/ready").exists() {
                            return path;
                        }
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        if forced {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            assert!(invocation.join("source/ready").exists());
            retained = Some(invocation);
        } else {
            runtime.analysis.cancel();
            let error = task.await.unwrap().unwrap_err();
            assert!(error.is::<TerminalFailure>());
            assert!(!invocation.exists());
            assert!(retained.as_ref().unwrap().join("source/ready").exists());
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn shared_source_keeps_entry_declarations_and_environment_mapping_independent() {
    let base = runtime("", "", true);
    let mut context = base.context().clone();
    context.env.insert("A".into(), "first".into());
    context.env.insert("B".into(), "second".into());
    let mut snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded);
    for (category, source, sensitivity, network) in [
        ("a", "A", "plain", ""),
        (
            "b",
            "B",
            "secret",
            "network=[{scope='host',host='example.org'}]",
        ),
    ] {
        let metadata = format!(
            "metadata_schema_version=2\ndest='~/.{category}'\n[permissions]\nschema_version=1\nenvironment=[{{name='{source}',sensitivity='{sensitivity}'}}]\n{network}\n[[files]]\nsource='config'\ngenerator={{script='gen.ts',runtime='bun',env=[{{source='{source}',target='MAPPED',sensitivity='{sensitivity}'}}],when_env='{source}'}}\n"
        );
        snapshot = snapshot
            .file(format!("app/{category}/shine.toml"), metadata.into_bytes())
            .file(format!("app/{category}/config"), b"fallback".to_vec())
            .file(
                format!("app/{category}/gen.ts"),
                b"fetch('https://example.org'); console.log(process.env.MAPPED);".to_vec(),
            );
    }
    let runtime = CoreRuntime::new(InMemoryHost::new(), context, snapshot.build());
    runtime.host().queue_process_output(Ok(ProcessOutput {
        exit_code: Some(0),
        stdout: b"second".to_vec(),
        ..Default::default()
    }));
    let files = runtime
        .inspect_apps_with_options(
            AppInspectionOptions {
                run_generators: true,
                ..Default::default()
            },
            &mut NullObserver,
        )
        .await
        .unwrap();
    let a = files.iter().find(|f| f.category.name == "a").unwrap();
    let b = files.iter().find(|f| f.category.name == "b").unwrap();
    assert!(!a.code_analysis.as_ref().unwrap().report.allowed);
    assert!(b.code_analysis.as_ref().unwrap().report.allowed);
    assert_eq!(b.desired_content.as_deref(), Some(b"second".as_slice()));
    assert_eq!(runtime.analysis.0.lock().unwrap().sources.parsed_files(), 1);
    let processes = runtime.host().process_requests();
    assert_eq!(processes.len(), 1);
    assert_eq!(processes[0].env.get("MAPPED").unwrap(), "second");
}

#[tokio::test]
async fn frontend_explicit_evaluation_cannot_bypass_core_admission() {
    let service = crate::frontend::FrontendService::new(runtime(
        "fetch('https://private.example/SECRET_LITERAL');",
        "",
        true,
    ));
    let result = service
        .inspect_apps_with_options(
            AppInspectionOptions {
                run_generators: true,
                ..Default::default()
            },
            &mut NullObserver,
        )
        .await
        .unwrap();
    assert_eq!(result.report.code_analysis.len(), 1);
    assert!(!result.report.code_analysis[0].report.allowed);
    let json = serde_json::to_string(&result.report).unwrap();
    assert!(!json.contains("SECRET_LITERAL"));
    assert!(!json.contains("private.example"));
    assert!(
        service
            .runtime()
            .host()
            .operations()
            .iter()
            .all(|op| matches!(op, HostOperation::Read(_)))
    );
}

#[tokio::test]
async fn changed_mapping_sensitivity_and_newly_triggered_scope_require_fresh_evidence() {
    let first = runtime("console.log(process.env.ENABLE)", "", true);
    let plan = first
        .plan_apps(request(LifecycleOperation::Install))
        .await
        .unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    for (from, to) in [
        (
            "env=[{ source='ENABLE', sensitivity='plain' }]",
            "env=[{ source='ENABLE', target='OTHER', sensitivity='plain' }]",
        ),
        ("sensitivity='plain'", "sensitivity='secret'"),
    ] {
        let mut snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded);
        for (path, bytes) in first.presets().files() {
            let bytes = if path.ends_with("shine.toml") {
                String::from_utf8(bytes.clone())
                    .unwrap()
                    .replace(from, to)
                    .into_bytes()
            } else {
                bytes.clone()
            };
            snapshot = snapshot.file(path, bytes);
        }
        let mut changed = CoreRuntime::new(
            InMemoryHost::new(),
            first.context().clone(),
            snapshot.build(),
        );
        changed.analysis = first.analysis.clone();
        let category = changed.app_categories(Some("demo")).unwrap().remove(0);
        let generator = category.files[0].generator.as_ref().unwrap();
        let binding = changed
            .generator_analysis(&category, "config", generator)
            .unwrap();
        assert!(changed.require_code_analysis(&binding).is_err());
        assert!(
            changed
                .run_app_script(
                    "demo",
                    "app/demo/gen.ts",
                    ArtifactRuntime::Bun,
                    "generator:config",
                    &generator.env,
                    ProcessRequest::default()
                )
                .await
                .is_err()
        );
        assert!(changed.host().operations().is_empty());
        let current = changed
            .plan_apps_prepared(request(LifecycleOperation::Install))
            .await
            .unwrap();
        assert!(approval.validate(&current).is_err());
    }
    let mut disabled = runtime("console.log(process.env.ENABLE)", "", false);
    let old = disabled
        .plan_apps(request(LifecycleOperation::Install))
        .await
        .unwrap();
    assert!(old.code_analysis.is_empty());
    let old_approval = PlanApprovalV1::for_reviewed_plan(&old).unwrap();
    disabled
        .context_mut_for_test()
        .env
        .insert("ENABLE".into(), "enabled".into());
    let missing = disabled
        .plan_apps_prepared(request(LifecycleOperation::Install))
        .await
        .unwrap();
    assert!(!missing.is_ready());
    assert!(old_approval.validate(&missing).is_err());
    assert_eq!(
        disabled.analysis.0.lock().unwrap().sources.parsed_files(),
        0
    );
    disabled
        .plan_apps(request(LifecycleOperation::Install))
        .await
        .unwrap();
    assert_eq!(
        disabled.analysis.0.lock().unwrap().sources.parsed_files(),
        1
    );
}

#[tokio::test]
async fn hook_artifact_and_teardown_differences_block_lifecycle_approval() {
    let base = runtime("", "", true);
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "app/demo/shine.toml",
            br#"metadata_schema_version=2
dest='~/.demo'
post_install={script='code.ts',runtime='bun'}
post_upgrade={script='code.ts',runtime='bun'}
[artifact]
script='code.ts'
teardown='teardown.ts'
runtime='bun'
[[files]]
source='config'
"#
            .to_vec(),
        )
        .file("app/demo/config", b"new".to_vec())
        .file(
            "app/demo/code.ts",
            b"fetch('https://missing.example');".to_vec(),
        )
        .file(
            "app/demo/teardown.ts",
            b"fetch('https://missing.example');".to_vec(),
        )
        .build();
    let runtime = CoreRuntime::new(InMemoryHost::new(), base.context().clone(), snapshot);
    let install = runtime
        .plan_apps(request(LifecycleOperation::Install))
        .await
        .unwrap();
    assert!(!install.is_ready());
    assert!(
        install
            .code_analysis
            .iter()
            .any(|b| b.scope.entry == "hook:0" && !b.scope.report.allowed)
    );
    for action in [AppArtifactAction::Apply, AppArtifactAction::Remove] {
        let plan = runtime
            .plan_app_artifact(AppArtifactPlanRequest {
                category: "demo".into(),
                action,
                input_versions: Default::default(),
            })
            .await
            .unwrap();
        assert!(!plan.is_ready());
        assert!(PlanApprovalV1::for_reviewed_plan(&plan).is_err());
        assert!(
            plan.code_analysis
                .iter()
                .any(|b| b.scope.entry == "artifact" && !b.scope.report.allowed)
        );
    }
    let destination = runtime.context().home_dir.join(".demo/config");
    runtime.host().put_file(&destination, b"old".to_vec());
    crate::install::AppManifest {
        entries: vec![crate::install::AppEntry {
            source: "app/demo/config".into(),
            destination,
            backup: None,
            content_hash: crate::install::hash_content(b"old"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
        ..Default::default()
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let before = runtime.host().operations().len();
    for operation in [LifecycleOperation::Upgrade, LifecycleOperation::Uninstall] {
        let plan = runtime.plan_apps(request(operation)).await.unwrap();
        assert!(!plan.is_ready());
        assert!(PlanApprovalV1::for_reviewed_plan(&plan).is_err());
        assert!(plan.code_analysis.iter().any(|b| !b.scope.report.allowed));
    }
    assert!(
        runtime.host().operations()[before..]
            .iter()
            .all(|op| matches!(op, HostOperation::Read(_)))
    );
}

#[tokio::test]
async fn windows_separator_scripts_use_the_same_snapshot_and_analysis_identity() {
    let base = runtime("", "", true);
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "app/demo/shine.toml",
            r#"metadata_schema_version=2
dest='~/.demo'
[permissions]
schema_version=1
environment=[{name='ENABLE',sensitivity='plain'}]
[[files]]
source='config'
generator={script='nested\gen.ts',runtime='bun',env=[{ source='ENABLE', sensitivity='plain' }],when_env='ENABLE'}
"#
            .replace(
                r"nested\gen.ts",
                if cfg!(windows) {
                    r"nested\gen.ts"
                } else {
                    "nested/gen.ts"
                },
            )
            .into_bytes(),
        )
        .file("app/demo/config", b"fallback".to_vec())
        .file("app/demo/nested/gen.ts", b"export {};".to_vec())
        .build();
    let runtime = CoreRuntime::new(InMemoryHost::new(), base.context().clone(), snapshot);
    let plan = runtime
        .plan_apps(request(LifecycleOperation::Install))
        .await
        .unwrap();
    assert!(plan.is_ready(), "{plan:?}");
    assert_eq!(
        plan.code_analysis[0].scope.script.as_deref(),
        Some("nested/gen.ts")
    );
    runtime.host().queue_process_output(Ok(ProcessOutput {
        exit_code: Some(0),
        stdout: b"generated".to_vec(),
        ..Default::default()
    }));
    let inspected = runtime
        .inspect_apps_with_options(
            AppInspectionOptions {
                run_generators: true,
                ..Default::default()
            },
            &mut NullObserver,
        )
        .await
        .unwrap();
    assert_eq!(
        inspected[0].desired_content.as_deref(),
        Some(b"generated".as_slice())
    );
    assert_eq!(runtime.analysis.0.lock().unwrap().sources.parsed_files(), 1);
}

#[tokio::test]
async fn lint_budget_failure_retains_completed_entry_differences() {
    let source = InMemoryHost::new();
    let live = runtime("fetch('https://missing.example');", "", true);
    for (path, bytes) in live.presets().files() {
        let bytes = if path.ends_with("shine.toml") {
            let mut metadata = String::from_utf8(bytes.clone()).unwrap();
            metadata.push_str("\n[[files]]\nsource='other'\ngenerator={script='other.ts',runtime='bun',env=[{ source='ENABLE', sensitivity='plain' }],when_env='ENABLE'}\n");
            metadata.into_bytes()
        } else {
            bytes.clone()
        };
        source.put_file(format!("/repo/{path}"), bytes);
    }
    source.put_file("/repo/app/demo/other", b"fallback".to_vec());
    source.put_file("/repo/app/demo/other.ts", b"export {};".to_vec());
    let session = CodeAnalysisSession::default();
    session.0.lock().unwrap().sources = Session::new(
        analysis::Limits {
            command_files: 1,
            ..Default::default()
        },
        session.1.clone(),
    );
    let report = crate::runtime::lint::lint_preset_path_with_analysis(
        &source,
        Path::new("/repo"),
        Path::new("app/demo"),
        session,
    )
    .await;
    assert!(!report.complete);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "code_analysis_command_budget")
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == "code_analysis_declaration_difference")
    );
    assert_eq!(report.analyzed_entries.len(), 1);
}
