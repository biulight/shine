//! Exercise the distinct code entry points through their process and report boundaries.
use super::*;
use crate::runtime::{
    InMemoryHost, NullObserver, PresetSnapshot, PresetSourceKind, ProcessOutput, RuntimeContext,
    RuntimePlatform,
};

async fn runtime(kind: PresetSourceKind) -> CoreRuntime<InMemoryHost> {
    let root = std::env::temp_dir().join("shine-app-execution-contracts");
    let mut context = RuntimeContext::isolated(
        root.join("home"),
        root.join("state"),
        root.join("presets"),
        root.join("bin"),
        RuntimePlatform::Linux,
    );
    context.is_external_presets = kind == PresetSourceKind::External;
    context.env = BTreeMap::from([
        ("ENABLE".into(), "yes".into()),
        ("VALUE".into(), "selected".into()),
        ("UNRELATED".into(), "private".into()),
    ]);
    let snapshot = PresetSnapshot::builder(kind)
        .file(
            "app/demo/shine.toml",
            br#"metadata_schema_version = 2
dest = '~/.config/demo'
post_install = { script = 'code.sh', env = ['VALUE=MAPPED'] }
[artifact]
script = 'code.sh'
teardown = 'code.sh'
env = ['VALUE=MAPPED']
[permissions]
schema_version = 1
filesystem = [{ access = ['execute'], base = 'preset', path = 'code.sh' }]
environment = [{ name = 'ENABLE', sensitivity = 'plain' }, { name = 'VALUE', sensitivity = 'plain' }]
[[files]]
source = 'config'
generator = { script = 'code.sh', env = ['ENABLE', 'VALUE=MAPPED'], when_env = 'ENABLE', auto = false }
"#.to_vec(),
        )
        .file("app/demo/config", b"fallback".to_vec())
        .file("app/demo/code.sh", b"#!/bin/sh\nexit 0\n".to_vec())
        .file(
            "app/other/shine.toml",
            b"metadata_schema_version = 2\n dest = '~/.config/other'\n[[files]]\nsource = 'config'\n".to_vec(),
        )
        .file("app/other/config", b"other".to_vec())
        .build();
    let runtime = CoreRuntime::new(InMemoryHost::new(), context, snapshot);
    let category = runtime.app_categories(Some("demo")).unwrap().remove(0);
    runtime.lint_app_code(&category).await.unwrap();
    let binding = runtime
        .generator_analysis(&category, "config", &generator(true).generator)
        .unwrap();
    runtime.prepare_code_analysis(&[binding]).await.unwrap();
    runtime
}

fn mappings() -> Vec<EnvVarSpec> {
    vec![EnvVarSpec {
        source: "VALUE".into(),
        target: "MAPPED".into(),
    }]
}

fn generator(explicit: bool) -> AppGeneratorRequest {
    AppGeneratorRequest {
        category: "demo".into(),
        source: "config".into(),
        explicit,
        generator: AppGenerator {
            script: "code.sh".into(),
            runtime: ArtifactRuntime::Native,
            env: mappings(),
            when_env: "ENABLE".into(),
            auto: false,
        },
    }
}

fn artifact(implicit: bool) -> AppArtifactRequest {
    AppArtifactRequest {
        category: "demo".into(),
        artifact: AppArtifact {
            script: "code.sh".into(),
            teardown: Some("code.sh".into()),
            runtime: ArtifactRuntime::Native,
            env: mappings(),
        },
        action: if implicit {
            AppArtifactAction::Remove
        } else {
            AppArtifactAction::Apply
        },
        implicit,
        dry_run: false,
    }
}

#[tokio::test]
async fn generator_gates_before_effects_and_retains_bounded_allowlisted_execution() {
    let runtime = runtime(PresetSourceKind::Embedded).await;
    assert!(
        runtime
            .run_app_generator(generator(false), &mut NullObserver)
            .await
            .unwrap()
            .is_none()
    );
    assert!(runtime.host().operations().is_empty());
    let mut disabled = generator(true);
    disabled.generator.when_env = "ABSENT".into();
    assert!(
        runtime
            .run_app_generator(disabled, &mut NullObserver)
            .await
            .unwrap()
            .is_none()
    );
    assert!(runtime.host().operations().is_empty());
    runtime.host().queue_process_output(Ok(ProcessOutput {
        exit_code: Some(0),
        stdout: b"generated".to_vec(),
        ..Default::default()
    }));
    assert_eq!(
        runtime
            .run_app_generator(generator(true), &mut NullObserver)
            .await
            .unwrap()
            .unwrap(),
        b"generated"
    );
    let requests = runtime.host().process_requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(
        request.env.get("MAPPED").map(String::as_str),
        Some("selected")
    );
    assert!(!request.env.contains_key("VALUE"));
    assert!(!request.env.contains_key("UNRELATED"));
    assert_eq!(request.timeout, Some(std::time::Duration::from_secs(30)));
    assert_eq!(request.stdout_limit, Some(8 * 1024 * 1024));
    assert_eq!(request.stderr_limit, Some(64 * 1024));
    assert_eq!(request.io, ProcessIo::Captured);

    runtime.host().queue_process_output(Ok(ProcessOutput {
        exit_code: Some(1),
        stderr: b"private-token".to_vec(),
        ..Default::default()
    }));
    let error = runtime
        .run_app_generator(generator(true), &mut NullObserver)
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "app 'demo' generator exited with 1 (details redacted)"
    );
}

#[tokio::test]
async fn hook_failure_stops_its_category_preserves_prior_notes_and_continues_other_categories() {
    let runtime = runtime(PresetSourceKind::Embedded).await;
    let mut category = runtime.app_categories(Some("demo")).unwrap().remove(0);
    let hook = |name: &str| AppHook {
        action: AppHookAction::Command(name.into()),
        args: vec!["arg".into()],
        show_output: true,
        env: mappings(),
    };
    category.post_install = vec![hook("first"), hook("fail"), hook("must-not-run")];
    let mut other = category.clone();
    other.name = "other".into();
    other.post_install = vec![hook("other-category")];
    runtime.lint_app_code(&category).await.unwrap();
    runtime.lint_app_code(&other).await.unwrap();
    for (exit_code, stdout) in [(0, " first note \n"), (1, "ignored"), (0, "other note")] {
        runtime.host().queue_process_output(Ok(ProcessOutput {
            exit_code: Some(exit_code),
            stdout: stdout.as_bytes().to_vec(),
            ..Default::default()
        }));
    }
    let report = runtime
        .run_app_hooks(
            AppHookRequest {
                categories: vec![category, other],
                changed: BTreeSet::from(["demo".into(), "other".into()]),
                phase: AppHookPhase::PostInstall,
                show_success: true,
            },
            &mut NullObserver,
        )
        .await;
    assert_eq!(report.notes, ["first note", "other note"]);
    assert_eq!(
        report
            .outcomes
            .iter()
            .map(|outcome| outcome.status)
            .collect::<Vec<_>>(),
        [LifecycleStatus::Failed, LifecycleStatus::Changed]
    );
    assert_eq!(report.outcomes[0].diagnostic_codes, ["app_hook_failed"]);
    let requests = runtime.host().process_requests();
    assert_eq!(
        requests
            .iter()
            .map(|request| request.program.as_str())
            .collect::<Vec<_>>(),
        ["first", "fail", "other-category"]
    );
    assert!(
        requests
            .iter()
            .all(|request| request.env == BTreeMap::from([("MAPPED".into(), "selected".into())]))
    );
}

#[tokio::test]
async fn artifact_retains_preview_implicit_failure_and_explicit_io_contracts() {
    for implicit in [true, false] {
        let runtime = runtime(PresetSourceKind::Embedded).await;
        let mut request = artifact(implicit);
        request.dry_run = true;
        assert_eq!(
            runtime
                .run_app_artifact(request, &mut NullObserver)
                .await
                .unwrap()
                .status,
            LifecycleStatus::Previewed
        );
        assert!(runtime.host().operations().is_empty());
        runtime.host().queue_process_output(Ok(ProcessOutput {
            exit_code: Some(7),
            ..Default::default()
        }));
        let result = runtime
            .run_app_artifact(artifact(implicit), &mut NullObserver)
            .await;
        if implicit {
            let outcome = result.unwrap();
            assert_eq!(outcome.status, LifecycleStatus::Failed);
            assert_eq!(outcome.diagnostic_codes, ["app_artifact_teardown_failed"]);
        } else {
            assert_eq!(
                result.unwrap_err().to_string(),
                "artifact script for 'demo' exited with 7"
            );
        }
        let requests = runtime.host().process_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].io,
            if implicit {
                ProcessIo::Captured
            } else {
                ProcessIo::Inherit
            }
        );
        assert_eq!(
            requests[0].env.get("MAPPED").map(String::as_str),
            Some("selected")
        );
        assert!(!requests[0].env.contains_key("UNRELATED"));
        assert!(requests[0].timeout.is_none());
    }
}

#[tokio::test]
async fn untrusted_entrypoints_reject_or_skip_without_any_host_effects() {
    let runtime = runtime(PresetSourceKind::External).await;
    assert!(
        runtime
            .run_app_generator(generator(true), &mut NullObserver)
            .await
            .unwrap_err()
            .to_string()
            .contains("scoped external-code trust")
    );
    assert_eq!(
        runtime
            .run_app_artifact(artifact(true), &mut NullObserver)
            .await
            .unwrap()
            .status,
        LifecycleStatus::Skipped
    );
    assert!(
        runtime
            .run_app_artifact(artifact(false), &mut NullObserver)
            .await
            .is_err()
    );
    let mut category = runtime.app_categories(Some("demo")).unwrap().remove(0);
    category.post_install = vec![AppHook {
        action: AppHookAction::Command("must-not-run".into()),
        args: vec![],
        show_output: true,
        env: mappings(),
    }];
    let report = runtime
        .run_app_hooks(
            AppHookRequest {
                categories: vec![category],
                changed: BTreeSet::from(["demo".into()]),
                phase: AppHookPhase::PostInstall,
                show_success: true,
            },
            &mut NullObserver,
        )
        .await;
    assert_eq!(report.outcomes[0].status, LifecycleStatus::Skipped);
    assert_eq!(
        report.outcomes[0].diagnostic_codes,
        ["app_hook_permission_required"]
    );
    assert!(runtime.host().operations().is_empty());
}

#[tokio::test]
async fn script_hook_keeps_fixed_environment_and_cleans_only_its_failed_invocation() {
    use crate::runtime::FileSystemObservationHost;

    let runtime = runtime(PresetSourceKind::Embedded).await;
    let category = runtime.app_categories(Some("demo")).unwrap().remove(0);
    let retained = runtime
        .context()
        .shine_dir
        .join("runtime/app/demo/other-run/keep");
    runtime.host().put_file(&retained, b"retained".to_vec());
    runtime
        .host()
        .queue_process_output(Err(anyhow::anyhow!("spawn failure")));

    let report = runtime
        .run_app_hooks(
            AppHookRequest {
                categories: vec![category],
                changed: BTreeSet::from(["demo".into()]),
                phase: AppHookPhase::PostInstall,
                show_success: true,
            },
            &mut NullObserver,
        )
        .await;

    assert_eq!(report.outcomes[0].status, LifecycleStatus::Failed);
    assert_eq!(report.outcomes[0].diagnostic_codes, ["app_hook_failed"]);
    let requests = runtime.host().process_requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(
        request.env.get("MAPPED").map(String::as_str),
        Some("selected")
    );
    assert_eq!(
        request.env.get("SHINE_APP_ID").map(String::as_str),
        Some("demo")
    );
    assert_eq!(
        Path::new(&request.env["SHINE_APP_DIR"]),
        request.cwd.as_deref().unwrap()
    );
    assert!(!request.env.contains_key("UNRELATED"));
    assert!(!request.env.contains_key("VALUE"));
    assert!(
        runtime
            .host()
            .metadata(request.cwd.as_deref().unwrap())
            .await
            .is_err()
    );
    assert_eq!(runtime.host().read(&retained).await.unwrap(), b"retained");
}

#[tokio::test]
async fn interrupted_teardown_stops_uninstall_before_file_and_receipt_removal() {
    use crate::runtime::{AppPlanRequest, FileSystemObservationHost, PlanningInputVersions};
    let runtime = runtime(PresetSourceKind::Embedded).await;
    let destination = runtime.context().home_dir.join(".config/demo/config");
    runtime.host().put_file(&destination, b"fallback".to_vec());
    AppManifest {
        entries: vec![AppEntry {
            source: "app/demo/config".into(),
            destination: destination.clone(),
            backup: None,
            content_hash: hash_content(b"fallback"),
            install_strategy: AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
        ..Default::default()
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let manifest_path = runtime.context().shine_dir.join("app-manifest.toml");
    let before = runtime.host().read(&manifest_path).await.unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo".into()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = crate::plan::PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().queue_process_output(Ok(ProcessOutput {
        interrupted: true,
        ..Default::default()
    }));
    let error = runtime
        .uninstall_apps_approved(
            request,
            &approval,
            &mut NullObserver,
            &mut super::lifecycle_tests::Interaction,
        )
        .await
        .unwrap_err();
    assert!(error.is::<crate::analysis::TerminalFailure>(), "{error:#}");
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"fallback"
    );
    assert_eq!(runtime.host().read(&manifest_path).await.unwrap(), before);
}
