use shine_core::install::{AppEntry, AppInstallStrategy, AppManifest, hash_content};
use shine_core::plan::PlanApprovalV1;
use shine_core::runtime::*;
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
        Box::pin(async { Ok(false) })
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

#[tokio::test]
async fn refresh_requires_the_exact_generated_file_receipt_even_with_force() {
    let root = std::env::temp_dir().join("shine-refresh-ownership-test");
    let mut context = RuntimeContext::isolated(
        root.clone(),
        root.join("state"),
        root.join("presets"),
        root.join("bin"),
        RuntimePlatform::current(),
    );
    context.env.insert("SOURCE".into(), "yes".into());
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file("app/demo/shine.toml", br#"metadata_schema_version = 2
dest = '~/.config/demo'
[permissions]
schema_version = 1
environment = [{ name = 'SOURCE', sensitivity = 'plain' }]
[[files]]
source = 'generated.txt'
generator = { script = 'gen.ts', runtime = 'bun', env = ['SOURCE'], when_env = 'SOURCE', auto = false }
"#.to_vec())
        .file("app/demo/generated.txt", b"fallback".to_vec())
        .file("app/demo/gen.ts", b"process.stdout.write('generated')".to_vec()).build();
    let host = InMemoryHost::new();
    let destination = root.join(".config/demo/generated.txt");
    host.put_file(&destination, b"installed".to_vec());
    let mut manifest = AppManifest {
        schema_version: 1,
        entries: vec![AppEntry {
            source: "app/other/config.txt".into(),
            destination: destination.clone(),
            backup: None,
            content_hash: hash_content(b"installed"),
            install_strategy: AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    };
    let runtime = CoreRuntime::new(host, context, snapshot);
    for owner in ["app/other/config.txt", "app/demo/old.txt"] {
        manifest.entries[0].source = owner.into();
        manifest
            .save(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap();
        let operations = runtime.host().operations();
        for force in [false, true] {
            for file in [None, Some("generated.txt".into())] {
                let request = AppRefreshPlanRequest {
                    category: "demo".into(),
                    file,
                    force,
                    input_versions: PlanningInputVersions::default(),
                };
                assert!(runtime.plan_app_refresh(request).await.is_err());
            }
        }
        assert!(
            runtime.host().operations()[operations.len()..]
                .iter()
                .all(|operation| matches!(operation, HostOperation::Read(_)))
        );
        assert_eq!(
            runtime.host().read(&destination).await.unwrap(),
            b"installed"
        );
    }
    manifest.entries[0].source = "app/demo/generated.txt".into();
    manifest
        .save(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    let request = AppRefreshPlanRequest {
        category: "demo".into(),
        file: None,
        force: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_app_refresh(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().queue_process_output(Ok(ProcessOutput {
        exit_code: Some(0),
        stdout: b"generated".to_vec(),
        stderr: vec![],
    }));
    runtime
        .refresh_app_generators_approved(request, &approval, &mut NullObserver, &mut Interaction)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"generated"
    );
    assert_eq!(
        AppManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .entries[0]
            .source,
        "app/demo/generated.txt"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn shell_snapshots_preserve_captured_executables_and_repair_mode_only_changes() {
    use shine_core::lifecycle::LifecycleOperation;
    use std::os::unix::fs::PermissionsExt;
    for transformed in [false, true] {
        let root =
            std::env::temp_dir().join(format!("shine-shell-mode-test-{}", uuid::Uuid::new_v4()));
        let presets = root.join("presets");
        let category = presets.join("shell/demo");
        std::fs::create_dir_all(&category).unwrap();
        let transforms = if transformed {
            "transforms = ['template']\n"
        } else {
            ""
        };
        let metadata = format!(
            "[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n{transforms}[files.permissions]\nschema_version = 1\n"
        );
        std::fs::write(category.join("shine.toml"), metadata).unwrap();
        for (name, mode) in [("demo.sh", 0o755), ("helper", 0o751), ("data.txt", 0o600)] {
            std::fs::write(category.join(name), b"#!/bin/sh\nprintf review-ok\n").unwrap();
            std::fs::set_permissions(category.join(name), std::fs::Permissions::from_mode(mode))
                .unwrap();
        }
        let snapshot = capture_preset_snapshot(
            &RealHost,
            PresetSnapshotRequest {
                source: PresetSnapshotSource::External(presets.clone()),
                overlay_root: None,
            },
        )
        .await
        .unwrap();
        // Reopening source modes during delivery must not replace captured intent.
        std::fs::set_permissions(
            category.join("demo.sh"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        let mut context = RuntimeContext::isolated(
            root.clone(),
            root.join("state"),
            presets,
            root.join("bin"),
            RuntimePlatform::current(),
        );
        context.is_external_presets = true;
        context.external_shell_mode = ExternalShellMode::Snapshot;
        context.shell = ShellType::Bash;
        let mut runtime = CoreRuntime::new(RealHost, context, snapshot);
        let requirements = runtime
            .external_code_requirements("shell/demo/demo")
            .await
            .unwrap()
            .requirements;
        runtime.context_mut_for_cli().trust_grants = requirements
            .iter()
            .map(shine_core::trust::TrustGrantV1::for_reviewed_requirement)
            .collect();
        let mut request = ShellPlanRequest {
            operation: LifecycleOperation::Install,
            target: Some("demo".into()),
            force: false,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        };
        let plan = runtime.plan_shells(request.clone()).await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        runtime
            .install_shells_approved(request.clone(), &approval)
            .await
            .unwrap();
        let installed = root.join("state/installed/shell/demo");
        for path in [root.join("bin/demo"), installed.join("helper")] {
            let output = std::process::Command::new(path).output().unwrap();
            assert!(output.status.success());
            assert_eq!(output.stdout, b"review-ok");
        }
        assert_eq!(
            std::fs::metadata(installed.join("data.txt"))
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );
        assert!(runtime.shell_snapshot_current("demo").await.unwrap());
        std::fs::set_permissions(
            installed.join("helper"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(!runtime.shell_snapshot_current("demo").await.unwrap());
        assert!(
            runtime
                .inspect_shells()
                .await
                .unwrap()
                .iter()
                .any(|file| file.status == InspectionFileStatus::UpdateAvail)
        );
        request.operation = LifecycleOperation::Upgrade;
        let plan = runtime.plan_shells(request.clone()).await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        runtime
            .upgrade_shells_approved(request, &approval)
            .await
            .unwrap();
        assert!(runtime.shell_snapshot_current("demo").await.unwrap());
        assert!(
            std::process::Command::new(installed.join("helper"))
                .output()
                .unwrap()
                .status
                .success()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn sys_bootstrap_approved_snapshots_execute_helpers_and_clean_success_and_failure() {
    for failure in [false, true] {
        let root =
            std::env::temp_dir().join(format!("shine-sys-invocation-{}", uuid::Uuid::new_v4()));
        let mut context = RuntimeContext::isolated(
            root.clone(),
            root.join("state"),
            root.join("presets"),
            root.join("bin"),
            RuntimePlatform::current(),
        );
        context.shell = ShellType::Bash;
        let tail = if failure {
            "exit 2"
        } else {
            "touch \"$SHINE_TARGET_HOME/detected\""
        };
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "sys/test/shine.toml",
                br#"version = 2
[[items]]
id = 'demo'
label = 'Demo'
permissions = { schema_version = 1 }
detect = { kind = 'path', path = '~/detected' }
install = { kind = 'script', path = 'install.sh' }
"#
                .to_vec(),
            )
            .file(
                "sys/test/install.sh",
                format!("set -e\n./helper\n{tail}\n").into_bytes(),
            )
            .file_with_executable(
                "sys/test/helper",
                b"#!/bin/sh\nprintf helper-ran\n".to_vec(),
                true,
            )
            .build();
        let runtime = CoreRuntime::new(RealHost, context, snapshot);
        let request = SysBootstrapPlanRequest {
            os_id: "test".into(),
            item_ids: vec!["demo".into()],
            sys_shell: "bash".into(),
            force_profile: false,
            input_versions: Default::default(),
        };
        let plan = runtime.plan_sys_bootstrap(request.clone()).await.unwrap();
        let cleanup = shine_core::plan::PermissionV1::Filesystem {
            access: shine_core::plan::FilesystemAccessV1::Remove,
            path: "shine:runtime/sys/test".into(),
        };
        assert!(plan.permissions.required.contains(&cleanup));
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        let report = runtime
            .run_sys_bootstrap_approved(request, &approval, &mut Interaction, &mut NullObserver)
            .await
            .unwrap();
        assert_eq!(
            report.outcomes[0].status,
            if failure {
                SysItemStatus::Failed
            } else {
                SysItemStatus::Installed
            }
        );
        assert!(
            report.outcomes[0]
                .logs
                .iter()
                .any(|line| line.contains("helper-ran"))
        );
        assert_eq!(
            std::fs::read_dir(root.join("state/runtime/sys/test"))
                .unwrap()
                .count(),
            0
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
