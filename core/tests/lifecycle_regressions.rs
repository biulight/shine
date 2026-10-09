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

fn install_request(target: &str, force: bool) -> AppPlanRequest {
    AppPlanRequest {
        operation: shine_core::lifecycle::LifecycleOperation::Install,
        target: Some(target.into()),
        force,
        purge: false,
        prune_stale: false,
        input_versions: Default::default(),
    }
}

#[tokio::test]
async fn reinstall_preserves_modified_copy_json_and_generated_files_before_execution() {
    for (generated, json) in [(false, false), (false, true), (true, false)] {
        let root = std::env::temp_dir().join("shine-reinstall-preserve");
        let mut context = RuntimeContext::isolated(
            root.join("home"),
            root.join("state"),
            root.join("presets"),
            root.join("bin"),
            RuntimePlatform::current(),
        );
        context.env.insert("SOURCE".into(), "yes".into());
        let generator = if generated {
            "generator = { script = 'gen.ts', runtime = 'bun', env = ['SOURCE'], when_env = 'SOURCE', auto = false }\n"
        } else {
            ""
        };
        let strategy = if json {
            "install_mode = 'json-merge'\nmanaged_keys = ['owned']\n"
        } else {
            ""
        };
        let desired = if json {
            b"{\"owned\":\"managed\"}".as_slice()
        } else {
            b"managed"
        };
        let current = if json {
            b"{\"owned\":\"user-modified\",\"unowned\":true}".as_slice()
        } else {
            b"user-modified"
        };
        let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("app/demo/shine.toml", format!("metadata_schema_version = 2\ndest = '~/.config/demo'\n[permissions]\nschema_version = 1\nenvironment = [{{ name = 'SOURCE', sensitivity = 'plain' }}]\n[[files]]\nsource = 'config'\n{strategy}{generator}").into_bytes())
            .file("app/demo/config", desired.to_vec())
            .file("app/demo/gen.ts", b"process.stdout.write('managed')".to_vec()).build();
        let host = InMemoryHost::new();
        let destination = context.home_dir.join(".config/demo/config");
        let backup = destination.with_file_name("config.shine.bak");
        host.put_file(&destination, current.to_vec());
        host.put_file(&backup, b"original-backup".to_vec());
        let entry = AppEntry {
            source: "app/demo/config".into(),
            destination: destination.clone(),
            backup: Some(backup.clone()),
            content_hash: if json {
                hash_content(b"{\n  \"owned\": \"managed\"\n}\n")
            } else {
                hash_content(desired)
            },
            install_strategy: if json {
                AppInstallStrategy::JsonMerge {
                    managed_keys: vec!["owned".into()],
                }
            } else {
                AppInstallStrategy::Copy
            },
            uses_env: generated,
            requires_admin: false,
        };
        let manifest = AppManifest {
            schema_version: 1,
            entries: vec![entry],
        };
        manifest.save(&host, &context.shine_dir).await.unwrap();
        let runtime = CoreRuntime::new(host, context, snapshot);
        let request = install_request("demo", false);
        let plan = runtime.plan_apps(request.clone()).await.unwrap();
        assert!(
            plan.steps
                .iter()
                .any(|step| step.resource.as_deref() == Some("config")
                    && step.action == shine_core::plan::PlanActionV1::Preserve)
        );
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        let start = runtime.host().operations().len();
        let report = runtime
            .install_apps_approved(request, &approval, &mut NullObserver, &mut Interaction)
            .await
            .unwrap();
        assert_eq!(report.files[0].action, AppFileAction::UserModified);
        assert!(
            !runtime.host().operations()[start..]
                .iter()
                .any(|op| matches!(op, HostOperation::Run { .. }))
        );
        assert_eq!(runtime.host().read(&destination).await.unwrap(), current);
        assert_eq!(
            runtime.host().read(&backup).await.unwrap(),
            b"original-backup"
        );
        assert_eq!(
            AppManifest::load(runtime.host(), &runtime.context().shine_dir)
                .await
                .unwrap()
                .entries,
            manifest.entries
        );
        // An explicitly reviewed force install still permits the replacement.
        if !generated {
            let request = install_request("demo", true);
            let plan = runtime.plan_apps(request.clone()).await.unwrap();
            let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
            runtime
                .install_apps_approved(request, &approval, &mut NullObserver, &mut Interaction)
                .await
                .unwrap();
            assert_ne!(runtime.host().read(&destination).await.unwrap(), current);
        }
    }
}

#[tokio::test]
async fn app_install_waits_before_replanning_and_keeps_concurrent_receipts() {
    for changed_target in ["other", "demo"] {
        let root = std::env::temp_dir().join("shine-app-serialized-install");
        let mut context = RuntimeContext::isolated(
            root.join("home"),
            root.join("state"),
            root.join("presets"),
            root.join("bin"),
            RuntimePlatform::current(),
        );
        context.is_external_presets = true;
        let host = InMemoryHost::new();
        let mut manifest = AppManifest {
            schema_version: 1,
            entries: vec![],
        };
        manifest.save(&host, &context.shine_dir).await.unwrap();
        let runtime = CoreRuntime::new(host.clone(), context.clone(), PresetSnapshot::builder(PresetSourceKind::External)
            .file("app/demo/shine.toml", b"metadata_schema_version = 2\ndest = '~/.config/demo'\n[[files]]\nsource = 'config'\n".to_vec())
            .file("app/demo/config", b"managed".to_vec()).build());
        let request = install_request("demo", false);
        let approval =
            PlanApprovalV1::for_reviewed_plan(&runtime.plan_apps(request.clone()).await.unwrap())
                .unwrap();
        // A competing lifecycle owns the lock and is about to commit its receipt.
        let guard = host
            .acquire_operation_lock(&context.shine_dir.join("app-lifecycle.lock"))
            .await
            .unwrap();
        let mut observer = NullObserver;
        let mut interaction = Interaction;
        let execution =
            runtime.install_apps_approved(request, &approval, &mut observer, &mut interaction);
        tokio::pin!(execution);
        std::future::poll_fn(|cx| {
            assert!(
                execution.as_mut().poll(cx).is_pending(),
                "install did not wait for the lifecycle lock"
            );
            std::task::Poll::Ready(())
        })
        .await;
        let destination = context
            .home_dir
            .join(format!(".config/{changed_target}/config"));
        host.put_file(&destination, b"concurrent".to_vec());
        let other_entry = AppEntry {
            source: format!("app/{changed_target}/config"),
            destination: destination.clone(),
            backup: None,
            content_hash: hash_content(b"concurrent"),
            install_strategy: AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        };
        manifest.upsert(other_entry.clone());
        manifest.save(&host, &context.shine_dir).await.unwrap();
        drop(guard);
        let result = execution.await;
        let manifest = AppManifest::load(&host, &context.shine_dir).await.unwrap();
        assert_eq!(
            manifest.find_by_source(&other_entry.source),
            Some(&other_entry)
        );
        assert_eq!(host.read(&destination).await.unwrap(), b"concurrent");
        if changed_target == "other" {
            result.unwrap();
            assert_eq!(manifest.entries.len(), 2);
        } else {
            assert!(
                result.is_err(),
                "a receipt changed while waiting must invalidate approval"
            );
            assert_eq!(manifest.entries.len(), 1);
        }
    }
}

#[tokio::test]
async fn static_app_resources_never_infer_code_from_their_names() {
    use shine_core::lifecycle::LifecycleOperation;

    for filename in [
        "config.toml",
        "generator:config.toml",
        "hook:config.toml",
        "artifact:config.toml",
        "profile-state",
    ] {
        for source_kind in [PresetSourceKind::Embedded, PresetSourceKind::External] {
            let snapshot = PresetSnapshot::builder(source_kind)
                .file(
                    "app/demo/shine.toml",
                    format!("metadata_schema_version = 2\ndest = '~/.config/demo'\n[[files]]\nsource = '{filename}'\ntarget = 'config.toml'\n").into_bytes(),
                )
                .file(format!("app/demo/{filename}"), b"static data".to_vec())
                .build();
            let root = std::env::temp_dir().join("shine-static-entry-test");
            let mut context = RuntimeContext::isolated(
                root.join("home"),
                root.join("state"),
                root.join("presets"),
                root.join("bin"),
                RuntimePlatform::current(),
            );
            context.is_external_presets = source_kind == PresetSourceKind::External;
            let runtime = CoreRuntime::new(InMemoryHost::new(), context, snapshot);
            let request = AppPlanRequest {
                operation: LifecycleOperation::Install,
                target: Some("demo".into()),
                force: false,
                purge: false,
                prune_stale: false,
                input_versions: PlanningInputVersions::default(),
            };
            let plan = runtime.plan_apps(request.clone()).await.unwrap();
            assert!(plan.is_ready());
            assert!(
                plan.code_boundaries.is_empty(),
                "{filename}: {:?}",
                plan.code_boundaries
            );
            let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
            runtime
                .install_apps_approved(request, &approval, &mut NullObserver, &mut Interaction)
                .await
                .unwrap();
            assert_eq!(
                runtime
                    .host()
                    .read(&runtime.context().home_dir.join(".config/demo/config.toml"))
                    .await
                    .unwrap(),
                b"static data"
            );
        }
    }
}

#[tokio::test]
async fn metadata_diagnostic_identity_does_not_depend_on_category_names() {
    for category in ["demo", "bun-lock-demo", "bun-package-demo"] {
        for (metadata, files, expected) in [
            (
                "[[files]]\nsource = 'config.toml'\ntransforms = ['unsupported']\n",
                vec!["config.toml"],
                "invalid_metadata",
            ),
            (
                "[[files]]\nsource = 'missing.toml'\n",
                vec!["config.toml"],
                "missing_reference",
            ),
            (
                "[[files]]\nsource = 'config.toml'\ntarget = 'same.toml'\n[[files]]\nsource = 'other.toml'\ntarget = 'same.toml'\n",
                vec!["config.toml", "other.toml"],
                "duplicate_target",
            ),
        ] {
            let root = std::env::temp_dir().join("shine-metadata-diagnostic-test");
            let path = root.join("app").join(category);
            let host = InMemoryHost::new();
            host.put_file(
                path.join("shine.toml"),
                format!("metadata_schema_version = 2\ndest = '~/.config/demo'\n{metadata}")
                    .into_bytes(),
            );
            for file in files {
                host.put_file(path.join(file), b"static data".to_vec());
            }
            let report = validate_preset_path(&host, &root, &path).await;
            assert!(!report.valid);
            assert_eq!(
                report.categories[0].diagnostics[0].code, expected,
                "{category}: {:?}",
                report.categories[0].diagnostics
            );
        }
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
