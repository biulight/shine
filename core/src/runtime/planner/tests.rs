use super::*;
use crate::plan::OpaqueCodeScopeV1;
use crate::runtime::{
    FileMetadata, HostError, HostOperation, InMemoryHost, PresetSnapshot, PresetSourceKind,
    RuntimeContext, RuntimePlatform, SplitDnsState,
};
use std::future::Future;
use std::pin::Pin;

async fn assert_frontend_journal(
    runtime: &CoreRuntime<InMemoryHost>,
    kind: crate::frontend::CapabilityKindV1,
    ready: bool,
) {
    use crate::frontend::{FrontendService, OperationStateV1};
    let expected = match kind {
        crate::frontend::CapabilityKindV1::App => {
            runtime.plan_app_operation_recovery().await.unwrap()
        }
        crate::frontend::CapabilityKindV1::Shell => {
            runtime.plan_shell_operation_recovery().await.unwrap()
        }
        crate::frontend::CapabilityKindV1::Sys => {
            runtime.plan_sys_operation_recovery().await.unwrap()
        }
    };
    let since = runtime.host().operations().len();
    let service = FrontendService::new(CoreRuntime::new(
        runtime.host().clone(),
        runtime.context().clone(),
        runtime.presets().clone(),
    ));
    let report = service.operation_state(kind).await.unwrap();
    assert_eq!(
        report.state,
        if ready {
            OperationStateV1::RecoveryReady
        } else {
            OperationStateV1::RecoveryBlocked
        }
    );
    assert_eq!(
        serde_json::to_value(report.recovery_plan.as_ref().unwrap()).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    let progress = report.journal.as_ref().unwrap();
    assert!(
        progress.prepared_actions + progress.applied_actions + progress.receipt_committed_actions
            > 0
    );
    let encoded = serde_json::to_string(&report).unwrap();
    let private_root = serde_json::to_string(&runtime.context().home_dir).unwrap();
    assert!(
        !encoded.contains(private_root.trim_matches('"')),
        "{encoded}"
    );
    assert!(!encoded.contains("approved_permissions"));
    assert!(
        runtime.host().operations()[since..]
            .iter()
            .all(|op| matches!(
                op,
                HostOperation::Read(_) | HostOperation::InspectSplitDns { .. }
            ))
    );
}
async fn frontend_recover(
    runtime: &CoreRuntime<InMemoryHost>,
    request: crate::frontend::ReviewRequest,
    expected: &PlanV1,
) {
    use crate::frontend::{ExecutionOptions, ExecutionResultV1, FrontendService};
    let service = FrontendService::new(CoreRuntime::new(
        runtime.host().clone(),
        runtime.context().clone(),
        runtime.presets().clone(),
    ));
    let readonly = service.read_only().request_review(&request).await.unwrap();
    assert_eq!(
        serde_json::to_value(&readonly.plan).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
    let trusted = service.into_trusted();
    let reviewed = trusted.review(request).await.unwrap();
    let approved = reviewed.approve_after_human_confirmation().unwrap();
    let mut events = Vec::new();
    let execution = trusted
        .apply(
            approved,
            ExecutionOptions::default(),
            &mut super::super::NullObserver,
            &mut Interaction,
            &mut events,
        )
        .await
        .unwrap();
    assert!(matches!(
        execution.report.result,
        ExecutionResultV1::Recovery { .. }
    ));
    assert_eq!(execution.report.operation, expected.operation);
    let encoded = serde_json::to_string(&execution.report).unwrap();
    assert!(!encoded.contains("approved_permissions"));
    assert!(!encoded.contains(runtime.context().home_dir.to_string_lossy().as_ref()));
    assert_eq!(events.len(), 2);
}

struct Interaction;

impl RuntimeInteraction for Interaction {
    fn confirm(&mut self, _code: &'static str, default: bool) -> Result<bool> {
        Ok(default)
    }

    fn authorize_admin<'a>(
        &'a mut self,
        _item_count: usize,
    ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>> {
        Box::pin(async { Ok(true) })
    }

    fn select_many(
        &mut self,
        _code: &'static str,
        _choices: &[String],
        defaults: &[String],
    ) -> Result<Vec<String>> {
        Ok(defaults.to_vec())
    }
}

struct NoAdminInteraction;

impl RuntimeInteraction for NoAdminInteraction {
    fn confirm(&mut self, _code: &'static str, default: bool) -> Result<bool> {
        Ok(default)
    }

    fn authorize_admin<'a>(
        &'a mut self,
        _item_count: usize,
    ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>> {
        Box::pin(async { panic!("preserved App files must not request administrator access") })
    }

    fn select_many(
        &mut self,
        _code: &'static str,
        _choices: &[String],
        defaults: &[String],
    ) -> Result<Vec<String>> {
        Ok(defaults.to_vec())
    }
}

#[derive(Clone)]
struct ObservationOnlyHost(InMemoryHost);

impl FileSystemObservationHost for ObservationOnlyHost {
    fn canonicalize<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<PathBuf, HostError>> + Send + 'a>> {
        self.0.canonicalize(path)
    }
    fn read<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, HostError>> + Send + 'a>> {
        self.0.read(path)
    }
    fn metadata<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<FileMetadata, HostError>> + Send + 'a>> {
        self.0.metadata(path)
    }
    fn read_dir<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<PathBuf>, HostError>> + Send + 'a>> {
        self.0.read_dir(path)
    }
    fn read_link<'a>(
        &'a self,
        path: &'a Path,
    ) -> Pin<Box<dyn Future<Output = Result<PathBuf, HostError>> + Send + 'a>> {
        self.0.read_link(path)
    }
}

impl SplitDnsObservationHost for ObservationOnlyHost {
    fn inspect_split_dns<'a>(
        &'a self,
        request: &'a SplitDnsRequest,
    ) -> Pin<Box<dyn Future<Output = Result<SplitDnsState>> + Send + 'a>> {
        self.0.inspect_split_dns(request)
    }
}

fn runtime(snapshot: PresetSnapshot) -> CoreRuntime<InMemoryHost> {
    let home = std::env::temp_dir().join("shine-planner-home");
    let shine = home.join(".shine");
    let mut context = RuntimeContext::isolated(
        home.clone(),
        shine.clone(),
        shine.join("presets"),
        shine.join("bin"),
        RuntimePlatform::Linux,
    );
    context.shell = super::super::ShellType::Bash;
    CoreRuntime::new(InMemoryHost::new(), context, snapshot)
}

fn static_copy_app_snapshot() -> PresetSnapshot {
    PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"managed".to_vec())
            .build()
}

#[tokio::test]
async fn app_approval_rejects_only_executable_intent_changing() {
    let snapshot = |executable| {
        PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"managed".to_vec())
            .file_with_executable("app/demo/helper", b"helper".to_vec(), executable)
            .build()
    };
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let reviewed = runtime(snapshot(false))
        .plan_apps(request.clone())
        .await
        .unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&reviewed).unwrap();
    let changed = runtime(snapshot(true)).plan_apps(request).await.unwrap();
    assert_eq!(reviewed.permissions, changed.permissions);
    assert_ne!(reviewed.inputs.preset, changed.inputs.preset);
    assert!(approval.validate(&changed).is_err());
}

fn privileged_static_copy_app_snapshot() -> PresetSnapshot {
    PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '/etc/demo'\n[permissions]\nschema_version = 1\nadministrator = true\n[[files]]\nsource = 'config.toml'\nrequires_admin = true\n".to_vec(),
            )
            .file("app/demo/config.toml", b"managed".to_vec())
            .build()
}

async fn seed_static_copy_app(
    runtime: &CoreRuntime<InMemoryHost>,
    current: &[u8],
    backup_content: Option<&[u8]>,
) -> (PathBuf, Option<PathBuf>) {
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    runtime.host().put_file(&destination, current.to_vec());
    let backup = backup_content.map(|content| {
        let backup = crate::install::backup_path(&destination);
        runtime.host().put_file(&backup, content.to_vec());
        backup
    });
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination: destination.clone(),
            backup: backup.clone(),
            content_hash: crate::install::hash_content(b"managed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    (destination, backup)
}

async fn seed_privileged_static_copy_app(
    runtime: &CoreRuntime<InMemoryHost>,
    current: &[u8],
    backup_content: Option<&[u8]>,
) -> (PathBuf, Option<PathBuf>) {
    let destination = PathBuf::from("/etc/demo/config.toml");
    runtime.host().put_file(&destination, current.to_vec());
    let backup = backup_content.map(|content| {
        let backup = crate::install::backup_path(&destination);
        runtime.host().put_file(&backup, content.to_vec());
        backup
    });
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination: destination.clone(),
            backup: backup.clone(),
            content_hash: crate::install::hash_content(b"managed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: true,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    (destination, backup)
}

fn observation_runtime(
    snapshot: PresetSnapshot,
) -> (CoreRuntime<ObservationOnlyHost>, InMemoryHost) {
    let home = std::env::temp_dir().join("shine-observation-only-home");
    let shine = home.join(".shine");
    let inner = InMemoryHost::new();
    (
        CoreRuntime::new(
            ObservationOnlyHost(inner.clone()),
            RuntimeContext::isolated(
                home,
                shine.clone(),
                shine.join("presets"),
                shine.join("bin"),
                RuntimePlatform::current(),
            ),
            snapshot,
        ),
        inner,
    )
}

fn bootstrap_snapshot(source: PresetSourceKind, with_permissions: bool) -> PresetSnapshot {
    let permissions = if with_permissions {
        "permissions = { schema_version = 1 }"
    } else {
        ""
    };
    PresetSnapshot::builder(source)
        .file(
            "sys/test/shine.toml",
            format!(
                r#"version = 2
[[items]]
id = 'tool'
label = 'Tool'
{permissions}
detect = {{ kind = 'path', path = '$HOME/.tool-present' }}
install = {{ kind = 'package', provider = 'homebrew', package = 'tool' }}
"#
            )
            .into_bytes(),
        )
        .build()
}

fn bootstrap_command_snapshot() -> PresetSnapshot {
    PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "sys/test/shine.toml",
            br#"version = 2
[[items]]
id = 'tool'
label = 'Tool'
permissions = { schema_version = 1 }
detect = { kind = 'command', command = 'tool', version_args = ['--version'] }
install = { kind = 'package', provider = 'homebrew', package = 'tool' }
"#
            .to_vec(),
        )
        .build()
}

fn bootstrap_request() -> SysBootstrapPlanRequest {
    SysBootstrapPlanRequest {
        os_id: "test".to_string(),
        item_ids: vec!["tool".to_string()],
        sys_shell: "zsh".to_string(),
        force_profile: false,
        input_versions: PlanningInputVersions::default(),
    }
}

#[tokio::test]
async fn sys_bootstrap_plan_is_observation_only_and_snapshot_bound() {
    let (runtime, host) = observation_runtime(bootstrap_snapshot(PresetSourceKind::Embedded, true));
    let missing = runtime
        .plan_sys_bootstrap(bootstrap_request())
        .await
        .unwrap();
    assert_eq!(missing.operation, PlanOperationV1::SysBootstrap);
    assert!(missing.is_ready());
    assert!(
        missing
            .steps
            .iter()
            .any(|step| { step.target == "sys/tool" && step.action == PlanActionV1::Execute })
    );
    assert!(
        missing
            .permissions
            .required
            .contains(&PermissionV1::Command {
                program: "brew".to_string(),
            })
    );
    assert!(
        host.operations()
            .iter()
            .all(|operation| matches!(operation, super::super::HostOperation::Read(_)))
    );

    host.put_file(
        runtime.context().home_dir.join(".tool-present"),
        b"present".to_vec(),
    );
    let present = runtime
        .plan_sys_bootstrap(bootstrap_request())
        .await
        .unwrap();
    assert!(
        present
            .steps
            .iter()
            .any(|step| { step.target == "sys/tool" && step.action == PlanActionV1::Update })
    );
    assert_ne!(missing.inputs.state, present.inputs.state);
}

#[tokio::test]
async fn sys_bootstrap_permission_scopes_preserve_origins_and_approval() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("sys/ubuntu/shine.toml", br#"version = 2
[[items]]
id = 'script'
label = 'Script'
detect = { kind = 'command', command = 'script-tool' }
install = { kind = 'script', path = 'install/tool.sh' }
permissions = { schema_version = 1, filesystem = [{ access = ['execute'], base = 'preset', path = 'install/tool.sh' }], commands = ['curl'], network = [{ scope = 'host', host = 'example.com' }] }
[[items]]
id = 'package'
label = 'Package'
detect = { kind = 'command', command = 'package-tool' }
install = { kind = 'package', provider = 'apt', package = 'package-tool' }
permissions = { schema_version = 1 }
"#.to_vec())
            .file("sys/ubuntu/install/tool.sh", b"#!/bin/sh\n".to_vec())
            .build();
    let (mut runtime, host) = observation_runtime(snapshot);
    runtime
        .context_mut_for_cli()
        .proxy_env
        .insert("HTTPS_PROXY".to_string(), "private-proxy-value".to_string());
    let mut request = bootstrap_request();
    request.os_id = "ubuntu".to_string();
    request.item_ids = vec!["script".to_string(), "package".to_string()];
    let plan = runtime.plan_sys_bootstrap(request).await.unwrap();
    assert!(plan.is_ready());
    assert_eq!(
        plan.permission_scopes
            .iter()
            .map(|scope| scope.target.as_deref())
            .collect::<Vec<_>>(),
        vec![
            Some("sys/script"),
            Some("sys/package"),
            Some("sys/profile"),
            None
        ]
    );
    let script = &plan.permission_scopes[0].permissions.required;
    let package = &plan.permission_scopes[1].permissions.required;
    assert!(plan.author_capabilities.contains(&PermissionV1::Command {
        program: "curl".to_string()
    }));
    assert!(!package.contains(&PermissionV1::Command {
        program: "curl".to_string()
    }));
    assert!(package.contains(&PermissionV1::Command {
        program: "apt-get".to_string()
    }));
    assert!(package.contains(&PermissionV1::Administrator));
    assert!(package.contains(&PermissionV1::Network {
        scope: NetworkScopeV1::Any
    }));
    let runtime_write = PermissionV1::Filesystem {
        access: FilesystemAccessV1::Write,
        path: "shine:runtime/sys/ubuntu".to_string(),
    };
    assert!(!script.contains(&runtime_write));
    assert!(
        plan.permission_scopes[3]
            .permissions
            .required
            .contains(&runtime_write)
    );
    assert_eq!(
        PermissionSetV1::new(
            plan.permission_scopes.iter().flat_map(|scope| scope
                .permissions
                .required
                .iter()
                .cloned())
        ),
        plan.permissions.required
    );
    let json = serde_json::to_string(&plan).unwrap();
    assert!(!json.contains("private-proxy-value"));
    assert_eq!(serde_json::from_str::<PlanV1>(&json).unwrap(), plan);
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let mut changed = plan.clone();
    changed.permission_scopes.swap(0, 1);
    assert!(approval.validate(&changed).is_err());
    assert!(
        host.operations()
            .iter()
            .all(|operation| matches!(operation, super::super::HostOperation::Read(_)))
    );
}

#[tokio::test]
async fn sys_bootstrap_plan_detects_an_executable_command_symlink() {
    let (runtime, host) = observation_runtime(bootstrap_command_snapshot());
    let target = runtime.context().home_dir.join("bin/tool-target");
    let other_target = runtime.context().home_dir.join("bin/other-tool-target");
    let command = runtime.context().home_dir.join(".local/bin/tool");
    host.put_file_with_mode(&target, b"tool".to_vec(), 0o100755);
    host.put_file_with_mode(&other_target, b"tool".to_vec(), 0o100755);
    host.symlink(&target, &command).await.unwrap();

    let plan = runtime
        .plan_sys_bootstrap(bootstrap_request())
        .await
        .unwrap();

    assert!(
        plan.steps
            .iter()
            .any(|step| { step.target == "sys/tool" && step.action == PlanActionV1::Update })
    );
    assert!(plan.is_ready());

    host.remove_file(&command).await.unwrap();
    host.symlink(&other_target, &command).await.unwrap();
    let retargeted = runtime
        .plan_sys_bootstrap(bootstrap_request())
        .await
        .unwrap();
    assert_ne!(plan.inputs.state, retargeted.inputs.state);
}

#[tokio::test]
async fn sys_bootstrap_package_adapter_does_not_require_an_empty_declaration() {
    let runtime = runtime(bootstrap_snapshot(PresetSourceKind::External, false));
    let plan = runtime
        .plan_sys_bootstrap(bootstrap_request())
        .await
        .unwrap();
    assert!(plan.is_ready());
    assert!(plan.permissions.uncomputable_codes.is_empty());
}

#[tokio::test]
async fn sys_bootstrap_approved_execution_rejects_changed_detection_state() {
    let runtime = runtime(bootstrap_snapshot(PresetSourceKind::Embedded, true));
    let request = bootstrap_request();
    let plan = runtime.plan_sys_bootstrap(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().put_file(
        runtime.context().home_dir.join(".tool-present"),
        b"present".to_vec(),
    );
    let mut interaction = Interaction;
    let mut observer = super::super::NullObserver;
    let error = runtime
        .run_sys_bootstrap_approved(request, &approval, &mut interaction, &mut observer)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Plan permission set changed after approval")
    );
    assert!(
        !runtime
            .host()
            .operations()
            .iter()
            .any(|operation| matches!(operation, super::super::HostOperation::Run { .. }))
    );
}

#[tokio::test]
async fn app_plan_is_pure_ready_and_payload_free() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"secret-looking-content".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let host = runtime.host().clone();
    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Install,
            target: Some("demo".to_string()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(plan.is_ready());
    assert!(
        plan.steps
            .iter()
            .any(|step| step.action == PlanActionV1::Create)
    );
    for access in [FilesystemAccessV1::Write, FilesystemAccessV1::Remove] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: "shine:app-operation-journal.toml".to_string(),
                })
        );
    }
    let encoded = serde_json::to_string(&plan).unwrap();
    assert!(!encoded.contains("secret-looking-content"));
    assert!(!host.operations().iter().any(|operation| matches!(
        operation,
        super::super::HostOperation::Write(_)
            | super::super::HostOperation::Remove(_)
            | super::super::HostOperation::Run { .. }
            | super::super::HostOperation::ApplySplitDns { .. }
    )));
}

#[tokio::test]
async fn unrestricted_opaque_code_is_added_only_when_app_code_is_triggered() {
    let opaque = PermissionV1::OpaqueCode {
        scope: OpaqueCodeScopeV1::Unrestricted,
    };
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let static_snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"metadata_schema_version = 2\ndest = '~/.config/demo'\n[permissions]\nschema_version = 2\nopaque_code = 'unrestricted'\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"managed".to_vec())
            .build();
    let static_plan = runtime(static_snapshot)
        .plan_apps(request.clone())
        .await
        .unwrap();
    assert!(static_plan.is_ready());
    assert!(!static_plan.permissions.required.contains(&opaque));

    let executable_snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"metadata_schema_version = 2\ndest = '~/.config/demo'\npost_install = { script = 'setup.sh' }\n[permissions]\nschema_version = 2\nopaque_code = 'unrestricted'\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"managed".to_vec())
            .file("app/demo/setup.sh", b"#!/bin/sh\n".to_vec())
            .build();
    let executable_plan = runtime(executable_snapshot)
        .plan_apps(request)
        .await
        .unwrap();
    assert!(executable_plan.is_ready(), "{executable_plan:?}");
    assert!(executable_plan.permissions.required.contains(&opaque));
}

#[cfg(unix)]
#[tokio::test]
async fn shell_permission_defaults_expose_unrestricted_command_effects() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "shell/demo/shine.toml",
                b"[permission_defaults]\nschema_version = 2\nopaque_code = 'unrestricted'\n[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\nplatforms = ['unix']\n".to_vec(),
            )
            .file("shell/demo/demo.sh", b"#!/bin/sh\n".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let request = ShellPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo/demo".to_string()),
        force: false,
        purge: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_shells(request.clone()).await.unwrap();

    assert!(plan.is_ready(), "{plan:?}");
    assert!(
        plan.permissions
            .required
            .contains(&PermissionV1::OpaqueCode {
                scope: OpaqueCodeScopeV1::Unrestricted,
            })
    );

    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .install_shells_approved(request, &approval)
        .await
        .unwrap();
    let uninstall = runtime
        .plan_shells(shell_uninstall_request())
        .await
        .unwrap();
    assert!(uninstall.is_ready(), "{uninstall:?}");
    assert!(
        !uninstall
            .permissions
            .required
            .contains(&PermissionV1::OpaqueCode {
                scope: OpaqueCodeScopeV1::Unrestricted,
            })
    );
}

#[tokio::test]
async fn sys_unrestricted_code_is_triggered_for_scripts_not_managed_resources() {
    let opaque = PermissionV1::OpaqueCode {
        scope: OpaqueCodeScopeV1::Unrestricted,
    };
    let script_snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "sys/test/shine.toml",
                b"version = 2\n[permission_defaults]\nschema_version = 2\nopaque_code = 'unrestricted'\n[[items]]\nid = 'scripted'\nlabel = 'Scripted'\ndetect = { kind = 'path', path = '$HOME/.scripted' }\ninstall = { kind = 'script', path = 'install.sh' }\n".to_vec(),
            )
            .file("sys/test/install.sh", b"#!/bin/sh\n".to_vec())
            .build();
    let script_runtime = runtime(script_snapshot);
    let script_plan = script_runtime
        .plan_sys_bootstrap(SysBootstrapPlanRequest {
            os_id: "test".to_string(),
            item_ids: vec!["scripted".to_string()],
            sys_shell: "zsh".to_string(),
            force_profile: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(script_plan.is_ready(), "{script_plan:?}");
    assert!(script_plan.permissions.required.contains(&opaque));

    script_runtime.host().put_file(
        script_runtime.context().home_dir.join(".scripted"),
        b"present".to_vec(),
    );
    let current_script_plan = script_runtime
        .plan_sys_bootstrap(SysBootstrapPlanRequest {
            os_id: "test".to_string(),
            item_ids: vec!["scripted".to_string()],
            sys_shell: "zsh".to_string(),
            force_profile: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(current_script_plan.is_ready(), "{current_script_plan:?}");
    assert!(!current_script_plan.permissions.required.contains(&opaque));

    let package_snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "sys/test/shine.toml",
                b"version = 2\n[permission_defaults]\nschema_version = 2\nopaque_code = 'unrestricted'\n[[items]]\nid = 'package'\nlabel = 'Package'\ndetect = { kind = 'path', path = '$HOME/.package' }\ninstall = { kind = 'package', provider = 'homebrew', package = 'package' }\n".to_vec(),
            )
            .build();
    let package_plan = runtime(package_snapshot)
        .plan_sys_bootstrap(SysBootstrapPlanRequest {
            os_id: "test".to_string(),
            item_ids: vec!["package".to_string()],
            sys_shell: "zsh".to_string(),
            force_profile: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(package_plan.is_ready(), "{package_plan:?}");
    assert!(!package_plan.permissions.required.contains(&opaque));

    let managed_snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "sys/test/shine.toml",
                b"version = 2\n[permission_defaults]\nschema_version = 2\nopaque_code = 'unrestricted'\n[[items]]\nid = 'managed'\nlabel = 'Managed'\nmode = 'managed'\ndriver = 'managed-file'\n[items.config]\nsource = 'managed.txt'\ntarget = '$HOME/.config/managed.txt'\n".to_vec(),
            )
            .file("sys/test/managed.txt", b"managed".to_vec())
            .build();
    let managed_plan = runtime(managed_snapshot)
        .plan_managed_sys(SysManagedPlanRequest {
            operation: LifecycleOperation::Install,
            os_id: "test".to_string(),
            target: Some("managed".to_string()),
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(managed_plan.is_ready(), "{managed_plan:?}");
    assert!(!managed_plan.permissions.required.contains(&opaque));
}

#[tokio::test]
async fn app_script_hook_is_bound_into_the_parent_plan() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"metadata_schema_version = 2\ndest = '~/.config/demo'\npost_upgrade = { script = 'refresh.ts', runtime = 'bun', env = ['TOKEN', 'OPTIONAL_TOKEN'] }\n[permissions]\nschema_version = 1\nfilesystem = [{ access = ['execute'], base = 'preset', path = 'refresh.ts' }]\nnetwork = [{ scope = 'any' }]\ncommands = ['bun']\nenvironment = [{ name = 'TOKEN', sensitivity = 'plain' }, { name = 'OPTIONAL_TOKEN', sensitivity = 'secret' }]\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"updated".to_vec())
            .file("app/demo/refresh.ts", b"export {};".to_vec())
            .build();
    let mut runtime = runtime(snapshot);
    runtime
        .context_mut_for_cli()
        .env
        .insert("TOKEN".to_string(), "secret-looking-value".to_string());
    seed_static_copy_app(&runtime, b"managed", None).await;

    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: Some("demo".to_string()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();

    assert!(plan.is_ready());
    assert!(plan.permissions.required.contains(&PermissionV1::Command {
        program: "bun".to_string(),
    }));
    assert!(
        plan.permissions
            .required
            .contains(&PermissionV1::Filesystem {
                access: FilesystemAccessV1::Execute,
                path: "preset:refresh.ts".to_string(),
            })
    );
    assert!(
        plan.author_capabilities
            .iter()
            .any(|permission| matches!(permission, PermissionV1::Network { .. }))
    );
    assert!(plan.steps.iter().any(|step| {
        step.resource.as_deref() == Some("hook:0") && step.action == PlanActionV1::Execute
    }));
    let encoded = serde_json::to_string(&plan).unwrap();
    assert!(!encoded.contains("secret-looking-value"));
    assert_eq!(plan.operation, PlanOperationV1::Upgrade);
}

#[tokio::test]
async fn legacy_overlay_metadata_blocks_recursive_artifact_hook_before_trust() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "app/demo/shine.toml",
                b"metadata_schema_version = 2\ndest = '~/.config/demo'\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"managed".to_vec())
            .overlay_file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\npost_install = { command = 'shine', args = ['app', 'artifact', 'apply', 'demo'] }\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .build();
    let runtime = runtime(snapshot);

    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Install,
            target: Some("demo".to_string()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();

    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .iter()
                .any(|code| code == "app_legacy_overlay_metadata")
    }));
    assert!(!plan.permissions.required.contains(&PermissionV1::Command {
        program: "shine".to_string(),
    }));
}

#[tokio::test]
async fn app_upgrade_ignores_permissions_from_uninstalled_presets() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"managed".to_vec())
            .file(
                "app/admin/shine.toml",
                b"dest = '/etc/admin'\n[permissions]\nschema_version = 1\nadministrator = true\n[[files]]\nsource = 'config.toml'\nrequires_admin = true\n".to_vec(),
            )
            .file("app/admin/config.toml", b"managed".to_vec())
            .file(
                "app/legacy/shine.toml",
                b"dest = '~/.config/legacy'\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/legacy/config.toml", b"managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    seed_static_copy_app(&runtime, b"managed", None).await;

    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: None,
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();

    assert!(plan.is_ready());
    assert!(
        !plan
            .permissions
            .required
            .contains(&PermissionV1::Administrator)
    );
    assert!(plan.permissions.uncomputable_codes.is_empty());
    assert!(
        plan.steps
            .iter()
            .all(|step| step.target != "app/admin" && step.target != "app/legacy")
    );
}

#[tokio::test]
async fn missing_permission_and_secret_identity_fail_closed() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file("app/demo/shine.toml", b"dest = '~/.config/demo'\n[[files]]\nsource = 'config.toml'\ngenerator = { script = 'gen.ts', runtime = 'bun', env = ['TOKEN'], when_env = 'TOKEN' }\n".to_vec())
            .file("app/demo/config.toml", b"fallback".to_vec())
            .file("app/demo/gen.ts", b"process.stdout.write('x')".to_vec())
            .build();
    let mut runtime = runtime(snapshot);
    runtime
        .context_mut_for_cli()
        .env
        .insert("TOKEN".to_string(), "plaintext".to_string());
    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Install,
            target: Some("demo".to_string()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(!plan.is_ready());
    let encoded = serde_json::to_string(&plan).unwrap();
    assert!(!encoded.contains("plaintext"));
}

#[tokio::test]
async fn secret_inputs_require_opaque_versions_and_never_serialize_values() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "app/demo/shine.toml",
            br#"dest = '~/.config/demo'
[permissions]
schema_version = 1
filesystem = [{ access = ['execute'], base = 'preset', path = 'gen.ts' }]
commands = ['bun']
environment = [{ name = 'TOKEN', sensitivity = 'secret' }]
[[files]]
source = 'config.toml'
generator = { script = 'gen.ts', runtime = 'bun', env = ['TOKEN'], when_env = 'TOKEN' }
"#
            .to_vec(),
        )
        .file("app/demo/config.toml", b"fallback".to_vec())
        .file(
            "app/demo/gen.ts",
            b"process.stdout.write('generated')".to_vec(),
        )
        .build();
    let mut runtime = runtime(snapshot);
    runtime
        .context_mut_for_cli()
        .env
        .insert("TOKEN".to_string(), "top-secret-value".to_string());
    let mut request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let missing = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(!missing.is_ready());
    assert!(
        missing
            .permissions
            .uncomputable_codes
            .contains("secret_input_identity_unavailable")
    );

    request
        .input_versions
        .insert_secret_version("TOKEN", OpaqueSecretVersion::new("vault-revision-7"));
    assert!(!format!("{:?}", request.input_versions).contains("vault-revision-7"));
    let ready = runtime.plan_apps(request).await.unwrap();
    assert!(ready.is_ready());
    let encoded = serde_json::to_string(&ready).unwrap();
    assert!(!encoded.contains("top-secret-value"));
    assert!(!encoded.contains("vault-revision-7"));
}

#[tokio::test]
async fn app_user_modification_is_preserved_unless_force_is_bound() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    runtime
        .host()
        .put_file(&destination, b"user-edited".to_vec());
    let manifest = AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination,
            backup: None,
            content_hash: crate::install::hash_content(b"desired"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    };
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&manifest).unwrap().into_bytes(),
    );
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let preserved = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(preserved.steps.iter().any(|step| {
        step.action == PlanActionV1::Preserve
            && step
                .diagnostic_codes
                .contains(&"app_user_modified".to_string())
    }));
    let forced = runtime
        .plan_apps(AppPlanRequest {
            force: true,
            ..request
        })
        .await
        .unwrap();
    assert!(forced.steps.iter().any(|step| {
        step.action == PlanActionV1::Update
            && step
                .diagnostic_codes
                .contains(&"app_user_modification_override".to_string())
    }));
    assert_ne!(
        preserved.fingerprint().unwrap(),
        forced.fingerprint().unwrap()
    );
}

#[tokio::test]
async fn app_upgrade_uses_the_latest_legacy_receipt_after_a_relocation() {
    let runtime = runtime(static_copy_app_snapshot());
    let source = "app/demo/config.toml".to_string();
    let old_destination = runtime.context().home_dir.join(".legacy/demo/config.toml");
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    runtime.host().put_file(&destination, b"managed".to_vec());
    let legacy_manifest = AppManifest {
        schema_version: 0,
        entries: vec![
            AppEntry {
                source: source.clone(),
                destination: old_destination,
                backup: None,
                content_hash: crate::install::hash_content(b"managed"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            },
            AppEntry {
                source,
                destination,
                backup: None,
                content_hash: crate::install::hash_content(b"managed"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            },
        ],
    };
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&legacy_manifest).unwrap().into_bytes(),
    );

    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: Some("demo".to_string()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();

    assert!(plan.is_ready());
    assert!(!plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_destination_occupied".to_string())
    }));
}

#[tokio::test]
async fn shell_and_sys_plans_use_only_observation_operations() {
    let platform = RuntimePlatform::current();
    let os_id = match platform {
        RuntimePlatform::Macos => "macos",
        RuntimePlatform::Linux => "ubuntu",
        RuntimePlatform::Windows => "windows",
    };
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("shell/demo/shine.toml", b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\nplatforms = ['unix']\n[files.permissions]\nschema_version = 1\n".to_vec())
            .file("shell/demo/demo.sh", b"#!/bin/sh\n".to_vec())
            .file(format!("sys/{os_id}/shine.toml"), b"version = 2\n[[items]]\nid = 'managed'\nlabel = 'Managed'\nmode = 'managed'\ndriver = 'managed-file'\npermissions = { schema_version = 1 }\n[items.config]\nsource = 'managed.txt'\ntarget = '$HOME/.config/managed.txt'\n".to_vec())
            .file(format!("sys/{os_id}/managed.txt"), b"managed".to_vec())
            .build();
    let (runtime, host) = observation_runtime(snapshot);
    if platform.is_unix() {
        let shell = runtime
            .plan_shells(ShellPlanRequest {
                operation: LifecycleOperation::Install,
                target: Some("demo/demo".to_string()),
                force: false,
                purge: false,
                input_versions: PlanningInputVersions::default(),
            })
            .await
            .unwrap();
        assert!(shell.is_ready());
    }
    let sys = runtime
        .plan_managed_sys(SysManagedPlanRequest {
            operation: LifecycleOperation::Install,
            os_id: os_id.to_string(),
            target: Some("managed".to_string()),
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(sys.is_ready());
    assert!(!host.operations().iter().any(|operation| matches!(
        operation,
        super::super::HostOperation::Write(_)
            | super::super::HostOperation::Remove(_)
            | super::super::HostOperation::Run { .. }
            | super::super::HostOperation::ApplySplitDns { .. }
            | super::super::HostOperation::RemoveSplitDns { .. }
    )));
}

#[tokio::test]
async fn app_plan_fingerprint_binds_manifest_and_live_state() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let initial = runtime.plan_apps(request.clone()).await.unwrap();
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    runtime.host().put_file(&destination, b"foreign".to_vec());
    let changed = runtime.plan_apps(request).await.unwrap();
    assert_ne!(initial.inputs.state, changed.inputs.state);
    assert_ne!(
        initial.fingerprint().unwrap(),
        changed.fingerprint().unwrap()
    );
}

#[tokio::test]
async fn approved_app_install_rejects_changed_state_before_mutation() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().put_file(
        runtime.context().home_dir.join(".config/demo/config.toml"),
        b"foreign".to_vec(),
    );

    let mut observer = super::super::NullObserver;
    let error = runtime
        .install_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Plan"));
    assert!(
        !runtime.host().operations().iter().any(|operation| matches!(
            operation,
            super::super::HostOperation::Write(_)
                | super::super::HostOperation::Remove(_)
                | super::super::HostOperation::Run { .. }
        ))
    );
}

#[tokio::test]
async fn approved_app_install_journals_create_and_commits_only_after_receipt_write() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file(
                "app/demo/config.toml",
                b"secret-managed-bytes".to_vec(),
            )
            .build();
    let runtime = runtime(snapshot);
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert_eq!(actions.len(), 1);
    let encoded = toml::to_string(&actions[0]).unwrap();
    assert!(encoded.contains(&crate::install::hash_content(b"secret-managed-bytes").to_string()));
    assert!(!encoded.contains("secret-managed-bytes"));

    let mut observer = super::super::NullObserver;
    runtime
        .install_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();

    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let journal = runtime
        .context()
        .shine_dir
        .join(super::super::APP_OPERATION_JOURNAL_FILE);
    let manifest_path = runtime.context().shine_dir.join("app-manifest.toml");
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(
        manifest
            .find_by_source("app/demo/config.toml")
            .map(|entry| entry.content_hash),
        Some(crate::install::hash_content(b"secret-managed-bytes"))
    );
    assert!(runtime.host().read(&journal).await.is_err());

    let operations = runtime.host().operations();
    let journal_writes = operations
        .iter()
        .enumerate()
        .filter_map(|(index, operation)| {
            matches!(operation, super::super::HostOperation::Write(path) if path == &journal)
                .then_some(index)
        })
        .collect::<Vec<_>>();
    let destination_write = operations
            .iter()
            .position(|operation| matches!(operation, super::super::HostOperation::Write(path) if path == &destination))
            .unwrap();
    let receipt_write = operations
            .iter()
            .position(|operation| matches!(operation, super::super::HostOperation::Write(path) if path == &manifest_path))
            .unwrap();
    let journal_commit = operations
            .iter()
            .position(|operation| matches!(operation, super::super::HostOperation::Remove(path) if path == &journal))
            .unwrap();
    assert_eq!(journal_writes.len(), 2);
    assert!(journal_writes[0] < destination_write);
    assert!(destination_write < journal_writes[1]);
    assert!(journal_writes[1] < receipt_write);
    assert!(receipt_write < journal_commit);
}

#[tokio::test]
async fn approved_json_merge_install_and_uninstall_use_key_owned_actions() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'settings.json'\ninstall_mode = 'json-merge'\nmanaged_keys = ['proxy', 'containersProxy']\n".to_vec(),
            )
            .file(
                "app/demo/settings.json",
                br#"{"proxy":"managed","containersProxy":"managed"}"#.to_vec(),
            )
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime
        .context()
        .home_dir
        .join(".config/demo/settings.json");
    let rollback = crate::action::managed_file_rollback_path(&destination);
    runtime
        .host()
        .put_file(&destination, br#"{"theme":"dark"}"#.to_vec());
    let install_request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let install_plan = runtime.plan_apps(install_request.clone()).await.unwrap();
    let install_approval = PlanApprovalV1::for_reviewed_plan(&install_plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&install_request, &install_plan, &install_approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::MergeManagedJson { .. }))
    ));
    let mut observer = super::super::NullObserver;
    runtime
        .install_apps_approved(
            install_request,
            &install_approval,
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();
    let installed = runtime.host().read(&destination).await.unwrap();
    let installed: serde_json::Value = serde_json::from_slice(&installed).unwrap();
    assert_eq!(installed["theme"], "dark");
    assert_eq!(installed["proxy"], "managed");
    assert!(runtime.host().read(&rollback).await.is_err());

    let uninstall_request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let uninstall_plan = runtime.plan_apps(uninstall_request.clone()).await.unwrap();
    assert!(
        uninstall_plan
            .permissions
            .required
            .contains(&PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: review_path(runtime.context(), &destination),
            })
    );
    let uninstall_approval = PlanApprovalV1::for_reviewed_plan(&uninstall_plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&uninstall_request, &uninstall_plan, &uninstall_approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::RemoveManagedJson { .. }))
    ));
    runtime
        .uninstall_apps_approved(
            uninstall_request,
            &uninstall_approval,
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();
    let remaining = runtime.host().read(&destination).await.unwrap();
    let remaining: serde_json::Value = serde_json::from_slice(&remaining).unwrap();
    assert_eq!(remaining, serde_json::json!({"theme": "dark"}));
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn approved_app_upgrade_journals_key_owned_json_relocation() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo-next'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'settings.json'\ninstall_mode = 'json-merge'\nmanaged_keys = ['containersProxy']\n".to_vec(),
            )
            .file(
                "app/demo/settings.json",
                br#"{"containersProxy":"next"}"#.to_vec(),
            )
            .build();
    let runtime = runtime(snapshot);
    let previous = runtime
        .context()
        .home_dir
        .join(".config/demo-old/settings.json");
    let desired = runtime
        .context()
        .home_dir
        .join(".config/demo-next/settings.json");
    let rollback = crate::action::managed_file_rollback_path(&previous);
    let previous_source = br#"{"proxy":"previous"}"#;
    runtime.host().put_file(
        &previous,
        br#"{"proxy":"previous","theme":"dark"}"#.to_vec(),
    );
    let managed_keys = vec!["proxy".to_string()];
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/settings.json".to_string(),
            destination: previous.clone(),
            backup: None,
            content_hash: crate::runtime::app::managed_json_hash(previous_source, &managed_keys)
                .unwrap(),
            install_strategy: crate::install::AppInstallStrategy::JsonMerge {
                managed_keys: managed_keys.clone(),
            },
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Update
            && step
                .diagnostic_codes
                .contains(&"app_destination_relocated".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::RelocateManagedJson { .. }))
    ));
    for (access, path) in [
        (FilesystemAccessV1::Write, previous.clone()),
        (FilesystemAccessV1::Remove, previous.clone()),
        (FilesystemAccessV1::Write, desired.clone()),
        (FilesystemAccessV1::Write, rollback.clone()),
        (FilesystemAccessV1::Remove, rollback.clone()),
    ] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: review_path(runtime.context(), &path),
                })
        );
    }

    let mut observer = super::super::NullObserver;
    runtime
        .upgrade_apps_approved(
            request,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();
    let previous_json: serde_json::Value =
        serde_json::from_slice(&runtime.host().read(&previous).await.unwrap()).unwrap();
    let desired_json: serde_json::Value =
        serde_json::from_slice(&runtime.host().read(&desired).await.unwrap()).unwrap();
    assert_eq!(previous_json, serde_json::json!({"theme": "dark"}));
    assert_eq!(desired_json, serde_json::json!({"containersProxy": "next"}));
    assert!(runtime.host().read(&rollback).await.is_err());
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    let entry = manifest.find_by_source("app/demo/settings.json").unwrap();
    assert_eq!(entry.destination, desired);
    assert_eq!(
        entry.content_hash,
        crate::runtime::app::managed_json_hash(
            br#"{"containersProxy":"next"}"#,
            &["containersProxy".to_string()],
        )
        .unwrap()
    );
}

#[tokio::test]
async fn missing_json_relocation_source_recovers_to_the_previous_receipt() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo-next'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'settings.json'\ninstall_mode = 'json-merge'\nmanaged_keys = ['proxy']\n".to_vec(),
            )
            .file("app/demo/settings.json", br#"{"proxy":"next"}"#.to_vec())
            .build();
    let runtime = runtime(snapshot);
    let previous = runtime
        .context()
        .home_dir
        .join(".config/demo-old/settings.json");
    let desired = runtime
        .context()
        .home_dir
        .join(".config/demo-next/settings.json");
    let rollback = crate::action::managed_file_rollback_path(&previous);
    let managed_keys = vec!["proxy".to_string()];
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/settings.json".to_string(),
            destination: previous.clone(),
            backup: None,
            content_hash: crate::runtime::app::managed_json_hash(
                br#"{"proxy":"previous"}"#,
                &managed_keys,
            )
            .unwrap(),
            install_strategy: crate::install::AppInstallStrategy::JsonMerge { managed_keys },
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(
            ir.actions.as_slice(),
            [action] if matches!(
                action.kind,
                crate::action::ActionKindV1::RelocateManagedJson {
                    previous_present: false,
                    previous_original_hash: None,
                    ..
                }
            )
        )
    ));

    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("app-manifest.toml"), 0);
    let mut observer = super::super::NullObserver;
    assert!(
        runtime
            .upgrade_apps_approved(
                request,
                &approval,
                AppApprovedUpgradeOptions::default(),
                &mut observer,
                &mut Interaction,
            )
            .await
            .is_err()
    );
    assert!(runtime.host().read(&previous).await.is_err());
    let desired_json: serde_json::Value =
        serde_json::from_slice(&runtime.host().read(&desired).await.unwrap()).unwrap();
    assert_eq!(desired_json, serde_json::json!({"proxy": "next"}));
    assert!(runtime.host().read(&rollback).await.is_err());

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.is_ready());
    assert!(recovery_plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"app_recovery_restore_json_relocation".to_string())
    }));
    frontend_recover(
        &runtime,
        crate::frontend::ReviewRequest::AppRecovery,
        &recovery_plan,
    )
    .await;
    assert!(runtime.host().read(&previous).await.is_err());
    assert!(runtime.host().read(&desired).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(
        manifest
            .find_by_source("app/demo/settings.json")
            .unwrap()
            .destination,
        previous
    );
}

#[tokio::test]
async fn json_relocation_receipt_failure_restores_only_owned_keys_on_both_sides() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo-next'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'settings.json'\ninstall_mode = 'json-merge'\nmanaged_keys = ['proxy']\n".to_vec(),
            )
            .file("app/demo/settings.json", br#"{"proxy":"next"}"#.to_vec())
            .build();
    let runtime = runtime(snapshot);
    let previous = runtime
        .context()
        .home_dir
        .join(".config/demo-old/settings.json");
    let desired = runtime
        .context()
        .home_dir
        .join(".config/demo-next/settings.json");
    let rollback = crate::action::managed_file_rollback_path(&previous);
    let managed_keys = vec!["proxy".to_string()];
    runtime.host().put_file(
        &previous,
        br#"{"proxy":"previous","theme":"light"}"#.to_vec(),
    );
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/settings.json".to_string(),
            destination: previous.clone(),
            backup: None,
            content_hash: crate::runtime::app::managed_json_hash(
                br#"{"proxy":"previous"}"#,
                &managed_keys,
            )
            .unwrap(),
            install_strategy: crate::install::AppInstallStrategy::JsonMerge {
                managed_keys: managed_keys.clone(),
            },
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("app-manifest.toml"), 0);
    let mut observer = super::super::NullObserver;
    assert!(
        runtime
            .upgrade_apps_approved(
                request,
                &approval,
                AppApprovedUpgradeOptions::default(),
                &mut observer,
                &mut Interaction,
            )
            .await
            .is_err()
    );
    runtime
        .host()
        .put_file(&previous, br#"{"theme":"dark","zoom":2}"#.to_vec());
    runtime
        .host()
        .put_file(&desired, br#"{"proxy":"next","font":"large"}"#.to_vec());

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.is_ready());
    assert!(recovery_plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"app_recovery_restore_json_relocation".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    let previous_json: serde_json::Value =
        serde_json::from_slice(&runtime.host().read(&previous).await.unwrap()).unwrap();
    let desired_json: serde_json::Value =
        serde_json::from_slice(&runtime.host().read(&desired).await.unwrap()).unwrap();
    assert_eq!(
        previous_json,
        serde_json::json!({"proxy": "previous", "theme": "dark", "zoom": 2})
    );
    assert_eq!(desired_json, serde_json::json!({"font": "large"}));
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn committed_json_relocation_cleanup_preserves_user_owned_values_on_both_sides() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo-next'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'settings.json'\ninstall_mode = 'json-merge'\nmanaged_keys = ['proxy']\n".to_vec(),
            )
            .file("app/demo/settings.json", br#"{"proxy":"next"}"#.to_vec())
            .build();
    let runtime = runtime(snapshot);
    let previous = runtime
        .context()
        .home_dir
        .join(".config/demo-old/settings.json");
    let desired = runtime
        .context()
        .home_dir
        .join(".config/demo-next/settings.json");
    let rollback = crate::action::managed_file_rollback_path(&previous);
    let managed_keys = vec!["proxy".to_string()];
    runtime.host().put_file(
        &previous,
        br#"{"proxy":"previous","theme":"light"}"#.to_vec(),
    );
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/settings.json".to_string(),
            destination: previous.clone(),
            backup: None,
            content_hash: crate::runtime::app::managed_json_hash(
                br#"{"proxy":"previous"}"#,
                &managed_keys,
            )
            .unwrap(),
            install_strategy: crate::install::AppInstallStrategy::JsonMerge { managed_keys },
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().fail_remove_after(&rollback, 0);
    let mut observer = super::super::NullObserver;
    assert!(
        runtime
            .upgrade_apps_approved(
                request,
                &approval,
                AppApprovedUpgradeOptions::default(),
                &mut observer,
                &mut Interaction,
            )
            .await
            .is_err()
    );
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(
        manifest
            .find_by_source("app/demo/settings.json")
            .unwrap()
            .destination,
        desired
    );
    runtime.host().put_file(
        &previous,
        br#"{"proxy":"user-owned","theme":"dark"}"#.to_vec(),
    );
    runtime
        .host()
        .put_file(&desired, br#"{"proxy":"next","font":"large"}"#.to_vec());

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.is_ready());
    assert!(recovery_plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"app_recovery_remove_committed_json_relocation_rollback".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&previous).await.unwrap(),
        br#"{"proxy":"user-owned","theme":"dark"}"#
    );
    assert_eq!(
        runtime.host().read(&desired).await.unwrap(),
        br#"{"proxy":"next","font":"large"}"#
    );
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn approved_privileged_app_install_journals_static_copy_creation() {
    let runtime = runtime(privileged_static_copy_app_snapshot());
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(
        plan.permissions
            .required
            .contains(&PermissionV1::Administrator)
    );
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(
            ir.actions.as_slice(),
            [action] if matches!(
                action.kind,
                crate::action::ActionKindV1::CreateManagedFile {
                    requires_admin: true,
                    ..
                }
            )
        )
    ));

    let mut observer = super::super::NullObserver;
    runtime
        .install_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();
    let destination = PathBuf::from("/etc/demo/config.toml");
    assert_eq!(runtime.host().read(&destination).await.unwrap(), b"managed");
    assert!(
        runtime
            .host()
            .operations()
            .contains(&HostOperation::WritePrivileged(destination))
    );
}

#[tokio::test]
async fn approved_privileged_app_install_journals_backup_aware_creation() {
    let runtime = runtime(privileged_static_copy_app_snapshot());
    let destination = PathBuf::from("/etc/demo/config.toml");
    let backup = crate::install::backup_path(&destination);
    runtime
        .host()
        .put_file(&destination, b"user-original".to_vec());
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(
            ir.actions.as_slice(),
            [action] if matches!(
                action.kind,
                crate::action::ActionKindV1::CreateManagedFileWithBackup {
                    requires_admin: true,
                    ..
                }
            )
        )
    ));

    let mut observer = super::super::NullObserver;
    runtime
        .install_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), b"managed");
    assert_eq!(
        runtime.host().read(&backup).await.unwrap(),
        b"user-original"
    );
    assert!(
        runtime
            .host()
            .operations()
            .contains(&HostOperation::MovePrivileged {
                from: destination,
                to: backup,
            })
    );
}

#[tokio::test]
async fn app_upgrade_permissions_exclude_current_files_and_bind_their_state() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("app/demo/shine.toml", b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\nenvironment = [{ name = 'SOURCE', sensitivity = 'plain' }]\n[[files]]\nsource = 'changed'\n[[files]]\nsource = 'current'\n[[files]]\nsource = 'manual'\ngenerator = { script = 'gen.ts', runtime = 'bun', env = ['SOURCE'], when_env = 'SOURCE', auto = false }\n".to_vec())
            .file("app/demo/changed", b"next".to_vec())
            .file("app/demo/current", b"current".to_vec())
            .file("app/demo/manual", b"fallback".to_vec())
            .file("app/demo/gen.ts", b"process.stdout.write('generated')".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let root = runtime.context().home_dir.join(".config/demo");
    let entries = [
        ("changed", b"previous".as_slice()),
        ("current", b"current".as_slice()),
        ("manual", b"generated".as_slice()),
    ]
    .into_iter()
    .map(|(name, bytes)| {
        let destination = root.join(name);
        runtime.host().put_file(&destination, bytes.to_vec());
        AppEntry {
            source: format!("app/demo/{name}"),
            destination,
            backup: None,
            content_hash: crate::install::hash_content(bytes),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }
    })
    .collect();
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries,
        })
        .unwrap()
        .into_bytes(),
    );
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: None,
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let current = review_path(runtime.context(), &root.join("current"));
    assert!(
        !plan
            .permissions
            .required
            .iter()
            .any(|p| matches!(p, PermissionV1::Filesystem { path, .. } if path == &current))
    );
    assert!(
        plan.permissions
            .required
            .contains(&PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: review_path(runtime.context(), &root.join("changed"))
            })
    );
    let manual = review_path(runtime.context(), &root.join("manual"));
    assert!(!plan.permissions.required.iter().any(|p| matches!(p,
            PermissionV1::Filesystem { path, .. } if path == &manual)));
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .iter()
            .any(|code| code == "app_manual_refresh_required")
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .put_file(root.join("current"), b"user edit".to_vec());
    let changed_state = runtime.plan_apps(request.clone()).await.unwrap();
    assert_ne!(
        plan.fingerprint().unwrap(),
        changed_state.fingerprint().unwrap()
    );
    assert!(
        runtime
            .upgrade_apps_approved(
                request.clone(),
                &approval,
                AppApprovedUpgradeOptions::default(),
                &mut super::super::NullObserver,
                &mut Interaction
            )
            .await
            .is_err()
    );
    runtime
        .host()
        .put_file(root.join("current"), b"current".to_vec());
    runtime
        .upgrade_apps_approved(
            request.clone(),
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut super::super::NullObserver,
            &mut Interaction,
        )
        .await
        .unwrap();
    let current_plan = runtime.plan_apps(request).await.unwrap();
    assert!(current_plan.permissions.required.is_empty());
}

#[tokio::test]
async fn approved_app_upgrade_journals_static_in_place_managed_update() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let rollback = crate::action::managed_file_rollback_path(&destination);
    runtime
        .host()
        .put_file(&destination, b"previous-managed".to_vec());
    runtime
        .host()
        .set_mode(&destination, 0o100600)
        .await
        .unwrap();
    let manifest = AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination: destination.clone(),
            backup: None,
            content_hash: crate::install::hash_content(b"previous-managed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    };
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&manifest).unwrap().into_bytes(),
    );
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::UpdateManagedFile { .. }))
    ));
    for access in [FilesystemAccessV1::Write, FilesystemAccessV1::Remove] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: review_path(runtime.context(), &rollback),
                })
        );
    }

    let mut observer = super::super::NullObserver;
    let report = runtime
        .upgrade_apps_approved(
            request,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();
    assert_eq!(report.updated_categories, vec!["demo"]);
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"next-managed"
    );
    assert_eq!(
        runtime
            .host()
            .metadata(&destination)
            .await
            .unwrap()
            .unix_mode,
        Some(0o100600)
    );
    assert!(runtime.host().read(&rollback).await.is_err());
    assert!(
        runtime
            .host()
            .read(
                &runtime
                    .context()
                    .shine_dir
                    .join(super::super::APP_OPERATION_JOURNAL_FILE)
            )
            .await
            .is_err()
    );
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(
        manifest
            .find_by_source("app/demo/config.toml")
            .map(|entry| entry.content_hash),
        Some(crate::install::hash_content(b"next-managed"))
    );
}

#[tokio::test]
async fn approved_app_upgrade_converges_legacy_duplicate_relocation_receipts() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.shine/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let previous_destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let destination = runtime.context().shine_dir.join("demo/config.toml");
    for path in [&previous_destination, &destination] {
        runtime.host().put_file(path, b"previous-managed".to_vec());
    }
    let entry = |destination: PathBuf| AppEntry {
        source: "app/demo/config.toml".to_string(),
        destination,
        backup: None,
        content_hash: crate::install::hash_content(b"previous-managed"),
        install_strategy: crate::install::AppInstallStrategy::Copy,
        uses_env: false,
        requires_admin: false,
    };
    let manifest = AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![
            entry(previous_destination.clone()),
            entry(destination.clone()),
        ],
    };
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&manifest).unwrap().into_bytes(),
    );
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();

    let mut observer = super::super::NullObserver;
    let report = runtime
        .upgrade_apps_approved(
            request,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();

    assert_eq!(report.updated_categories, vec!["demo"]);
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"next-managed"
    );
    assert_eq!(
        runtime.host().read(&previous_destination).await.unwrap(),
        b"previous-managed"
    );
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    let matching = manifest
        .entries
        .iter()
        .filter(|entry| entry.source == "app/demo/config.toml")
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].destination, destination);
    assert_eq!(
        matching[0].content_hash,
        crate::install::hash_content(b"next-managed")
    );
}

#[tokio::test]
async fn approved_app_upgrade_journals_static_copy_relocation() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo-next'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let previous = runtime
        .context()
        .home_dir
        .join(".config/demo-old/config.toml");
    let desired = runtime
        .context()
        .home_dir
        .join(".config/demo-next/config.toml");
    let rollback = crate::action::managed_file_rollback_path(&previous);
    runtime
        .host()
        .put_file(&previous, b"previous-managed".to_vec());
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination: previous.clone(),
            backup: None,
            content_hash: crate::install::hash_content(b"previous-managed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Update
            && step
                .diagnostic_codes
                .contains(&"app_destination_relocated".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::RelocateManagedFile { .. }))
    ));
    for (access, path) in [
        (FilesystemAccessV1::Remove, previous.clone()),
        (FilesystemAccessV1::Write, desired.clone()),
        (FilesystemAccessV1::Write, rollback.clone()),
        (FilesystemAccessV1::Remove, rollback.clone()),
    ] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: review_path(runtime.context(), &path),
                })
        );
    }

    let mut observer = super::super::NullObserver;
    runtime
        .upgrade_apps_approved(
            request,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();
    assert!(runtime.host().read(&previous).await.is_err());
    assert_eq!(
        runtime.host().read(&desired).await.unwrap(),
        b"next-managed"
    );
    assert!(runtime.host().read(&rollback).await.is_err());
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    let entry = manifest.find_by_source("app/demo/config.toml").unwrap();
    assert_eq!(entry.destination, desired);
    assert!(entry.backup.is_none());
}

#[tokio::test]
async fn app_relocation_uses_previous_receipt_administrator_identity() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo-next'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let previous = PathBuf::from("/etc/demo-old/config.toml");
    let desired = runtime
        .context()
        .home_dir
        .join(".config/demo-next/config.toml");
    let rollback = crate::action::managed_file_rollback_path(&previous);
    runtime
        .host()
        .put_file(&previous, b"previous-managed".to_vec());
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination: previous.clone(),
            backup: None,
            content_hash: crate::install::hash_content(b"previous-managed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: true,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(
        plan.permissions
            .required
            .contains(&PermissionV1::Administrator)
    );
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let mut observer = super::super::NullObserver;
    runtime
        .upgrade_apps_approved(
            request,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();
    assert!(runtime.host().read(&previous).await.is_err());
    assert_eq!(
        runtime.host().read(&desired).await.unwrap(),
        b"next-managed"
    );
    assert!(runtime.host().read(&rollback).await.is_err());
    assert!(
        runtime
            .host()
            .operations()
            .contains(&HostOperation::MovePrivileged {
                from: previous,
                to: rollback,
            })
    );
}

async fn generated_relocation_fixture(
    requires_admin: bool,
) -> (
    CoreRuntime<InMemoryHost>,
    AppPlanRequest,
    PathBuf,
    PathBuf,
    PathBuf,
) {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "app/demo/shine.toml",
            br#"
dest = '~/.config/demo-next'
[permissions]
schema_version = 1
environment = [{ name = 'ENABLE', sensitivity = 'plain' }]
[[files]]
source = 'config.toml'
generator = { script = 'gen.sh', env = ['ENABLE'], when_env = 'ENABLE', auto = true }
"#
            .to_vec(),
        )
        .file("app/demo/config.toml", b"fallback".to_vec())
        .file("app/demo/gen.sh", b"#!/bin/sh\nprintf generated".to_vec())
        .build();
    let mut runtime = runtime(snapshot);
    runtime
        .context_mut_for_cli()
        .env
        .insert("ENABLE".into(), "1".into());
    let previous = runtime
        .context()
        .home_dir
        .join(".config/demo-old/config.toml");
    let desired = runtime
        .context()
        .home_dir
        .join(".config/demo-next/config.toml");
    let backup = crate::install::backup_path(&previous);
    runtime.host().put_file(&previous, b"managed".to_vec());
    runtime.host().put_file(&backup, b"user-original".to_vec());
    AppManifest {
        entries: vec![AppEntry {
            source: "app/demo/config.toml".into(),
            destination: previous.clone(),
            backup: Some(backup.clone()),
            content_hash: crate::install::hash_content(b"managed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin,
        }],
        ..Default::default()
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".into()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    (runtime, request, previous, desired, backup)
}

#[tokio::test]
async fn generated_relocation_binds_backup_changes_before_any_execution() {
    for change_mode_only in [false, true] {
        let (runtime, request, previous, desired, backup) =
            generated_relocation_fixture(false).await;
        let plan = runtime.plan_apps(request.clone()).await.unwrap();
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        for (access, path) in [
            (FilesystemAccessV1::Write, &desired),
            (FilesystemAccessV1::Remove, &desired),
            (FilesystemAccessV1::Remove, &previous),
            (FilesystemAccessV1::Write, &previous),
            (FilesystemAccessV1::Remove, &backup),
        ] {
            assert!(
                plan.permissions
                    .required
                    .contains(&PermissionV1::Filesystem {
                        access,
                        path: review_path(runtime.context(), path),
                    })
            );
        }
        assert!(
            plan.steps
                .iter()
                .any(|step| step.action == PlanActionV1::Remove
                    && step.resource.as_deref() == Some("relocation-source:config.toml"))
        );
        assert!(plan.steps.iter().any(|step| {
            step.diagnostic_codes
                .iter()
                .any(|code| code == "app_generated_relocation_backup_restored")
        }));
        let operations = runtime.host().operations().len();
        if change_mode_only {
            runtime
                .host()
                .put_file_with_mode(&backup, b"user-original".to_vec(), 0o600);
        } else {
            runtime.host().put_file(&backup, b"changed-backup".to_vec());
        }
        let changed = runtime.plan_apps(request.clone()).await.unwrap();
        assert_ne!(plan.fingerprint().unwrap(), changed.fingerprint().unwrap());
        assert!(
            runtime
                .upgrade_apps_approved(
                    request,
                    &approval,
                    AppApprovedUpgradeOptions::default(),
                    &mut super::super::NullObserver,
                    &mut Interaction
                )
                .await
                .is_err()
        );
        assert!(
            runtime.host().operations()[operations..]
                .iter()
                .all(|operation| matches!(operation, HostOperation::Read(_)))
        );
        assert_eq!(runtime.host().read(&previous).await.unwrap(), b"managed");
        assert!(runtime.host().read(&desired).await.is_err());
    }
}

#[tokio::test]
async fn generated_relocation_restores_backup_and_checks_old_admin_identity() {
    struct AdminReview(bool);
    impl RuntimeInteraction for AdminReview {
        fn confirm(&mut self, _: &'static str, default: bool) -> Result<bool> {
            Ok(default)
        }
        fn authorize_admin<'a>(
            &'a mut self,
            count: usize,
        ) -> Pin<Box<dyn Future<Output = Result<bool>> + Send + 'a>> {
            assert_eq!(count, 1);
            self.0 = true;
            Box::pin(async { Ok(true) })
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
    for requires_admin in [false, true] {
        let (runtime, request, previous, desired, backup) =
            generated_relocation_fixture(requires_admin).await;
        let plan = runtime.plan_apps(request.clone()).await.unwrap();
        assert_eq!(
            plan.permissions
                .required
                .contains(&PermissionV1::Administrator),
            requires_admin
        );
        let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
        runtime
            .host()
            .queue_process_output(Ok(super::super::ProcessOutput {
                exit_code: Some(0),
                stdout: b"generated".to_vec(),
                stderr: Vec::new(),
            }));
        let mut interaction = AdminReview(false);
        let report = runtime
            .upgrade_apps_approved(
                request,
                &approval,
                AppApprovedUpgradeOptions::default(),
                &mut super::super::NullObserver,
                &mut interaction,
            )
            .await
            .unwrap();
        assert_eq!(interaction.0, requires_admin);
        assert_eq!(report.failed, 0);
        assert_eq!(runtime.host().read(&desired).await.unwrap(), b"generated");
        assert_eq!(
            runtime.host().read(&previous).await.unwrap(),
            b"user-original"
        );
        assert!(runtime.host().read(&backup).await.is_err());
        let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap();
        let entry = manifest.find_by_source("app/demo/config.toml").unwrap();
        assert_eq!(entry.destination, desired);
        assert!(entry.backup.is_none());
    }
}

#[tokio::test]
async fn generated_relocation_preserves_incomplete_backup_state() {
    for missing_source in [false, true] {
        let (runtime, request, previous, _, backup) = generated_relocation_fixture(false).await;
        runtime
            .host()
            .remove_file(if missing_source { &previous } else { &backup })
            .await
            .unwrap();
        let plan = runtime.plan_apps(request).await.unwrap();
        assert!(!plan.is_ready());
        let expected = if missing_source {
            "app_relocation_backup_source_missing"
        } else {
            "app_relocation_backup_unsupported"
        };
        assert!(
            plan.steps
                .iter()
                .any(|step| step.action == PlanActionV1::Blocked
                    && step.diagnostic_codes.iter().any(|code| code == expected))
        );
        assert!(
            !runtime
                .host()
                .operations()
                .iter()
                .any(|op| matches!(op, HostOperation::Run { .. }))
        );
    }
}

#[tokio::test]
async fn app_relocation_rejects_destination_created_after_review() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo-next'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let previous = runtime
        .context()
        .home_dir
        .join(".config/demo-old/config.toml");
    let desired = runtime
        .context()
        .home_dir
        .join(".config/demo-next/config.toml");
    runtime
        .host()
        .put_file(&previous, b"previous-managed".to_vec());
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination: previous.clone(),
            backup: None,
            content_hash: crate::install::hash_content(b"previous-managed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .put_file(&desired, b"late-user-file".to_vec());

    let mut observer = super::super::NullObserver;
    let error = runtime
        .upgrade_apps_approved(
            request,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Plan"));
    assert_eq!(
        runtime.host().read(&previous).await.unwrap(),
        b"previous-managed"
    );
    assert_eq!(
        runtime.host().read(&desired).await.unwrap(),
        b"late-user-file"
    );
    assert!(
        runtime
            .host()
            .read(
                &runtime
                    .context()
                    .shine_dir
                    .join(super::super::APP_OPERATION_JOURNAL_FILE)
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn app_relocation_recovery_after_receipt_commit_cleans_only_rollback() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo-next'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let previous = runtime
        .context()
        .home_dir
        .join(".config/demo-old/config.toml");
    let desired = runtime
        .context()
        .home_dir
        .join(".config/demo-next/config.toml");
    let rollback = crate::action::managed_file_rollback_path(&previous);
    runtime
        .host()
        .put_file(&previous, b"previous-managed".to_vec());
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination: previous.clone(),
            backup: None,
            content_hash: crate::install::hash_content(b"previous-managed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().fail_remove_after(&rollback, 0);

    let mut observer = super::super::NullObserver;
    assert!(
        runtime
            .upgrade_apps_approved(
                request,
                &approval,
                AppApprovedUpgradeOptions::default(),
                &mut observer,
                &mut Interaction,
            )
            .await
            .is_err()
    );
    assert!(runtime.host().read(&previous).await.is_err());
    assert_eq!(
        runtime.host().read(&desired).await.unwrap(),
        b"next-managed"
    );
    assert_eq!(
        runtime.host().read(&rollback).await.unwrap(),
        b"previous-managed"
    );
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(
        manifest
            .find_by_source("app/demo/config.toml")
            .unwrap()
            .destination,
        desired
    );

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::App, true).await;
    assert!(recovery_plan.is_ready());
    assert!(recovery_plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"app_recovery_remove_committed_relocation_rollback".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert!(runtime.host().read(&previous).await.is_err());
    assert_eq!(
        runtime.host().read(&desired).await.unwrap(),
        b"next-managed"
    );
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn app_relocation_receipt_failure_recovers_previous_file_and_backup() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo-next'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let previous = runtime
        .context()
        .home_dir
        .join(".config/demo-old/config.toml");
    let desired = runtime
        .context()
        .home_dir
        .join(".config/demo-next/config.toml");
    let backup = crate::install::backup_path(&previous);
    let rollback = crate::action::managed_file_rollback_path(&previous);
    runtime
        .host()
        .put_file(&previous, b"previous-managed".to_vec());
    runtime.host().put_file(&backup, b"user-original".to_vec());
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination: previous.clone(),
            backup: Some(backup.clone()),
            content_hash: crate::install::hash_content(b"previous-managed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("app-manifest.toml"), 0);

    let mut observer = super::super::NullObserver;
    assert!(
        runtime
            .upgrade_apps_approved(
                request,
                &approval,
                AppApprovedUpgradeOptions::default(),
                &mut observer,
                &mut Interaction,
            )
            .await
            .is_err()
    );
    assert_eq!(
        runtime.host().read(&previous).await.unwrap(),
        b"user-original"
    );
    assert!(runtime.host().read(&backup).await.is_err());
    assert_eq!(
        runtime.host().read(&rollback).await.unwrap(),
        b"previous-managed"
    );
    assert_eq!(
        runtime.host().read(&desired).await.unwrap(),
        b"next-managed"
    );

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.is_ready());
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&previous).await.unwrap(),
        b"previous-managed"
    );
    assert_eq!(
        runtime.host().read(&backup).await.unwrap(),
        b"user-original"
    );
    assert!(runtime.host().read(&desired).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn app_relocation_recovers_created_destination_when_previous_file_was_missing() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo-next'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let previous = runtime
        .context()
        .home_dir
        .join(".config/demo-old/config.toml");
    let desired = runtime
        .context()
        .home_dir
        .join(".config/demo-next/config.toml");
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination: previous.clone(),
            backup: None,
            content_hash: crate::install::hash_content(b"previous-managed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("app-manifest.toml"), 0);

    let mut observer = super::super::NullObserver;
    assert!(
        runtime
            .upgrade_apps_approved(
                request,
                &approval,
                AppApprovedUpgradeOptions::default(),
                &mut observer,
                &mut Interaction,
            )
            .await
            .is_err()
    );
    assert_eq!(
        runtime.host().read(&desired).await.unwrap(),
        b"next-managed"
    );
    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert!(runtime.host().read(&previous).await.is_err());
    assert!(runtime.host().read(&desired).await.is_err());
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(
        manifest
            .find_by_source("app/demo/config.toml")
            .unwrap()
            .destination,
        previous
    );
}

#[tokio::test]
async fn approved_privileged_app_upgrade_journals_static_copy_update() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '/etc/demo'\n[permissions]\nschema_version = 1\nadministrator = true\n[[files]]\nsource = 'config.toml'\nrequires_admin = true\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let (destination, _) = seed_privileged_static_copy_app(&runtime, b"managed", None).await;
    runtime
        .host()
        .set_mode(&destination, 0o100600)
        .await
        .unwrap();
    let rollback = crate::action::managed_file_rollback_path(&destination);
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(
            ir.actions.as_slice(),
            [action] if matches!(
                action.kind,
                crate::action::ActionKindV1::UpdateManagedFile {
                    requires_admin: true,
                    ..
                }
            )
        )
    ));

    let mut observer = super::super::NullObserver;
    runtime
        .upgrade_apps_approved(
            request,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"next-managed"
    );
    assert!(runtime.host().read(&rollback).await.is_err());
    let operations = runtime.host().operations();
    assert!(operations.contains(&HostOperation::MovePrivileged {
        from: destination.clone(),
        to: rollback.clone(),
    }));
    assert!(operations.contains(&HostOperation::WritePrivileged(destination.clone())));
    assert!(operations.contains(&HostOperation::SetModePrivileged {
        path: destination,
        mode: 0o100600,
    }));
    assert!(operations.contains(&HostOperation::RemovePrivileged(rollback)));
}

#[tokio::test]
async fn app_managed_update_blocks_an_occupied_transaction_rollback_path() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    runtime
        .host()
        .put_file(&destination, b"previous-managed".to_vec());
    runtime.host().put_file(
        crate::action::managed_file_rollback_path(&destination),
        b"foreign".to_vec(),
    );
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: vec![AppEntry {
                source: "app/demo/config.toml".to_string(),
                destination,
                backup: None,
                content_hash: crate::install::hash_content(b"previous-managed"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
        })
        .unwrap()
        .into_bytes(),
    );
    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: Some("demo".to_string()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_update_rollback_occupied".to_string())
    }));
}

#[tokio::test]
async fn approved_app_install_journals_an_existing_static_managed_update() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"next-managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let rollback = crate::action::managed_file_rollback_path(&destination);
    runtime
        .host()
        .put_file(&destination, b"previous-managed".to_vec());
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: vec![AppEntry {
                source: "app/demo/config.toml".to_string(),
                destination: destination.clone(),
                backup: None,
                content_hash: crate::install::hash_content(b"previous-managed"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
        })
        .unwrap()
        .into_bytes(),
    );
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let mut observer = super::super::NullObserver;
    let report = runtime
        .install_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();
    assert!(report.lifecycle.summary().changed > 0);
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"next-managed"
    );
    assert!(runtime.host().read(&rollback).await.is_err());
    assert!(
        runtime
            .host()
            .read(
                &runtime
                    .context()
                    .shine_dir
                    .join(super::super::APP_OPERATION_JOURNAL_FILE)
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn approved_app_install_journals_backup_creation_and_commits_both_paths() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let backup = crate::install::backup_path(&destination);
    runtime
        .host()
        .put_file(&destination, b"user-original".to_vec());
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    for permission in [
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Remove,
            path: "home:.config/demo/config.toml".to_string(),
        },
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Write,
            path: "home:.config/demo/config.toml.shine.bak".to_string(),
        },
    ] {
        assert!(plan.permissions.required.contains(&permission));
    }
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions[0].actions[0].kind,
        crate::action::ActionKindV1::CreateManagedFileWithBackup { .. }
    ));

    let mut observer = super::super::NullObserver;
    runtime
        .install_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();

    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    let entry = manifest
        .find_by_source("app/demo/config.toml")
        .expect("backup-aware receipt");
    assert_eq!(entry.backup.as_ref(), Some(&backup));
    assert_eq!(runtime.host().read(&destination).await.unwrap(), b"desired");
    assert_eq!(
        runtime.host().read(&backup).await.unwrap(),
        b"user-original"
    );
    assert!(
        runtime
            .host()
            .read(
                &runtime
                    .context()
                    .shine_dir
                    .join(super::super::APP_OPERATION_JOURNAL_FILE)
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn app_install_blocks_instead_of_replacing_an_existing_backup() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let backup = crate::install::backup_path(&destination);
    runtime
        .host()
        .put_file(&destination, b"user-original".to_vec());
    runtime.host().put_file(&backup, b"older-backup".to_vec());
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let plan = runtime.plan_apps(request).await.unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_backup_occupied".to_string())
    }));
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user-original"
    );
    assert_eq!(runtime.host().read(&backup).await.unwrap(), b"older-backup");
}

#[tokio::test]
async fn generated_app_install_is_backup_aware() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "app/demo/shine.toml",
            br#"dest = '~/.config/demo'
[permissions]
schema_version = 1
filesystem = [{ access = ['execute'], base = 'preset', path = 'gen.ts' }]
commands = ['bun']
environment = [{ name = 'SOURCE', sensitivity = 'plain' }]
[[files]]
source = 'generated.txt'
generator = { script = 'gen.ts', runtime = 'bun', env = ['SOURCE'], when_env = 'SOURCE' }
"#
            .to_vec(),
        )
        .file("app/demo/generated.txt", b"fallback".to_vec())
        .file(
            "app/demo/gen.ts",
            b"process.stdout.write('generated')".to_vec(),
        )
        .build();
    let mut runtime = runtime(snapshot);
    runtime
        .context_mut_for_cli()
        .env
        .insert("SOURCE".to_string(), "available".to_string());
    let destination = runtime
        .context()
        .home_dir
        .join(".config/demo/generated.txt");
    let backup = crate::install::backup_path(&destination);
    runtime
        .host()
        .put_file(&destination, b"user-original".to_vec());
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let ready = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(ready.is_ready());
    for permission in [
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Remove,
            path: "home:.config/demo/generated.txt".to_string(),
        },
        PermissionV1::Filesystem {
            access: FilesystemAccessV1::Write,
            path: "home:.config/demo/generated.txt.shine.bak".to_string(),
        },
    ] {
        assert!(ready.permissions.required.contains(&permission));
    }

    runtime.host().put_file(&backup, b"older-backup".to_vec());
    let blocked = runtime.plan_apps(request).await.unwrap();
    assert!(!blocked.is_ready());
    assert!(blocked.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_backup_occupied".to_string())
    }));
}

#[tokio::test]
async fn backup_aware_install_blocks_a_non_regular_destination() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let target = runtime.context().home_dir.join("user-config.toml");
    runtime.host().put_file(&target, b"user-original".to_vec());
    runtime.host().symlink(&target, &destination).await.unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let plan = runtime.plan_apps(request).await.unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_backup_source_not_regular".to_string())
    }));
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user-original"
    );
}

#[tokio::test]
async fn approved_backup_install_rejects_a_backup_created_after_review() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let backup = crate::install::backup_path(&destination);
    runtime
        .host()
        .put_file(&destination, b"user-original".to_vec());
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().put_file(&backup, b"late-backup".to_vec());

    let mut observer = super::super::NullObserver;
    let error = runtime
        .install_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Plan"));
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user-original"
    );
    assert_eq!(runtime.host().read(&backup).await.unwrap(), b"late-backup");
    assert!(
        runtime
            .host()
            .read(
                &runtime
                    .context()
                    .shine_dir
                    .join(super::super::APP_OPERATION_JOURNAL_FILE)
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn app_install_receipt_failure_leaves_journal_for_explicit_recovery() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let manifest_path = runtime.context().shine_dir.join("app-manifest.toml");
    runtime.host().fail_write_after(&manifest_path, 0);

    let mut observer = super::super::NullObserver;
    let error = runtime
        .install_apps_approved(request.clone(), &approval, &mut observer, &mut Interaction)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("failed to write app manifest"));
    assert!(
        runtime
            .host()
            .read(&runtime.context().home_dir.join(".config/demo/config.toml"))
            .await
            .is_ok()
    );
    assert!(
        runtime
            .host()
            .read(
                &runtime
                    .context()
                    .shine_dir
                    .join(super::super::APP_OPERATION_JOURNAL_FILE)
            )
            .await
            .is_ok()
    );
    assert!(runtime.host().read(&manifest_path).await.is_err());

    let blocked = runtime.plan_apps(request).await.unwrap();
    assert!(!blocked.is_ready());
    assert!(blocked.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"app_recovery_required".to_string())
            && step.action == PlanActionV1::Blocked
    }));
}

#[tokio::test]
async fn backup_install_receipt_failure_recovers_the_original_file() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let backup = crate::install::backup_path(&destination);
    runtime
        .host()
        .put_file(&destination, b"user-original".to_vec());
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("app-manifest.toml"), 0);

    let mut observer = super::super::NullObserver;
    assert!(
        runtime
            .install_apps_approved(request, &approval, &mut observer, &mut Interaction)
            .await
            .is_err()
    );
    assert_eq!(runtime.host().read(&destination).await.unwrap(), b"desired");
    assert_eq!(
        runtime.host().read(&backup).await.unwrap(),
        b"user-original"
    );

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::App, true).await;
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user-original"
    );
    assert!(runtime.host().read(&backup).await.is_err());
}

#[tokio::test]
async fn targeted_app_refresh_points_uninstalled_generator_to_install() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                br#"dest = '~/.config/demo'
[[files]]
source = 'generated.txt'
generator = { script = 'gen.ts', runtime = 'bun', env = ['SOURCE'], when_env = 'SOURCE', auto = false }
"#
                .to_vec(),
            )
            .file("app/demo/generated.txt", b"fallback".to_vec())
            .file("app/demo/gen.ts", b"process.stdout.write('generated')".to_vec())
            .build();
    let runtime = runtime(snapshot);

    let error = runtime
        .plan_app_refresh(AppRefreshPlanRequest {
            category: "demo".to_string(),
            file: Some(PathBuf::from("generated.txt")),
            force: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "app 'demo' generated file is not installed: generated.txt; run `shine install app/demo` to install it before refreshing"
    );
}

#[tokio::test]
async fn app_snapshot_plans_keep_category_observations_separate() {
    let mut builder = PresetSnapshot::builder(PresetSourceKind::Embedded);
    for category in ["first", "second"] {
        builder = builder
            .file(
                format!("app/{category}/shine.toml"),
                format!(
                    r#"dest = '~/.config/{category}'
[permissions]
schema_version = 1
filesystem = [{{ access = ['execute'], base = 'preset', path = 'gen.ts' }}]
commands = ['bun']
environment = [{{ name = 'SOURCE', sensitivity = 'plain' }}]
[[files]]
source = 'generated.txt'
generator = {{ script = 'gen.ts', runtime = 'bun', env = ['SOURCE'], when_env = 'SOURCE' }}
"#
                )
                .into_bytes(),
            )
            .file(
                format!("app/{category}/generated.txt"),
                b"fallback".to_vec(),
            )
            .file(
                format!("app/{category}/gen.ts"),
                b"process.stdout.write('generated')".to_vec(),
            );
    }
    let mut runtime = runtime(builder.build());
    runtime
        .context_mut_for_cli()
        .env
        .insert("SOURCE".into(), "value".into());
    // Only one category has been invoked before; these observations differ.
    runtime
        .host()
        .create_dir_all(&runtime.context().shine_dir.join("runtime/app/first"))
        .await
        .unwrap();
    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Install,
            target: None,
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(plan.is_ready());
    for category in ["first", "second"] {
        let target = format!("app/{category}");
        let steps: Vec<_> = plan
            .steps
            .iter()
            .filter(|step| step.target == target)
            .collect();
        let create = steps
            .iter()
            .position(|step| step.resource.as_deref() == Some("generator-runtime:gen.ts"))
            .unwrap();
        let execute = steps
            .iter()
            .position(|step| step.action == PlanActionV1::Execute)
            .unwrap();
        let cleanup = steps
            .iter()
            .position(|step| step.resource.as_deref() == Some("generator-runtime:gen.ts:cleanup"))
            .unwrap();
        assert!(create < execute && execute < cleanup);
    }
}

#[tokio::test]
async fn app_refresh_plan_is_payload_free_and_rejects_changed_destination() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                br#"dest = '~/.config/demo'
[permissions]
schema_version = 1
filesystem = [{ access = ['execute'], base = 'preset', path = 'gen.ts' }]
commands = ['bun']
environment = [{ name = 'SOURCE', sensitivity = 'plain' }]
[[files]]
source = 'generated.txt'
generator = { script = 'gen.ts', runtime = 'bun', env = ['SOURCE'], when_env = 'SOURCE', auto = false }
"#
                .to_vec(),
            )
            .file("app/demo/generated.txt", b"fallback".to_vec())
            .file("app/demo/gen.ts", b"process.stdout.write('generated')".to_vec())
            .build();
    let mut runtime = runtime(snapshot);
    runtime
        .context_mut_for_cli()
        .env
        .insert("SOURCE".to_string(), "sensitive-source-value".to_string());
    let destination = runtime
        .context()
        .home_dir
        .join(".config/demo/generated.txt");
    runtime.host().put_file(&destination, b"installed".to_vec());
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: vec![AppEntry {
                source: "app/demo/generated.txt".to_string(),
                destination: destination.clone(),
                backup: None,
                content_hash: crate::install::hash_content(b"installed"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
        })
        .unwrap()
        .into_bytes(),
    );
    let request = AppRefreshPlanRequest {
        category: "demo".to_string(),
        file: None,
        force: false,
        input_versions: PlanningInputVersions::default(),
    };

    let plan = runtime.plan_app_refresh(request.clone()).await.unwrap();
    assert_eq!(plan.operation, PlanOperationV1::AppRefresh);
    assert!(plan.is_ready());
    let step_index = |action, resource| {
        plan.steps
            .iter()
            .position(|step| step.action == action && step.resource.as_deref() == Some(resource))
            .unwrap()
    };
    let materialize = step_index(PlanActionV1::Create, "generator-runtime:gen.ts");
    let execute = step_index(PlanActionV1::Execute, "generator:generated.txt");
    let cleanup = step_index(PlanActionV1::Remove, "generator-runtime:gen.ts:cleanup");
    assert!(materialize < execute && execute < cleanup);
    for access in [FilesystemAccessV1::Write, FilesystemAccessV1::Remove] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: "shine:runtime/app/demo".to_string(),
                })
        );
    }
    assert!(
        !serde_json::to_string(&plan)
            .unwrap()
            .contains("sensitive-source-value")
    );

    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().put_file(&destination, b"changed".to_vec());
    let mut observer = super::super::NullObserver;
    let error = runtime
        .refresh_app_generators_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Plan changed"));
    assert!(
        !runtime
            .host()
            .operations()
            .iter()
            .any(|operation| matches!(operation, super::super::HostOperation::Run { .. }))
    );
}

#[tokio::test]
async fn app_artifact_plan_scopes_secrets_and_rejects_changed_runtime_state() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
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
    let mut runtime = runtime(snapshot);
    let optional = runtime
        .plan_app_artifact(AppArtifactPlanRequest {
            category: "demo".to_string(),
            action: AppArtifactAction::Apply,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(optional.is_ready());
    runtime
        .context_mut_for_cli()
        .env
        .insert("TOKEN".to_string(), "top-secret-value".to_string());
    let mut versions = PlanningInputVersions::default();
    versions.insert_secret_version("TOKEN", OpaqueSecretVersion::new("vault-revision-9"));
    let request = AppArtifactPlanRequest {
        category: "demo".to_string(),
        action: AppArtifactAction::Apply,
        input_versions: versions,
    };

    let plan = runtime.plan_app_artifact(request.clone()).await.unwrap();
    assert_eq!(plan.operation, PlanOperationV1::AppArtifactApply);
    assert!(plan.is_ready());
    let encoded = serde_json::to_string(&plan).unwrap();
    assert!(!encoded.contains("top-secret-value"));
    assert!(!encoded.contains("vault-revision-9"));
    let remove = runtime
        .plan_app_artifact(AppArtifactPlanRequest {
            action: AppArtifactAction::Remove,
            ..request.clone()
        })
        .await
        .unwrap();
    assert_eq!(remove.operation, PlanOperationV1::AppArtifactRemove);
    assert!(remove.is_ready());

    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().put_file(
        runtime.context().shine_dir.join("state/app/demo/changed"),
        b"changed".to_vec(),
    );
    let mut observer = super::super::NullObserver;
    let error = runtime
        .run_app_artifact_approved(request, &approval, &mut observer)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Plan changed"));
    assert!(
        !runtime
            .host()
            .operations()
            .iter()
            .any(|operation| matches!(operation, super::super::HostOperation::Run { .. }))
    );
}

#[tokio::test]
async fn sys_profile_plan_is_observation_only_and_rejects_changed_shell_state() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "sys/test/shine.toml",
            br#"version = 2
[[items]]
id = 'tool'
label = 'Tool'
permissions = { schema_version = 1 }
detect = { kind = 'path', path = '$HOME/.tool-present' }
install = { kind = 'package', provider = 'homebrew', package = 'tool' }
[[items.shell]]
shells = ['zsh']
phase = 'post'
path = '$HOME/.tool/bin'
"#
            .to_vec(),
        )
        .build();
    let runtime = runtime(snapshot);
    runtime.host().put_file(
        runtime.context().home_dir.join(".tool-present"),
        b"present".to_vec(),
    );
    let request = SysProfilePlanRequest {
        os_id: "test".to_string(),
        item_id: "tool".to_string(),
        enabled: true,
    };

    let plan = runtime.plan_sys_profile(request.clone()).await.unwrap();
    assert_eq!(plan.operation, PlanOperationV1::SysProfileEnable);
    assert!(plan.is_ready());
    let disable = runtime
        .plan_sys_profile(SysProfilePlanRequest {
            enabled: false,
            ..request.clone()
        })
        .await
        .unwrap();
    assert_eq!(disable.operation, PlanOperationV1::SysProfileDisable);
    assert!(disable.is_ready());
    assert!(
        runtime
            .host()
            .operations()
            .iter()
            .all(|operation| matches!(operation, super::super::HostOperation::Read(_)))
    );

    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().put_file(
        runtime.context().home_dir.join(".zshrc"),
        b"user change".to_vec(),
    );
    let error = runtime
        .set_sys_profile_approved(request, &approval)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Plan changed"));
    assert!(
        !runtime.host().operations().iter().any(|operation| matches!(
            operation,
            super::super::HostOperation::Write(_)
                | super::super::HostOperation::Remove(_)
                | super::super::HostOperation::Run { .. }
        ))
    );
}

#[tokio::test]
async fn sys_profile_recovery_restores_owned_blocks_and_preserves_later_user_edits() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
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
shells = ['bash', 'zsh']
phase = 'post'
path = '$HOME/.tool/bin'
"#
            .to_vec(),
        )
        .build();
    let mut runtime = runtime(snapshot);
    runtime.context_mut_for_cli().shell = super::super::ShellType::Bash;
    let profile = runtime.context().home_dir.join(".bashrc");
    runtime.host().put_file(
        runtime.context().home_dir.join(".tool-present"),
        b"present".to_vec(),
    );
    runtime.host().put_file(&profile, b"before\n".to_vec());
    let request = SysProfilePlanRequest {
        os_id: "ubuntu".to_string(),
        item_id: "tool".to_string(),
        enabled: true,
    };
    let plan = runtime.plan_sys_profile(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.target == "sys/profile"
            && step
                .diagnostic_codes
                .contains(&"sys_profile_block_transaction".to_string())
            && step
                .diagnostic_codes
                .contains(&"sys_profile_merge_recovery_unsupported".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("sys-manifest.toml"), 0);
    assert!(
        runtime
            .set_sys_profile_approved(request, &approval)
            .await
            .is_err()
    );
    let interrupted = String::from_utf8(runtime.host().read(&profile).await.unwrap()).unwrap();
    assert!(interrupted.contains("shine ubuntu sys pre"));
    runtime.host().put_file(
        &profile,
        format!("{interrupted}\nuser-after-interruption\n").into_bytes(),
    );

    let recovery_plan = runtime.plan_sys_operation_recovery().await.unwrap();
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::Sys, true).await;
    assert!(recovery_plan.is_ready());
    frontend_recover(
        &runtime,
        crate::frontend::ReviewRequest::SysRecovery,
        &recovery_plan,
    )
    .await;
    let restored = String::from_utf8(runtime.host().read(&profile).await.unwrap()).unwrap();
    assert!(restored.contains("before"));
    assert!(restored.contains("user-after-interruption"));
    assert!(!restored.contains("shine ubuntu sys pre"));
    assert!(
        SysRunManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .entries
            .is_empty()
    );
}

#[tokio::test]
async fn bulk_managed_sys_upgrade_plans_only_profile_enabled_items() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "sys/test/shine.toml",
            br#"version = 2
[[items]]
id = 'enabled'
label = 'Enabled'
mode = 'managed'
driver = 'managed-file'
permissions = { schema_version = 1 }
[items.config]
source = 'enabled.txt'
target = '$HOME/.config/enabled.txt'
[[items]]
id = 'disabled'
label = 'Disabled'
mode = 'managed'
driver = 'managed-file'
permissions = { schema_version = 1 }
[items.config]
source = 'disabled.txt'
target = '$HOME/.config/disabled.txt'
"#
            .to_vec(),
        )
        .file("sys/test/enabled.txt", b"enabled".to_vec())
        .file("sys/test/disabled.txt", b"disabled".to_vec())
        .build();
    let runtime = runtime(snapshot);
    let entry = |item_id: &str, profile_enabled: bool| SysRunEntry {
        os_id: "test".to_string(),
        item_id: item_id.to_string(),
        label: item_id.to_string(),
        status: super::super::SysItemStatus::Installed,
        detail: String::new(),
        updated_at: "1".to_string(),
        managed: true,
        profile_enabled,
        receipt: None,
    };
    runtime.host().put_file(
        runtime.context().shine_dir.join("sys-manifest.toml"),
        toml::to_string(&SysRunManifest {
            schema_version: super::super::SYS_MANIFEST_SCHEMA_VERSION,
            entries: vec![entry("enabled", true), entry("disabled", false)],
        })
        .unwrap()
        .into_bytes(),
    );

    let plan = runtime
        .plan_managed_sys(SysManagedPlanRequest {
            operation: LifecycleOperation::Upgrade,
            os_id: "test".to_string(),
            target: None,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(plan.steps.iter().any(|step| step.target == "sys/enabled"));
    assert!(!plan.steps.iter().any(|step| step.target == "sys/disabled"));
}

#[tokio::test]
async fn app_target_plan_ignores_unrelated_manifest_and_live_state() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"desired".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let demo_destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let other_destination = runtime.context().home_dir.join(".config/other/config.toml");
    runtime
        .host()
        .put_file(&demo_destination, b"desired".to_vec());
    runtime
        .host()
        .put_file(&other_destination, b"first".to_vec());
    let mut manifest = AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![
            AppEntry {
                source: "app/demo/config.toml".to_string(),
                destination: demo_destination,
                backup: None,
                content_hash: crate::install::hash_content(b"desired"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            },
            AppEntry {
                source: "app/other/config.toml".to_string(),
                destination: other_destination.clone(),
                backup: None,
                content_hash: crate::install::hash_content(b"first"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            },
        ],
    };
    let manifest_path = runtime.context().shine_dir.join("app-manifest.toml");
    runtime.host().put_file(
        &manifest_path,
        toml::to_string(&manifest).unwrap().into_bytes(),
    );
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let initial = runtime.plan_apps(request.clone()).await.unwrap();

    manifest.entries[1].content_hash = crate::install::hash_content(b"second");
    runtime.host().put_file(
        &manifest_path,
        toml::to_string(&manifest).unwrap().into_bytes(),
    );
    runtime
        .host()
        .put_file(&other_destination, b"second".to_vec());
    let unchanged = runtime.plan_apps(request).await.unwrap();
    assert_eq!(initial.inputs.state, unchanged.inputs.state);
    assert_eq!(
        initial.fingerprint().unwrap(),
        unchanged.fingerprint().unwrap()
    );
}

#[tokio::test]
async fn filesystem_review_tracks_typed_shell_transactions_without_widening_permissions() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file(
            "shell/demo/shine.toml",
            b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n".to_vec(),
        )
        .file("shell/demo/demo.sh", b"echo demo\n".to_vec())
        .build();
    let mut runtime = external_shell_runtime(snapshot);
    let profile = runtime.context().home_dir.join(".zshrc");
    runtime.context_mut_for_cli().shell_config_paths = vec![profile];
    let plan = runtime.plan_shells(shell_install_request()).await.unwrap();
    assert!(plan.is_ready());
    let purpose = |path: &str| {
        plan.filesystem_review.iter().filter(|group| group.permissions.iter().any(|permission| matches!(permission, PermissionV1::Filesystem { path: actual, .. } if actual == path))).map(|group| group.purpose).collect::<BTreeSet<_>>()
    };
    assert_eq!(
        purpose("home:.zshrc"),
        BTreeSet::from([FilesystemPurposeV1::UserTarget])
    );
    assert_eq!(
        purpose("home:.zshrc.shine.rollback"),
        BTreeSet::from([FilesystemPurposeV1::Recovery])
    );
    // Launcher resources follow the compiling host, even though this fixture
    // selects Linux/Bash to discover the .sh source. Windows installs both shims.
    #[cfg(unix)]
    let installation_paths_expected = ["shine:bin/demo", "shine:shell/profile.sh"];
    #[cfg(not(unix))]
    let installation_paths_expected = [
        "shine:bin/demo.ps1",
        "shine:bin/demo.cmd",
        "shine:shell/profile.sh",
    ];
    let installation_paths = plan
        .filesystem_review
        .iter()
        .filter(|group| group.purpose == FilesystemPurposeV1::Installation)
        .flat_map(|group| group.permissions.iter())
        .filter_map(|permission| match permission {
            PermissionV1::Filesystem { path, .. } => Some(path.as_str()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        installation_paths,
        BTreeSet::from(installation_paths_expected)
    );
    for path in [
        "shine:shell-manifest.toml",
        "shine:shell-operation-journal.toml",
        "shine:installed/shell/demo",
        "shine:installed/shell/.demo.shine.stage",
        "shine:installed/shell/.demo.shine.rollback",
        "shine:installed/shell/demo/demo.sh",
    ] {
        assert_eq!(
            purpose(path),
            BTreeSet::from([FilesystemPurposeV1::Maintenance]),
            "{path}"
        );
    }
    for group in &plan.filesystem_review {
        assert!(
            group
                .permissions
                .iter()
                .all(|permission| plan.permissions.required.contains(permission))
        );
    }
    assert!(
        runtime
            .host()
            .operations()
            .iter()
            .all(|operation| matches!(operation, HostOperation::Read(_)))
    );
}

#[test]
fn filesystem_review_user_destination_wins_even_inside_shine_or_with_rollback_suffix() {
    let runtime = runtime(static_copy_app_snapshot());
    let context = runtime.context();
    let destination = context.shine_dir.join("shell-manifest.toml");
    let rollback_named_destination = context.home_dir.join("my.shine.rollback");
    let mut permissions = PermissionAccumulator::default();
    add_shine_receipt_permission(
        context,
        &mut permissions,
        "shell-manifest.toml",
        LifecycleOperation::Install,
    );
    for path in [&destination, &rollback_named_destination] {
        permissions.implicit(PermissionV1::Filesystem {
            access: FilesystemAccessV1::Write,
            path: review_path(context, path),
        });
    }
    let groups = permissions.review_groups();
    for path in [&destination, &rollback_named_destination] {
        assert!(
            groups
                .iter()
                .any(|group| group.purpose == FilesystemPurposeV1::UserTarget
                    && group.permissions.contains(&PermissionV1::Filesystem {
                        access: FilesystemAccessV1::Write,
                        path: review_path(context, path)
                    }))
        );
    }
}

fn shell_launcher_snapshot() -> PresetSnapshot {
    PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "shell/demo/shine.toml",
                b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .file("shell/demo/demo.sh", b"#!/bin/sh\necho demo\n".to_vec())
            .build()
}

fn shell_bun_launcher_snapshot() -> PresetSnapshot {
    PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "shell/demo/shine.toml",
                b"[[files]]\nsource = 'demo.ts'\ntarget = 'demo'\nruntime = 'bun'\n[files.permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .file("shell/demo/demo.ts", b"console.log('demo')\n".to_vec())
            .build()
}

fn transformed_shell_snapshot(script: &[u8]) -> PresetSnapshot {
    PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "shell/demo/shine.toml",
                b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\ntransforms = ['template']\n[files.permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .file("shell/demo/demo.sh", script.to_vec())
            .build()
}

fn shell_install_request() -> ShellPlanRequest {
    ShellPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo/demo".to_string()),
        force: false,
        purge: false,
        input_versions: PlanningInputVersions::default(),
    }
}

#[tokio::test]
async fn missing_shell_template_inputs_block_approval_without_mutation() {
    let mut missing = runtime(transformed_shell_snapshot(b"@@PRIVATE_INPUT@@"));
    missing.context_mut_for_cli().platform = RuntimePlatform::Linux;
    missing.context_mut_for_cli().shell = super::super::ShellType::Zsh;
    let plan = missing.plan_shells(shell_install_request()).await.unwrap();
    assert!(!plan.is_ready());
    assert!(PlanApprovalV1::for_reviewed_plan(&plan).is_err());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step.diagnostic_codes == ["shell_template_inputs_missing"]
    }));
    let json = serde_json::to_string(&plan).unwrap();
    assert!(!json.contains("PRIVATE_INPUT"));
    assert!(!json.contains("undefined template"));
    assert!(
        missing
            .host()
            .operations()
            .iter()
            .all(|op| matches!(op, HostOperation::Read(_)))
    );

    // Other rendering errors must not be mistaken for absent input.
    let mut invalid = runtime(transformed_shell_snapshot(&[0xff]));
    invalid.context_mut_for_cli().platform = RuntimePlatform::Linux;
    invalid.context_mut_for_cli().shell = super::super::ShellType::Zsh;
    assert!(invalid.plan_shells(shell_install_request()).await.is_err());
}

#[tokio::test]
async fn shell_plan_and_cache_exclude_inactive_platform_sources() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "shell/demo/shine.toml",
                b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\nplatforms = ['unix']\n[files.permissions]\nschema_version = 1\n\n[[files]]\nsource = 'demo.ps1'\ntarget = 'demo'\nplatforms = ['windows']\n[files.permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .file("shell/demo/demo.sh", b"#!/bin/sh\necho demo\n".to_vec())
            .file("shell/demo/demo.ps1", b"Write-Output demo\n".to_vec())
            .file("shell/demo/helper.txt", b"shared helper\n".to_vec())
            .file(
                "shell/syntax/shine.toml",
                b"[[files]]\nsource = 'native.sh'\ntarget = 'native'\n[files.permissions]\nschema_version = 1\n\n[[files]]\nsource = 'native.ps1'\ntarget = 'native'\n[files.permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .file("shell/syntax/native.sh", b"#!/bin/sh\necho native\n".to_vec())
            .file(
                "shell/syntax/native.ps1",
                b"Write-Output native\n".to_vec(),
            )
            .build();
    let mut runtime = runtime(snapshot);
    runtime.context_mut_for_cli().platform = RuntimePlatform::Linux;
    runtime.context_mut_for_cli().shell = super::super::ShellType::Zsh;
    let request = shell_install_request();

    let plan = runtime.plan_shells(request.clone()).await.unwrap();

    assert!(plan.is_ready());
    let inactive_cache = review_path(
        runtime.context(),
        &runtime.context().presets_dir.join("shell/demo/demo.ps1"),
    );
    assert!(!plan.permissions.required.iter().any(|permission| matches!(
        permission,
        PermissionV1::Filesystem { path, .. } if path == &inactive_cache
    )));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .install_shells_approved(request, &approval)
        .await
        .unwrap();
    let cache = runtime.context().presets_dir.join("shell/demo");
    assert!(runtime.host().read(&cache.join("demo.sh")).await.is_ok());
    assert!(runtime.host().read(&cache.join("helper.txt")).await.is_ok());
    assert!(runtime.host().read(&cache.join("demo.ps1")).await.is_err());

    runtime.context_mut_for_cli().platform = RuntimePlatform::Windows;
    runtime.context_mut_for_cli().shell = super::super::ShellType::Bash;
    let categories = runtime.shell_categories(Some("demo")).unwrap();
    assert_eq!(categories.len(), 1);
    assert!(categories[0].files.is_empty());
    let effective = runtime
        .effective_shell_cache_logicals(&categories[0])
        .unwrap();
    assert!(!effective.contains("shell/demo/demo.ps1"));

    let syntax = runtime.shell_categories(Some("syntax")).unwrap();
    assert_eq!(syntax[0].files.len(), 1);
    assert_eq!(syntax[0].files[0].source_rel, PathBuf::from("native.sh"));
    let effective = runtime.effective_shell_cache_logicals(&syntax[0]).unwrap();
    assert!(effective.contains("shell/syntax/native.sh"));
    assert!(!effective.contains("shell/syntax/native.ps1"));
}

#[tokio::test]
async fn shell_upgrade_ignores_uninstalled_categories_and_noop_permissions() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "shell/demo/shine.toml",
                b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .file("shell/demo/demo.sh", b"#!/bin/sh\necho demo\n".to_vec())
            .file(
                "shell/unused/shine.toml",
                b"[[files]]\nsource = 'unused.sh'\ntarget = 'unused'\n[files.permissions]\nschema_version = 1\ncommands = ['unused-runtime']\n"
                    .to_vec(),
            )
            .file(
                "shell/unused/unused.sh",
                b"#!/bin/sh\necho unused\n".to_vec(),
            )
            .file(
                "shell/legacy/shine.toml",
                b"[[files]]\nsource = 'legacy.sh'\ntarget = 'legacy'\n".to_vec(),
            )
            .file(
                "shell/legacy/legacy.sh",
                b"#!/bin/sh\necho legacy\n".to_vec(),
            )
            .build();
    let runtime = runtime(snapshot);
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    runtime
        .install_shells_approved(install, &approval)
        .await
        .unwrap();

    let plan = runtime
        .plan_shells(ShellPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: None,
            force: false,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();

    assert!(plan.is_ready());
    assert!(plan.permissions.required.is_empty());
    assert!(plan.permissions.uncomputable_codes.is_empty());
    assert_eq!(plan.steps.len(), 1);
    assert!(plan.steps.iter().all(|step| {
        step.target == "shell/demo/demo"
            && step.resource.is_none()
            && step.action == PlanActionV1::None
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let operation_count = runtime.host().operations().len();
    let report = runtime
        .upgrade_shells_approved(
            ShellPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: None,
                force: false,
                purge: false,
                input_versions: PlanningInputVersions::default(),
            },
            &approval,
        )
        .await
        .unwrap();
    assert!(report.updated_targets.is_empty());
    let operations = runtime.host().operations();
    let new_mutations = operations[operation_count..]
        .iter()
        .filter(|operation| !matches!(operation, HostOperation::Read(_)))
        .collect::<Vec<_>>();
    assert!(new_mutations.is_empty(), "{new_mutations:?}");

    let error = runtime
        .plan_shells(ShellPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: Some("unused".to_string()),
            force: false,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("not installed"));
}

fn external_shell_runtime(snapshot: PresetSnapshot) -> CoreRuntime<InMemoryHost> {
    let home = std::env::temp_dir().join("shine-planner-external-shell-home");
    let shine = home.join(".shine");
    let mut context = RuntimeContext::isolated(
        home.clone(),
        shine.clone(),
        home.join("external-presets"),
        shine.join("bin"),
        RuntimePlatform::Linux,
    );
    context.shell = super::super::ShellType::Bash;
    context.is_external_presets = true;
    context.external_shell_mode = ExternalShellMode::Snapshot;
    let mut runtime = CoreRuntime::new(InMemoryHost::new(), context, snapshot);
    trust_current_external_shell(&mut runtime);
    runtime
}

#[tokio::test]
async fn shell_upgrade_plans_only_profile_files_that_need_reconciliation() {
    let metadata = b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n";
    let snapshot = |script: &[u8]| {
        PresetSnapshot::builder(PresetSourceKind::External)
            .file("shell/demo/shine.toml", metadata.to_vec())
            .file("shell/demo/demo.sh", script.to_vec())
            .build()
    };
    let installed = external_shell_runtime(snapshot(b"echo old\n"));
    installed.host().put_file(
        installed
            .context()
            .presets_dir
            .join("shell/demo/shine.toml"),
        metadata.to_vec(),
    );
    installed.host().put_file(
        installed.context().presets_dir.join("shell/demo/demo.sh"),
        b"echo old\n".to_vec(),
    );
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&installed.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    installed
        .install_shells_approved(install, &approval)
        .await
        .unwrap();

    let mut changed = CoreRuntime::new(
        installed.host().clone(),
        installed.context().clone(),
        snapshot(b"echo new\n"),
    );
    trust_current_external_shell(&mut changed);
    changed.host().put_file(
        changed.context().presets_dir.join("shell/demo/demo.sh"),
        b"echo new\n".to_vec(),
    );
    let request = ShellPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo/demo".to_string()),
        force: false,
        purge: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = changed.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.resource.as_deref() == Some("shared-snapshot") && step.action == PlanActionV1::Update
    }));
    assert!(!plan.steps.iter().any(|step| step.target == "shell/profile"));
    let shell_config = changed.context().shell_config_paths[0].clone();
    let config_label = review_path(changed.context(), &shell_config);
    assert!(!plan.permissions.required.iter().any(|permission| {
        matches!(permission, PermissionV1::Filesystem { path, .. } if path == &config_label)
    }));
    let config_before = changed.host().read(&shell_config).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    changed
        .upgrade_shells_approved(request.clone(), &approval)
        .await
        .unwrap();
    assert_eq!(
        changed.host().read(&shell_config).await.unwrap(),
        config_before
    );

    let managed_profile = super::super::managed_shell_profile_path(
        &changed.context().shine_dir,
        changed.context().shell,
    );
    changed
        .host()
        .put_file(&managed_profile, b"old profile\n".to_vec());
    let profile_label = review_path(changed.context(), &managed_profile);
    let profile_drift = changed.plan_shells(request.clone()).await.unwrap();
    assert!(profile_drift.steps.iter().any(|step| {
        step.target == "shell/profile" && step.resource.as_deref() == Some(profile_label.as_str())
    }));
    assert!(
        !profile_drift.permissions.required.iter().any(|permission| {
            matches!(permission, PermissionV1::Filesystem { path, .. } if path == &config_label)
        })
    );
    let approval = PlanApprovalV1::for_reviewed_plan(&profile_drift).unwrap();
    changed
        .upgrade_shells_approved(request.clone(), &approval)
        .await
        .unwrap();

    changed
        .host()
        .put_file(&shell_config, b"user config\n".to_vec());
    let drifted = changed.plan_shells(request.clone()).await.unwrap();
    assert!(drifted.is_ready());
    assert!(drifted.steps.iter().any(|step| {
        step.target == "shell/profile"
            && step.resource.as_deref() == Some(config_label.as_str())
            && step.action == PlanActionV1::Update
    }));
    assert!(
        drifted
            .permissions
            .required
            .contains(&PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: config_label,
            })
    );
    let approval = PlanApprovalV1::for_reviewed_plan(&drifted).unwrap();
    changed
        .upgrade_shells_approved(request, &approval)
        .await
        .unwrap();
    let updated = String::from_utf8(changed.host().read(&shell_config).await.unwrap()).unwrap();
    assert!(updated.contains(super::super::profile::SHELL_SENTINEL_START));

    let source_metadata = b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\nneeds_source = true\n";
    let source_snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file("shell/demo/shine.toml", source_metadata.to_vec())
        .file("shell/demo/demo.sh", b"echo new\n".to_vec())
        .build();
    let mut source_changed = CoreRuntime::new(
        changed.host().clone(),
        changed.context().clone(),
        source_snapshot,
    );
    trust_current_external_shell(&mut source_changed);
    source_changed.host().put_file(
        source_changed
            .context()
            .presets_dir
            .join("shell/demo/shine.toml"),
        source_metadata.to_vec(),
    );
    let plan = source_changed
        .plan_shells(ShellPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: Some("demo/demo".to_string()),
            force: false,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.target == "shell/profile" && step.resource.as_deref() == Some(profile_label.as_str())
    }));
    assert!(!plan.permissions.required.iter().any(|permission| {
            matches!(permission, PermissionV1::Filesystem { path, .. } if path == &review_path(source_changed.context(), &shell_config))
        }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    source_changed
        .upgrade_shells_approved(
            ShellPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: Some("demo/demo".to_string()),
                force: false,
                purge: false,
                input_versions: PlanningInputVersions::default(),
            },
            &approval,
        )
        .await
        .unwrap();
}

fn trust_current_external_shell(runtime: &mut CoreRuntime<InMemoryHost>) {
    let requirements = runtime
        .shell_categories(None)
        .unwrap()
        .into_iter()
        .flat_map(|category| {
            category
                .files
                .iter()
                .flat_map(|file| {
                    runtime
                        .shell_external_code_requirements(&category, file)
                        .unwrap()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    runtime.context_mut_for_cli().trust_grants = requirements
        .iter()
        .map(crate::trust::TrustGrantV1::for_reviewed_requirement)
        .collect();
}

#[cfg(unix)]
#[tokio::test]
async fn external_shell_requires_current_target_local_trust() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "shell/demo/shine.toml",
                b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 2\nopaque_code = 'unrestricted'\n"
                    .to_vec(),
            )
            .file("shell/demo/demo.sh", b"#!/bin/sh\n".to_vec())
            .build();
    let mut runtime = external_shell_runtime(snapshot);
    runtime.context_mut_for_cli().trust_grants.clear();
    let request = shell_install_request();

    let blocked = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(!blocked.is_ready());
    assert!(blocked.steps.iter().any(|step| {
        step.target == "shell/demo/demo"
            && step
                .diagnostic_codes
                .contains(&"shell_external_code_not_allowed".to_string())
    }));

    trust_current_external_shell(&mut runtime);
    assert!(runtime.plan_shells(request).await.unwrap().is_ready());
}

#[cfg(unix)]
#[tokio::test]
async fn live_external_shell_requires_development_trust() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .base_root("/external")
        .file(
            "shell/demo/shine.toml",
            b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n".to_vec(),
        )
        .file("shell/demo/demo.sh", b"#!/bin/sh\n".to_vec())
        .build();
    let mut runtime = external_shell_runtime(snapshot);
    runtime.context_mut_for_cli().external_shell_mode = ExternalShellMode::Live;
    let request = shell_install_request();

    let blocked = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(!blocked.is_ready());
    assert!(blocked.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_live_requires_development_trust".to_string())
    }));

    let requirements = runtime
        .external_code_requirements("shell/demo/demo")
        .await
        .unwrap()
        .requirements;
    runtime.context_mut_for_cli().trust_grants = requirements
        .iter()
        .map(|requirement| {
            crate::trust::TrustGrantV1::for_development_requirement(requirement).unwrap()
        })
        .collect();
    assert!(runtime.plan_shells(request).await.unwrap().is_ready());
}

#[cfg(unix)]
#[tokio::test]
async fn shared_snapshot_change_requires_trust_for_installed_siblings() {
    fn snapshot(helper: &[u8]) -> PresetSnapshot {
        PresetSnapshot::builder(PresetSourceKind::External)
                .file(
                    "shell/demo/shine.toml",
                    b"[[files]]\nsource = 'a.sh'\ntarget = 'a'\n\n[[files]]\nsource = 'b.sh'\ntarget = 'b'\n"
                        .to_vec(),
                )
                .file("shell/demo/a.sh", b"#!/bin/sh\necho a\n".to_vec())
                .file("shell/demo/b.sh", b"#!/bin/sh\necho b\n".to_vec())
                .file("shell/demo/helper.data", helper.to_vec())
                .build()
    }

    let previous = external_shell_runtime(snapshot(b"before"));
    let host = previous.host().clone();
    let context = previous.context().clone();
    let deployed = context.shine_dir.join("installed/shell/demo");
    for (name, bytes) in [
        (
            "shine.toml",
            previous.presets().get("shell/demo/shine.toml").unwrap(),
        ),
        ("a.sh", previous.presets().get("shell/demo/a.sh").unwrap()),
        ("b.sh", previous.presets().get("shell/demo/b.sh").unwrap()),
        (
            "helper.data",
            previous.presets().get("shell/demo/helper.data").unwrap(),
        ),
    ] {
        host.put_file(deployed.join(name), bytes.to_vec());
    }
    let entries = ["a", "b"]
        .into_iter()
        .map(|command| ShellManifestEntry {
            launcher_format: None,
            launcher_config_dir: None,

            category: "demo".to_string(),
            command: command.to_string(),
            mode: ExternalShellMode::Snapshot,
            source_path: deployed.join(format!("{command}.sh")),
            rendered_path: context
                .shine_dir
                .join(format!("rendered/shell/demo/{command}.sh")),
            runtime: "native".to_string(),
            bun_dependencies: None,
            dependency_hash: None,
            transforms: Vec::new(),
            env: Vec::new(),
            needs_source: false,
            content_hash: 1,
        })
        .collect();
    host.put_file(
        context.shine_dir.join("shell-manifest.toml"),
        toml::to_string(&ShellManifest {
            schema_version: super::super::SHELL_MANIFEST_SCHEMA_VERSION,
            entries,
        })
        .unwrap()
        .into_bytes(),
    );

    let mut changed = CoreRuntime::new(host, context, snapshot(b"after"));
    let a_requirement = changed
        .external_code_requirements("shell/demo/a")
        .await
        .unwrap()
        .requirements
        .remove(0);
    changed
        .context_mut_for_cli()
        .trust_grants
        .retain(|grant| grant.target != "shell/demo/a");
    changed.context_mut_for_cli().trust_grants.push(
        crate::trust::TrustGrantV1::for_reviewed_requirement(&a_requirement),
    );

    let plan = changed
        .plan_shells(ShellPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: Some("demo/a".to_string()),
            force: false,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.target == "shell/demo/b"
            && step
                .diagnostic_codes
                .contains(&"shell_shared_code_target_trust_required".to_string())
    }));

    let b_requirement = changed
        .external_code_requirements("shell/demo/b")
        .await
        .unwrap()
        .requirements
        .remove(0);
    changed.context_mut_for_cli().trust_grants.push(
        crate::trust::TrustGrantV1::for_reviewed_requirement(&b_requirement),
    );
    let ready = changed
        .plan_shells(ShellPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: Some("demo/a".to_string()),
            force: false,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(ready.is_ready());
    assert!(ready.code_boundaries.iter().any(|boundary| {
        boundary.target == "shell/demo/b"
            && boundary.target_role == CodeTargetRoleV2::SharedResourceAffected
            && boundary.shared_resource.as_deref() == Some("shell/demo/shared-category")
    }));
}

#[tokio::test]
async fn transformed_shell_install_transactions_rendered_output_before_receipt() {
    let runtime = runtime(transformed_shell_snapshot(b"#!/bin/sh\necho first\n"));
    let request = shell_install_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.resource.as_deref() == Some("preset-cache")
            && step
                .diagnostic_codes
                .contains(&"shell_cache_replace_transaction".to_string())
    }));
    assert!(plan.steps.iter().any(|step| {
        step.resource.as_deref() == Some("rendered-output")
            && step
                .diagnostic_codes
                .contains(&"shell_rendered_replace_transaction".to_string())
    }));
    let rendered = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/demo.sh");
    let rollback = managed_file_rollback_path(&rendered);
    for (access, path) in [
        (FilesystemAccessV1::Write, &rendered),
        (FilesystemAccessV1::Remove, &rendered),
        (FilesystemAccessV1::Write, &rollback),
        (FilesystemAccessV1::Remove, &rollback),
    ] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: review_path(runtime.context(), path),
                })
        );
    }
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .install_shells_approved(request, &approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&rendered).await.unwrap(),
        b"#!/bin/sh\necho first\n"
    );
    assert!(runtime.host().metadata(&rollback).await.is_err());
    assert!(
        runtime
            .host()
            .metadata(
                &runtime
                    .context()
                    .shine_dir
                    .join(super::super::SHELL_OPERATION_JOURNAL_FILE)
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn embedded_shell_cache_receipt_failure_removes_created_files_on_recovery() {
    let runtime = runtime(shell_launcher_snapshot());
    let request = shell_install_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .install_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt write should fail");
    let source = runtime.context().presets_dir.join("shell/demo/demo.sh");
    assert_eq!(
        runtime.host().read(&source).await.unwrap(),
        b"#!/bin/sh\necho demo\n"
    );
    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_previous_cache".to_string())
    }));
    frontend_recover(
        &runtime,
        crate::frontend::ReviewRequest::ShellRecovery,
        &recovery,
    )
    .await;
    assert!(runtime.host().metadata(&source).await.is_err());
    assert!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .is_none()
    );
}

#[tokio::test]
async fn embedded_shell_cache_marker_failure_restores_previous_files_and_receipt() {
    let original = runtime(shell_launcher_snapshot());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install, &approval)
        .await
        .unwrap();
    let previous_receipt = ShellManifest::load(original.host(), &original.context().shine_dir)
        .await
        .unwrap()
        .find("shell/demo/demo")
        .unwrap()
        .clone();
    let desired = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "shell/demo/shine.toml",
                b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .file(
                "shell/demo/demo.sh",
                b"#!/bin/sh\necho desired\n".to_vec(),
            )
            .build();
    let runtime = CoreRuntime::new(original.host().clone(), original.context().clone(), desired);
    let request = ShellPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".to_string()),
        force: true,
        purge: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_cache_replace_transaction".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let journal = runtime
        .context()
        .shine_dir
        .join(super::super::SHELL_OPERATION_JOURNAL_FILE);
    runtime.host().fail_write_after(&journal, 2);
    runtime
        .install_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell cache receipt marker write should fail");
    let source = runtime.context().presets_dir.join("shell/demo/demo.sh");
    let rollback = managed_file_rollback_path(&source);
    assert_eq!(
        runtime.host().read(&source).await.unwrap(),
        b"#!/bin/sh\necho desired\n"
    );
    assert_eq!(
        runtime.host().read(&rollback).await.unwrap(),
        b"#!/bin/sh\necho demo\n"
    );
    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&source).await.unwrap(),
        b"#!/bin/sh\necho demo\n"
    );
    assert!(runtime.host().metadata(&rollback).await.is_err());
    assert_eq!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .unwrap(),
        &previous_receipt
    );
}

#[tokio::test]
async fn occupied_embedded_shell_cache_rollback_blocks_before_mutation() {
    let runtime = runtime(shell_launcher_snapshot());
    let source = runtime.context().presets_dir.join("shell/demo/demo.sh");
    let rollback = managed_file_rollback_path(&source);
    runtime.host().put_file(&rollback, b"occupied".to_vec());
    let plan = runtime.plan_shells(shell_install_request()).await.unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.resource.as_deref() == Some("preset-cache")
            && step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"shell_cache_rollback_occupied".to_string())
    }));
    assert!(runtime.host().metadata(&source).await.is_err());
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), b"occupied");
}

#[tokio::test]
async fn rendered_shell_receipt_failure_removes_created_output_on_recovery() {
    let runtime = runtime(transformed_shell_snapshot(b"#!/bin/sh\necho first\n"));
    let request = shell_install_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .install_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt write should fail");
    let rendered = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/demo.sh");
    assert!(runtime.host().metadata(&rendered).await.is_ok());
    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_previous_rendered_file".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert!(runtime.host().metadata(&rendered).await.is_err());
    assert!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .is_none()
    );
}

#[tokio::test]
async fn rendered_shell_marker_failure_restores_previous_output_and_receipt() {
    let original = runtime(transformed_shell_snapshot(b"#!/bin/sh\necho previous\n"));
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install, &approval)
        .await
        .unwrap();
    let previous_receipt = ShellManifest::load(original.host(), &original.context().shine_dir)
        .await
        .unwrap()
        .find("shell/demo/demo")
        .unwrap()
        .clone();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        transformed_shell_snapshot(b"#!/bin/sh\necho desired\n"),
    );
    runtime.host().put_file(
        runtime.context().presets_dir.join("shell/demo/demo.sh"),
        b"#!/bin/sh\necho desired\n".to_vec(),
    );
    let request = ShellPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let journal = runtime
        .context()
        .shine_dir
        .join(super::super::SHELL_OPERATION_JOURNAL_FILE);
    runtime.host().fail_write_after(&journal, 2);
    runtime
        .upgrade_shells_approved(request, &approval)
        .await
        .err()
        .expect("rendered receipt marker write should fail");
    let rendered = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/demo.sh");
    let rollback = managed_file_rollback_path(&rendered);
    assert_eq!(
        runtime.host().read(&rendered).await.unwrap(),
        b"#!/bin/sh\necho desired\n"
    );
    assert_eq!(
        runtime.host().read(&rollback).await.unwrap(),
        b"#!/bin/sh\necho previous\n"
    );
    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&rendered).await.unwrap(),
        b"#!/bin/sh\necho previous\n"
    );
    assert!(runtime.host().metadata(&rollback).await.is_err());
    assert_eq!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .unwrap(),
        &previous_receipt
    );
}

#[tokio::test]
async fn transformed_shell_uninstall_transactions_rendered_output() {
    let runtime = runtime(transformed_shell_snapshot(b"#!/bin/sh\necho rendered\n"));
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    runtime
        .install_shells_approved(install, &approval)
        .await
        .unwrap();

    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.resource.as_deref() == Some("rendered-output")
            && step.action == PlanActionV1::Remove
            && step
                .diagnostic_codes
                .contains(&"shell_rendered_file_remove_transaction".to_string())
    }));
    let rendered = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/demo.sh");
    let unrelated = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/unrelated.sh");
    runtime
        .host()
        .put_file(&unrelated, b"unrelated rendered bytes".to_vec());
    let rollback = managed_file_rollback_path(&rendered);
    for (access, path) in [
        (FilesystemAccessV1::Remove, &rendered),
        (FilesystemAccessV1::Write, &rollback),
        (FilesystemAccessV1::Remove, &rollback),
    ] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: review_path(runtime.context(), path),
                })
        );
    }

    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .unwrap();
    assert!(runtime.host().metadata(&rendered).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_err());
    assert_eq!(
        runtime.host().read(&unrelated).await.unwrap(),
        b"unrelated rendered bytes"
    );
    assert!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .is_none()
    );
}

#[tokio::test]
async fn rendered_uninstall_receipt_failure_restores_file_on_recovery() {
    let runtime = runtime(transformed_shell_snapshot(b"#!/bin/sh\necho rendered\n"));
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    runtime
        .install_shells_approved(install, &approval)
        .await
        .unwrap();
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let rendered = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/demo.sh");
    let rollback = managed_file_rollback_path(&rendered);
    let cache = runtime.context().presets_dir.join("shell/demo/demo.sh");
    let cache_rollback = managed_file_rollback_path(&cache);
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt removal should fail");
    assert!(runtime.host().metadata(&rendered).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_ok());
    assert!(runtime.host().metadata(&cache).await.is_err());
    assert!(runtime.host().metadata(&cache_rollback).await.is_ok());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_removed_rendered_file".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&rendered).await.unwrap(),
        b"#!/bin/sh\necho rendered\n"
    );
    assert!(runtime.host().metadata(&rollback).await.is_err());
    assert_eq!(
        runtime.host().read(&cache).await.unwrap(),
        b"#!/bin/sh\necho rendered\n"
    );
    assert!(runtime.host().metadata(&cache_rollback).await.is_err());
    assert!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .is_some()
    );
}

#[tokio::test]
async fn shell_profile_recovery_preserves_unrelated_post_interruption_edits() {
    let runtime = runtime(shell_launcher_snapshot());
    let config = runtime.context().shell_config_paths[0].clone();
    runtime.host().put_file(&config, b"before\n".to_vec());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    runtime
        .install_shells_approved(install, &approval)
        .await
        .unwrap();
    assert!(
        String::from_utf8(runtime.host().read(&config).await.unwrap())
            .unwrap()
            .contains(super::super::profile::SHELL_SENTINEL_START)
    );

    let mut request = shell_uninstall_request();
    request.target = None;
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.steps.iter().any(|step| {
        step.target == "shell/profile"
            && step
                .diagnostic_codes
                .contains(&"shell_profile_reconcile_transaction".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt removal should fail");
    runtime
        .host()
        .put_file(&config, b"before\nuser-after-interruption\n".to_vec());
    let journal_text = String::from_utf8(
        runtime
            .host()
            .read(
                &runtime
                    .context()
                    .shine_dir
                    .join(super::super::SHELL_OPERATION_JOURNAL_FILE),
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        journal_text.contains("reconcile-shell-profile"),
        "journal: {journal_text}"
    );

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(
        recovery.steps.iter().any(|step| {
            step.diagnostic_codes
                .contains(&"shell_recovery_restore_profile".to_string())
        }),
        "recovery diagnostics: {:?}",
        recovery
            .steps
            .iter()
            .flat_map(|step| step.diagnostic_codes.iter())
            .collect::<Vec<_>>()
    );
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    let restored = String::from_utf8(runtime.host().read(&config).await.unwrap()).unwrap();
    assert!(restored.contains("before"));
    assert!(restored.contains("user-after-interruption"));
    assert!(restored.contains(super::super::profile::SHELL_SENTINEL_START));
    assert!(restored.contains(super::super::profile::SHELL_SENTINEL_END));
}

#[tokio::test]
async fn rendered_uninstall_recovery_blocks_modified_rollback() {
    let runtime = runtime(transformed_shell_snapshot(b"#!/bin/sh\necho rendered\n"));
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    runtime
        .install_shells_approved(install, &approval)
        .await
        .unwrap();
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt removal should fail");
    let rendered = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/demo.sh");
    let rollback = managed_file_rollback_path(&rendered);
    runtime
        .host()
        .put_file(&rollback, b"user changed rollback".to_vec());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(!recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_rendered_file_removal_changed".to_string())
    }));
    assert_eq!(
        runtime.host().read(&rollback).await.unwrap(),
        b"user changed rollback"
    );
}

#[tokio::test]
async fn rendered_uninstall_marker_failure_reconstructs_receipt_before_restore() {
    let runtime = runtime(transformed_shell_snapshot(b"#!/bin/sh\necho rendered\n"));
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    runtime
        .install_shells_approved(install, &approval)
        .await
        .unwrap();
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let journal = runtime
        .context()
        .shine_dir
        .join(super::super::SHELL_OPERATION_JOURNAL_FILE);
    runtime.host().fail_write_after(&journal, 4);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell rendered removal commit marker should fail");
    let rendered = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/demo.sh");
    let rollback = managed_file_rollback_path(&rendered);
    assert!(runtime.host().metadata(&rendered).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_ok());
    assert!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .is_none()
    );

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_removed_rendered_file".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&rendered).await.unwrap(),
        b"#!/bin/sh\necho rendered\n"
    );
    assert!(runtime.host().metadata(&rollback).await.is_err());
    assert!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .is_some()
    );
}

#[tokio::test]
async fn rendered_uninstall_preserves_a_file_with_an_unselected_consumer() {
    let runtime = runtime(PresetSnapshot::builder(PresetSourceKind::Embedded).build());
    let rendered = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/shared.sh");
    runtime
        .host()
        .put_file(&rendered, b"#!/bin/sh\necho shared\n".to_vec());
    let entries = ["one", "two"]
        .into_iter()
        .map(|command| ShellManifestEntry {
            launcher_format: None,
            launcher_config_dir: None,

            category: "demo".to_string(),
            command: command.to_string(),
            mode: ExternalShellMode::Snapshot,
            source_path: runtime.context().presets_dir.join("shell/demo/shared.sh"),
            rendered_path: rendered.clone(),
            runtime: "native".to_string(),
            bun_dependencies: None,
            dependency_hash: None,
            transforms: vec!["template".to_string()],
            env: Vec::new(),
            needs_source: false,
            content_hash: 1,
        })
        .collect();
    runtime.host().put_file(
        runtime.context().shine_dir.join("shell-manifest.toml"),
        toml::to_string(&ShellManifest {
            schema_version: super::super::SHELL_MANIFEST_SCHEMA_VERSION,
            entries,
        })
        .unwrap()
        .into_bytes(),
    );
    let plan = runtime
        .plan_shells(ShellPlanRequest {
            operation: LifecycleOperation::Uninstall,
            target: Some("demo/one".to_string()),
            force: false,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(!plan.steps.iter().any(|step| {
        step.resource.as_deref() == Some("rendered-output") && step.action == PlanActionV1::Remove
    }));
    assert_eq!(
        runtime.host().read(&rendered).await.unwrap(),
        b"#!/bin/sh\necho shared\n"
    );
}

#[tokio::test]
async fn rendered_uninstall_blocks_an_occupied_rollback_when_destination_is_missing() {
    let runtime = runtime(transformed_shell_snapshot(b"#!/bin/sh\necho rendered\n"));
    let rendered = runtime
        .context()
        .shine_dir
        .join("rendered/shell/demo/demo.sh");
    let rollback = managed_file_rollback_path(&rendered);
    runtime.host().put_file(
        runtime.context().shine_dir.join("shell-manifest.toml"),
        toml::to_string(&ShellManifest {
            schema_version: super::super::SHELL_MANIFEST_SCHEMA_VERSION,
            entries: vec![ShellManifestEntry {
                launcher_format: None,
                launcher_config_dir: None,

                category: "demo".to_string(),
                command: "demo".to_string(),
                mode: ExternalShellMode::Snapshot,
                source_path: runtime.context().presets_dir.join("shell/demo/demo.sh"),
                rendered_path: rendered.clone(),
                runtime: "native".to_string(),
                bun_dependencies: None,
                dependency_hash: None,
                transforms: vec!["template".to_string()],
                env: Vec::new(),
                needs_source: false,
                content_hash: 1,
            }],
        })
        .unwrap()
        .into_bytes(),
    );
    runtime
        .host()
        .put_file(&rollback, b"unclaimed rollback".to_vec());

    let plan = runtime
        .plan_shells(shell_uninstall_request())
        .await
        .unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"shell_rendered_file_removal_rollback_occupied".to_string())
    }));
    assert_eq!(
        runtime.host().read(&rollback).await.unwrap(),
        b"unclaimed rollback"
    );
}

#[tokio::test]
async fn live_render_refuses_to_run_while_shell_recovery_is_pending() {
    let runtime = runtime(transformed_shell_snapshot(b"#!/bin/sh\necho rendered\n"));
    let install = shell_install_request();
    let plan = runtime.plan_shells(install.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .install_shells_approved(install, &approval)
        .await
        .err()
        .expect("Shell receipt write should fail");

    let error = runtime
        .render_live_shell("shell/demo/demo")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("requires explicit recovery"));
}

async fn relocated_bun_shell_runtime() -> (CoreRuntime<InMemoryHost>, ShellPlanRequest) {
    let metadata = ["compress", "convert", "resize"].map(|name| format!(
            "[[files]]\nsource = '{name}.ts'\ntarget = 'img-{name}'\nruntime = 'bun'\nenv = ['IMAGE_QUALITY']\n[files.permissions]\nschema_version = 1\nenvironment = [{{ name = 'IMAGE_QUALITY', sensitivity = 'plain' }}]\n"
        )).join("\n");
    let snapshot = |source| {
        let mut builder = PresetSnapshot::builder(source)
            .file("shell/demo/shine.toml", metadata.as_bytes().to_vec());
        for name in ["compress", "convert", "resize"] {
            builder = builder.file(
                format!("shell/demo/{name}.ts"),
                b"console.log('fixture');\n".to_vec(),
            );
        }
        builder.build()
    };
    let mut original = runtime(snapshot(PresetSourceKind::Embedded));
    original
        .context_mut_for_cli()
        .env
        .insert("IMAGE_QUALITY".into(), "80".into());
    let mut request = shell_install_request();
    request.target = Some("demo".into());
    let plan = original.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    original
        .install_shells_approved(request.clone(), &approval)
        .await
        .unwrap();
    let mut context = original.context().clone();
    context.presets_dir = context.home_dir.join("moved-external-presets");
    context.is_external_presets = true;
    context.external_shell_mode = ExternalShellMode::Snapshot;
    let mut moved = CoreRuntime::new(
        original.host().clone(),
        context,
        snapshot(PresetSourceKind::External),
    );
    trust_current_external_shell(&mut moved);
    for (logical, bytes) in moved.presets().files() {
        moved
            .host()
            .put_file(moved.context().presets_dir.join(logical), bytes.clone());
    }
    request.operation = LifecycleOperation::Upgrade;
    (moved, request)
}

#[tokio::test]
async fn shell_upgrade_after_source_relocation_retains_receipt_owned_bun_launchers() {
    let (runtime, request) = relocated_bun_shell_runtime().await;
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_snapshot_replace_transaction".into())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .upgrade_shells_approved(request, &approval)
        .await
        .unwrap();
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    for name in ["compress", "convert", "resize"] {
        let target = format!("shell/demo/img-{name}");
        let receipt = manifest.find(&target).unwrap();
        let expected = runtime
            .context()
            .shine_dir
            .join(format!("installed/shell/demo/{name}.ts"));
        assert_eq!(receipt.source_path, expected);
        let spec = shell_link_spec_from_manifest_entry(receipt).unwrap();
        for resource in prepare_launcher_resources(&runtime.context().bin_dir, &spec) {
            assert!(
                prepared_launcher_resource_is_exact(runtime.host(), &resource)
                    .await
                    .unwrap()
            );
        }
        assert_eq!(
            runtime.host().read(&expected).await.unwrap(),
            b"console.log('fixture');\n"
        );
    }
    let snapshot = runtime.context().shine_dir.join("installed/shell/demo");
    for path in [
        shell_snapshot_stage_path(&snapshot),
        shell_snapshot_rollback_path(&snapshot),
        runtime
            .context()
            .shine_dir
            .join("shell-operation-journal.toml"),
    ] {
        assert!(runtime.host().metadata(&path).await.is_err());
    }
}

#[tokio::test]
async fn shell_upgrade_after_source_relocation_still_blocks_foreign_launchers() {
    let (runtime, request) = relocated_bun_shell_runtime().await;
    let launcher = command_path_for_name(&runtime.context().bin_dir, "img-compress".as_ref());
    let foreign = b"#!/bin/sh\necho foreign\n";
    runtime.host().put_file(&launcher, foreign.to_vec());
    let plan = runtime.plan_shells(request).await.unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(
        |step| step.target == "shell/demo/img-compress" && step.action == PlanActionV1::Blocked
    ));
    assert_eq!(runtime.host().read(&launcher).await.unwrap(), foreign);
}

#[tokio::test]
async fn approved_external_shell_install_transactions_the_shared_snapshot() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "shell/demo/shine.toml",
                b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .file("shell/demo/demo.sh", b"#!/bin/sh\necho external\n".to_vec())
            .build();
    let runtime = external_shell_runtime(snapshot);
    runtime.host().put_file(
            runtime
                .context()
                .presets_dir
                .join("shell/demo/shine.toml"),
            b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 1\n"
                .to_vec(),
        );
    runtime.host().put_file(
        runtime.context().presets_dir.join("shell/demo/demo.sh"),
        b"#!/bin/sh\necho external\n".to_vec(),
    );
    let request = shell_install_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.target == "shell/demo"
            && step
                .diagnostic_codes
                .contains(&"shell_snapshot_replace_transaction".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .install_shells_approved(request, &approval)
        .await
        .unwrap();

    let installed = runtime
        .context()
        .shine_dir
        .join("installed/shell/demo/demo.sh");
    assert_eq!(
        runtime.host().read(&installed).await.unwrap(),
        b"#!/bin/sh\necho external\n"
    );
    assert!(
        runtime
            .host()
            .metadata(&shell_snapshot_stage_path(
                &runtime.context().shine_dir.join("installed/shell/demo")
            ))
            .await
            .is_err()
    );
    assert!(
        runtime
            .host()
            .metadata(&shell_snapshot_rollback_path(
                &runtime.context().shine_dir.join("installed/shell/demo")
            ))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn external_shell_snapshot_marker_failure_restores_receipts_tree_and_launcher() {
    let metadata = b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 1\n";
    let script = b"#!/bin/sh\necho external\n";
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file("shell/demo/shine.toml", metadata.to_vec())
        .file("shell/demo/demo.sh", script.to_vec())
        .build();
    let runtime = external_shell_runtime(snapshot);
    runtime.host().put_file(
        runtime.context().presets_dir.join("shell/demo/shine.toml"),
        metadata.to_vec(),
    );
    runtime.host().put_file(
        runtime.context().presets_dir.join("shell/demo/demo.sh"),
        script.to_vec(),
    );
    let request = shell_install_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let journal = runtime
        .context()
        .shine_dir
        .join(super::super::SHELL_OPERATION_JOURNAL_FILE);
    runtime.host().fail_write_after(&journal, 4);
    assert!(
        runtime
            .install_shells_approved(request, &approval)
            .await
            .is_err()
    );

    let destination = runtime.context().shine_dir.join("installed/shell/demo");
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    assert_eq!(
        runtime
            .host()
            .read(&destination.join("demo.sh"))
            .await
            .unwrap(),
        script
    );
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().metadata(&journal).await.is_ok());
    assert!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .is_some()
    );

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_previous_snapshot".to_string())
    }));
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_remove_created_launcher".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert!(runtime.host().metadata(&destination).await.is_err());
    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().metadata(&journal).await.is_err());
    assert!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .is_none()
    );
}

#[tokio::test]
async fn external_shell_snapshot_uninstall_receipt_failure_restores_tree_and_receipt() {
    let metadata = b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 1\n";
    let script = b"#!/bin/sh\necho external\n";
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file("shell/demo/shine.toml", metadata.to_vec())
        .file("shell/demo/demo.sh", script.to_vec())
        .build();
    let runtime = external_shell_runtime(snapshot);
    runtime.host().put_file(
        runtime.context().presets_dir.join("shell/demo/shine.toml"),
        metadata.to_vec(),
    );
    runtime.host().put_file(
        runtime.context().presets_dir.join("shell/demo/demo.sh"),
        script.to_vec(),
    );
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    runtime
        .install_shells_approved(install, &approval)
        .await
        .unwrap();

    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.resource.as_deref() == Some("shared-snapshot")
            && step.action == PlanActionV1::Remove
            && step
                .diagnostic_codes
                .contains(&"shell_snapshot_remove_transaction".to_string())
    }));
    let destination = runtime.context().shine_dir.join("installed/shell/demo");
    let rollback = shell_snapshot_rollback_path(&destination);
    for (access, path) in [
        (FilesystemAccessV1::Remove, &destination),
        (FilesystemAccessV1::Write, &rollback),
        (FilesystemAccessV1::Remove, &rollback),
    ] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: review_path(runtime.context(), path),
                })
        );
    }
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt removal should fail");
    assert!(runtime.host().metadata(&destination).await.is_err());
    assert_eq!(
        runtime
            .host()
            .read(&rollback.join("demo.sh"))
            .await
            .unwrap(),
        script
    );

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_removed_snapshot".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(
        runtime
            .host()
            .read(&destination.join("demo.sh"))
            .await
            .unwrap(),
        script
    );
    assert!(runtime.host().metadata(&rollback).await.is_err());
    assert!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .is_some()
    );
}

#[tokio::test]
async fn external_shell_snapshot_upgrade_marker_failure_restores_previous_tree_and_receipt() {
    let metadata = b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 1\n";
    let previous_script = b"#!/bin/sh\necho previous\n";
    let desired_script = b"#!/bin/sh\necho desired\n";
    let previous_snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file("shell/demo/shine.toml", metadata.to_vec())
        .file("shell/demo/demo.sh", previous_script.to_vec())
        .build();
    let original = external_shell_runtime(previous_snapshot);
    original.host().put_file(
        original.context().presets_dir.join("shell/demo/shine.toml"),
        metadata.to_vec(),
    );
    original.host().put_file(
        original.context().presets_dir.join("shell/demo/demo.sh"),
        previous_script.to_vec(),
    );
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install, &approval)
        .await
        .unwrap();
    let previous_receipt = ShellManifest::load(original.host(), &original.context().shine_dir)
        .await
        .unwrap()
        .find("shell/demo/demo")
        .unwrap()
        .clone();

    let desired_snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file("shell/demo/shine.toml", metadata.to_vec())
        .file("shell/demo/demo.sh", desired_script.to_vec())
        .build();
    let mut runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        desired_snapshot,
    );
    trust_current_external_shell(&mut runtime);
    runtime.host().put_file(
        runtime.context().presets_dir.join("shell/demo/demo.sh"),
        desired_script.to_vec(),
    );
    let request = ShellPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let journal = runtime
        .context()
        .shine_dir
        .join(super::super::SHELL_OPERATION_JOURNAL_FILE);
    runtime.host().fail_write_after(&journal, 2);
    assert!(
        runtime
            .upgrade_shells_approved(request, &approval)
            .await
            .is_err()
    );
    let destination = runtime.context().shine_dir.join("installed/shell/demo");
    let rollback = shell_snapshot_rollback_path(&destination);
    assert_eq!(
        runtime
            .host()
            .read(&destination.join("demo.sh"))
            .await
            .unwrap(),
        desired_script
    );
    assert_eq!(
        runtime
            .host()
            .read(&rollback.join("demo.sh"))
            .await
            .unwrap(),
        previous_script
    );

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(
        runtime
            .host()
            .read(&destination.join("demo.sh"))
            .await
            .unwrap(),
        previous_script
    );
    assert!(runtime.host().metadata(&rollback).await.is_err());
    let restored_receipt = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap()
        .find("shell/demo/demo")
        .unwrap()
        .clone();
    assert_eq!(restored_receipt, previous_receipt);
}

#[tokio::test]
async fn approved_shell_launcher_creation_journals_before_mutation_and_commits_after_receipt() {
    let runtime = runtime(shell_launcher_snapshot());
    let request = shell_install_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    let journal = runtime
        .context()
        .shine_dir
        .join(super::super::SHELL_OPERATION_JOURNAL_FILE);
    for access in [FilesystemAccessV1::Write, FilesystemAccessV1::Remove] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: "shine:shell-operation-journal.toml".to_string(),
                })
        );
    }
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .install_shells_approved(request, &approval)
        .await
        .unwrap();

    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let manifest_path = runtime.context().shine_dir.join("shell-manifest.toml");
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().read(&journal).await.is_err());
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.find("shell/demo/demo").is_some());

    let operations = runtime.host().operations();
    let first_journal_write = operations
        .iter()
        .position(|operation| matches!(operation, HostOperation::Write(path) if path == &journal))
        .unwrap();
    let launcher_mutation = operations
        .iter()
        .position(|operation| {
            matches!(operation, HostOperation::CreateSymlink { link, .. } if link == &launcher)
                || matches!(operation, HostOperation::Write(path) if path == &launcher)
        })
        .unwrap();
    let receipt_write = operations
        .iter()
        .position(
            |operation| matches!(operation, HostOperation::Write(path) if path == &manifest_path),
        )
        .unwrap();
    let journal_commit = operations
        .iter()
        .position(|operation| matches!(operation, HostOperation::Remove(path) if path == &journal))
        .unwrap();
    assert!(first_journal_write < launcher_mutation);
    assert!(launcher_mutation < receipt_write);
    assert!(receipt_write < journal_commit);
}

#[tokio::test]
async fn shell_receipt_failure_leaves_explicit_recovery_that_removes_unchanged_launcher() {
    let runtime = runtime(shell_launcher_snapshot());
    let request = shell_install_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let manifest_path = runtime.context().shine_dir.join("shell-manifest.toml");
    runtime.host().fail_write_after(&manifest_path, 0);

    let error = runtime
        .install_shells_approved(request.clone(), &approval)
        .await
        .err()
        .expect("Shell receipt write should fail");
    assert!(error.to_string().contains("failed to write shell manifest"));
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let journal = runtime
        .context()
        .shine_dir
        .join(super::super::SHELL_OPERATION_JOURNAL_FILE);
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().read(&journal).await.is_ok());

    let blocked = runtime.plan_shells(request).await.unwrap();
    assert!(!blocked.is_ready());
    assert!(blocked.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"shell_recovery_required".to_string())
    }));
    let recovery_plan = runtime.plan_shell_operation_recovery().await.unwrap();
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::Shell, true).await;
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    let report = runtime
        .recover_shell_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(report.rolled_back_actions.len(), 3);
    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().read(&journal).await.is_err());
}

#[tokio::test]
async fn shell_recovery_preserves_a_launcher_changed_after_interruption() {
    let runtime = runtime(shell_launcher_snapshot());
    let request = shell_install_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .install_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt write should fail");
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    runtime
        .host()
        .put_file(&launcher, b"#!/bin/sh\necho user\n".to_vec());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::Shell, false).await;
    assert!(!recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"shell_recovery_launcher_changed".to_string())
    }));
    assert_eq!(
        runtime.host().read(&launcher).await.unwrap(),
        b"#!/bin/sh\necho user\n"
    );
}

#[tokio::test]
async fn shell_recovery_preserves_a_launcher_after_receipt_commit() {
    let runtime = runtime(shell_launcher_snapshot());
    let request = shell_install_request();
    let plan = runtime.plan_shells(request).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let journal = runtime
        .context()
        .shine_dir
        .join(super::super::SHELL_OPERATION_JOURNAL_FILE);
    runtime.host().fail_remove_after(&journal, 0);
    let error = runtime
        .install_shells_approved(shell_install_request(), &approval)
        .await
        .err()
        .expect("Shell journal commit should fail");
    assert!(
        error
            .to_string()
            .contains("removing Shell operation journal")
    );

    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.find("shell/demo/demo").is_some());
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::Shell, true).await;
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_receipt_already_committed".to_string())
    }));
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    let report = runtime
        .recover_shell_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert!(report.rolled_back_actions.is_empty());
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().read(&journal).await.is_err());
}

#[tokio::test]
async fn managed_shell_launcher_update_moves_old_resource_before_receipt_commit() {
    let original = runtime(shell_launcher_snapshot());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install.clone(), &approval)
        .await
        .unwrap();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        shell_bun_launcher_snapshot(),
    );
    let plan = runtime.plan_shells(install.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_managed_launcher_update_transaction".to_string())
    }));
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let rollback = managed_file_rollback_path(&launcher);
    for (access, path) in [
        (FilesystemAccessV1::Remove, &launcher),
        (FilesystemAccessV1::Write, &rollback),
        (FilesystemAccessV1::Remove, &rollback),
    ] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: review_path(runtime.context(), path),
                })
        );
    }
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .install_shells_approved(install, &approval)
        .await
        .unwrap();
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(manifest.find("shell/demo/demo").unwrap().runtime, "bun");
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().metadata(&rollback).await.is_err());
    assert!(
        runtime
            .host()
            .read(
                &runtime
                    .context()
                    .shine_dir
                    .join("shell-operation-journal.toml"),
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn shell_launcher_update_receipt_failure_recovers_previous_resource() {
    let original = runtime(shell_launcher_snapshot());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install.clone(), &approval)
        .await
        .unwrap();
    let launcher = command_path_for_name(&original.context().bin_dir, "demo".as_ref());
    let old_kind = original.host().metadata(&launcher).await.unwrap().kind;
    let old_target = original.host().read_link(&launcher).await.ok();
    let old_bytes = original.host().read(&launcher).await.ok();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        shell_bun_launcher_snapshot(),
    );
    let plan = runtime.plan_shells(install.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .install_shells_approved(install, &approval)
        .await
        .err()
        .expect("replacement receipt write should fail");
    let rollback = managed_file_rollback_path(&launcher);
    assert!(runtime.host().metadata(&rollback).await.is_ok());
    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_previous_launcher".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().metadata(&launcher).await.unwrap().kind,
        old_kind
    );
    assert_eq!(runtime.host().read_link(&launcher).await.ok(), old_target);
    assert_eq!(runtime.host().read(&launcher).await.ok(), old_bytes);
    assert!(runtime.host().metadata(&rollback).await.is_err());
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(manifest.find("shell/demo/demo").unwrap().runtime, "native");
}

#[tokio::test]
async fn shell_launcher_update_write_failure_restores_moved_previous_resource() {
    let original = runtime(shell_launcher_snapshot());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install.clone(), &approval)
        .await
        .unwrap();
    let launcher = command_path_for_name(&original.context().bin_dir, "demo".as_ref());
    let old_kind = original.host().metadata(&launcher).await.unwrap().kind;
    let old_target = original.host().read_link(&launcher).await.ok();
    let old_bytes = original.host().read(&launcher).await.ok();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        shell_bun_launcher_snapshot(),
    );
    let plan = runtime.plan_shells(install.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().fail_write_after(&launcher, 0);
    runtime
        .install_shells_approved(install, &approval)
        .await
        .err()
        .expect("replacement launcher write should fail");
    let rollback = managed_file_rollback_path(&launcher);
    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_ok());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().metadata(&launcher).await.unwrap().kind,
        old_kind
    );
    assert_eq!(runtime.host().read_link(&launcher).await.ok(), old_target);
    assert_eq!(runtime.host().read(&launcher).await.ok(), old_bytes);
    assert!(runtime.host().metadata(&rollback).await.is_err());
}

#[tokio::test]
async fn shell_launcher_update_rename_failure_recovers_not_started_state() {
    let original = runtime(shell_launcher_snapshot());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install.clone(), &approval)
        .await
        .unwrap();
    let launcher = command_path_for_name(&original.context().bin_dir, "demo".as_ref());
    let old_target = original.host().read_link(&launcher).await.ok();
    let old_bytes = original.host().read(&launcher).await.ok();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        shell_bun_launcher_snapshot(),
    );
    let rollback = managed_file_rollback_path(&launcher);
    let plan = runtime.plan_shells(install.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().fail_rename_after(&launcher, &rollback, 0);
    runtime
        .install_shells_approved(install, &approval)
        .await
        .err()
        .expect("moving old launcher should fail");
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().metadata(&rollback).await.is_err());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_launcher_update_not_started".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read_link(&launcher).await.ok(), old_target);
    assert_eq!(runtime.host().read(&launcher).await.ok(), old_bytes);
}

#[tokio::test]
async fn shell_launcher_update_commit_failure_cleans_only_exact_rollback() {
    let original = runtime(shell_launcher_snapshot());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install.clone(), &approval)
        .await
        .unwrap();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        shell_bun_launcher_snapshot(),
    );
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let rollback = managed_file_rollback_path(&launcher);
    let plan = runtime.plan_shells(install.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().fail_remove_after(&rollback, 0);
    runtime
        .install_shells_approved(install, &approval)
        .await
        .err()
        .expect("rollback cleanup should fail");
    let replacement = runtime.host().read(&launcher).await.unwrap();
    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_cleanup_launcher_rollback".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&launcher).await.unwrap(), replacement);
    assert!(runtime.host().metadata(&rollback).await.is_err());
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(manifest.find("shell/demo/demo").unwrap().runtime, "bun");
}

#[tokio::test]
async fn shell_launcher_update_recovery_blocks_modified_replacement() {
    let original = runtime(shell_launcher_snapshot());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install.clone(), &approval)
        .await
        .unwrap();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        shell_bun_launcher_snapshot(),
    );
    let plan = runtime.plan_shells(install.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .install_shells_approved(install, &approval)
        .await
        .err()
        .expect("replacement receipt write should fail");
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let rollback = managed_file_rollback_path(&launcher);
    runtime
        .host()
        .put_file(&launcher, b"#!/bin/sh\necho user change\n".to_vec());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(!recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_launcher_update_changed".to_string())
    }));
    assert_eq!(
        runtime.host().read(&launcher).await.unwrap(),
        b"#!/bin/sh\necho user change\n"
    );
    assert!(runtime.host().metadata(&rollback).await.is_ok());
}

#[tokio::test]
async fn occupied_shell_launcher_rollback_blocks_before_update() {
    let original = runtime(shell_launcher_snapshot());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install.clone(), &approval)
        .await
        .unwrap();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        shell_bun_launcher_snapshot(),
    );
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let rollback = managed_file_rollback_path(&launcher);
    runtime.host().put_file(&rollback, b"user-owned".to_vec());

    let plan = runtime.plan_shells(install).await.unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_launcher_rollback_occupied".to_string())
    }));
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), b"user-owned");
}

fn shell_uninstall_request() -> ShellPlanRequest {
    ShellPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo/demo".to_string()),
        force: false,
        purge: false,
        input_versions: PlanningInputVersions::default(),
    }
}

async fn installed_shell_runtime() -> CoreRuntime<InMemoryHost> {
    let runtime = runtime(shell_launcher_snapshot());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&runtime.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    runtime
        .install_shells_approved(install, &approval)
        .await
        .unwrap();
    runtime
}

#[tokio::test]
async fn approved_shell_launcher_removal_moves_before_receipt_commit_and_cleans_transaction() {
    let runtime = installed_shell_runtime().await;
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_managed_launcher_remove_transaction".to_string())
    }));
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let rollback = managed_file_rollback_path(&launcher);
    for (access, path) in [
        (FilesystemAccessV1::Remove, &launcher),
        (FilesystemAccessV1::Write, &rollback),
        (FilesystemAccessV1::Remove, &rollback),
    ] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: review_path(runtime.context(), path),
                })
        );
    }
    let operation_offset = runtime.host().operations().len();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .unwrap();

    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_err());
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.find("shell/demo/demo").is_none());
    let journal = runtime
        .context()
        .shine_dir
        .join(super::super::SHELL_OPERATION_JOURNAL_FILE);
    assert!(runtime.host().metadata(&journal).await.is_err());

    let operations = runtime.host().operations();
    let operations = &operations[operation_offset..];
    let journal_write = operations
        .iter()
        .position(|operation| matches!(operation, HostOperation::Write(path) if path == &journal))
        .unwrap();
    let launcher_move = operations
        .iter()
        .position(|operation| matches!(operation, HostOperation::Remove(path) if path == &launcher))
        .unwrap();
    let receipt_write = operations
            .iter()
            .position(|operation| matches!(operation, HostOperation::Write(path) if path == &runtime.context().shine_dir.join("shell-manifest.toml")))
            .unwrap();
    let receipt_commit_marker = operations
        .iter()
        .rposition(|operation| matches!(operation, HostOperation::Write(path) if path == &journal))
        .unwrap();
    let rollback_cleanup = operations
        .iter()
        .position(|operation| matches!(operation, HostOperation::Remove(path) if path == &rollback))
        .unwrap();
    assert!(journal_write < launcher_move);
    assert!(launcher_move < receipt_write);
    assert!(receipt_write < receipt_commit_marker);
    assert!(receipt_commit_marker < rollback_cleanup);
}

#[tokio::test]
async fn shell_launcher_removal_receipt_failure_restores_moved_launcher() {
    let runtime = installed_shell_runtime().await;
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let rollback = managed_file_rollback_path(&launcher);
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt removal should fail");
    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_ok());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_removed_launcher".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().metadata(&rollback).await.is_err());
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.find("shell/demo/demo").is_some());
}

#[cfg(unix)]
#[tokio::test]
async fn legacy_shell_launcher_manifest_failure_restores_without_creating_a_receipt() {
    let runtime = runtime(shell_launcher_snapshot());
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let source = runtime.context().presets_dir.join("shell/demo/demo.sh");
    runtime.host().symlink(&source, &launcher).await.unwrap();
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_legacy_launcher_remove_transaction".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let rollback = managed_file_rollback_path(&launcher);
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell manifest write should fail");
    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_ok());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_removed_legacy_launcher".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().metadata(&rollback).await.is_err());
    assert!(
        ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .find("shell/demo/demo")
            .is_none()
    );
}

#[tokio::test]
async fn shell_launcher_removal_marker_failure_reconstructs_receipt_before_restore() {
    let runtime = installed_shell_runtime().await;
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let rollback = managed_file_rollback_path(&launcher);
    let journal = runtime
        .context()
        .shine_dir
        .join(super::super::SHELL_OPERATION_JOURNAL_FILE);
    runtime.host().fail_write_after(&journal, 3);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt commit marker should fail");
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.find("shell/demo/demo").is_none());
    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_ok());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_restore_removed_launcher_receipt".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.find("shell/demo/demo").is_some());
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().metadata(&rollback).await.is_err());
}

#[tokio::test]
async fn shell_launcher_removal_commit_failure_cleans_only_exact_rollback() {
    let runtime = installed_shell_runtime().await;
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let rollback = managed_file_rollback_path(&launcher);
    runtime.host().fail_remove_after(&rollback, 0);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell launcher rollback cleanup should fail");
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.find("shell/demo/demo").is_none());
    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_ok());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_cleanup_removed_launcher".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().metadata(&rollback).await.is_err());
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.find("shell/demo/demo").is_none());
}

#[tokio::test]
async fn shell_launcher_removal_recovery_blocks_modified_rollback_material() {
    let runtime = installed_shell_runtime().await;
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let rollback = managed_file_rollback_path(&launcher);
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("shell-manifest.toml"), 0);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("Shell receipt removal should fail");
    runtime
        .host()
        .put_file(&rollback, b"user-changed rollback".to_vec());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(!recovery.is_ready());
    assert!(recovery.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_recovery_launcher_removal_changed".to_string())
    }));
    assert_eq!(
        runtime.host().read(&rollback).await.unwrap(),
        b"user-changed rollback"
    );
}

#[tokio::test]
async fn occupied_shell_launcher_rollback_blocks_before_removal() {
    let runtime = installed_shell_runtime().await;
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let rollback = managed_file_rollback_path(&launcher);
    runtime.host().put_file(&rollback, b"user-owned".to_vec());

    let plan = runtime
        .plan_shells(shell_uninstall_request())
        .await
        .unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_launcher_rollback_occupied".to_string())
    }));
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), b"user-owned");
}

#[tokio::test]
async fn approved_shell_uninstall_preserves_a_modified_receipt_owned_launcher() {
    let runtime = installed_shell_runtime().await;
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    runtime
        .host()
        .put_file(&launcher, b"#!/bin/sh\necho user-owned\n".to_vec());
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Preserve
            && step
                .diagnostic_codes
                .contains(&"shell_foreign_launcher_preserved".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let report = runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .unwrap();

    assert_eq!(
        runtime.host().read(&launcher).await.unwrap(),
        b"#!/bin/sh\necho user-owned\n"
    );
    assert!(report.lifecycle.outcomes.iter().any(|outcome| {
        outcome.status == crate::lifecycle::LifecycleStatus::Conflict
            && outcome
                .diagnostic_codes
                .contains(&"shell_command_conflict".to_string())
    }));
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.find("shell/demo/demo").is_none());
}

#[tokio::test]
async fn approved_shell_upgrade_uses_managed_launcher_update_transaction() {
    let original = runtime(shell_launcher_snapshot());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install, &approval)
        .await
        .unwrap();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        shell_bun_launcher_snapshot(),
    );
    let request = ShellPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_managed_launcher_update_transaction".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .upgrade_shells_approved(request, &approval)
        .await
        .unwrap();
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(manifest.find("shell/demo/demo").unwrap().runtime, "bun");
}

#[cfg(not(unix))]
#[tokio::test]
async fn shell_plan_binds_both_windows_launcher_resources() {
    let runtime = runtime(shell_launcher_snapshot());
    let plan = runtime.plan_shells(shell_install_request()).await.unwrap();
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let cmd = launcher.with_extension("cmd");
    for destination in [launcher, cmd] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access: FilesystemAccessV1::Write,
                    path: review_path(runtime.context(), &destination),
                })
        );
    }
}

#[cfg(not(unix))]
#[tokio::test]
async fn shell_recovery_removes_a_partial_windows_shim_pair() {
    let runtime = runtime(shell_launcher_snapshot());
    let request = shell_install_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let cmd = launcher.with_extension("cmd");
    runtime.host().fail_write_after(&cmd, 0);
    runtime
        .install_shells_approved(request, &approval)
        .await
        .err()
        .expect("second Windows shim write should fail");
    assert!(runtime.host().metadata(&launcher).await.is_ok());
    assert!(runtime.host().metadata(&cmd).await.is_err());

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert!(runtime.host().metadata(&launcher).await.is_err());
    assert!(runtime.host().metadata(&cmd).await.is_err());
}

#[cfg(not(unix))]
#[tokio::test]
async fn shell_recovery_restores_a_partial_windows_launcher_update() {
    let original = runtime(shell_launcher_snapshot());
    let install = shell_install_request();
    let approval =
        PlanApprovalV1::for_reviewed_plan(&original.plan_shells(install.clone()).await.unwrap())
            .unwrap();
    original
        .install_shells_approved(install.clone(), &approval)
        .await
        .unwrap();
    let launcher = command_path_for_name(&original.context().bin_dir, "demo".as_ref());
    let cmd = launcher.with_extension("cmd");
    let old_ps1 = original.host().read(&launcher).await.unwrap();
    let old_cmd = original.host().read(&cmd).await.unwrap();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        shell_bun_launcher_snapshot(),
    );
    let plan = runtime.plan_shells(install.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().fail_write_after(&cmd, 0);
    runtime
        .install_shells_approved(install, &approval)
        .await
        .err()
        .expect("second replacement shim write should fail");

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&launcher).await.unwrap(), old_ps1);
    assert_eq!(runtime.host().read(&cmd).await.unwrap(), old_cmd);
}

#[cfg(not(unix))]
#[tokio::test]
async fn shell_recovery_restores_a_partial_windows_launcher_removal() {
    let runtime = installed_shell_runtime().await;
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let cmd = launcher.with_extension("cmd");
    let launcher_bytes = runtime.host().read(&launcher).await.unwrap();
    let cmd_bytes = runtime.host().read(&cmd).await.unwrap();
    let cmd_rollback = managed_file_rollback_path(&cmd);
    runtime.host().fail_rename_after(&cmd, &cmd_rollback, 0);
    runtime
        .uninstall_shells_approved(request, &approval)
        .await
        .err()
        .expect("second Windows launcher move should fail");

    let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
    assert!(recovery.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&recovery).unwrap();
    runtime
        .recover_shell_operation_approved(&approval)
        .await
        .unwrap();
    assert_eq!(
        runtime.host().read(&launcher).await.unwrap(),
        launcher_bytes
    );
    assert_eq!(runtime.host().read(&cmd).await.unwrap(), cmd_bytes);
    assert!(runtime.host().metadata(&cmd_rollback).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn shell_overlay_launcher_ownership_matches_inspection_and_plan() {
    for retain_overlay in [true, false] {
        let original = installed_shell_runtime().await;
        let mut context = original.context().clone();
        let overlay = context.home_dir.join("overlay");
        context.overlay_dir = retain_overlay.then_some(overlay.clone());
        let source = overlay.join("shell/demo/demo.sh");
        let (mut manifest, _) = load_shell_manifest(original.host(), &context.shine_dir)
            .await
            .unwrap();
        manifest.entries[0].source_path = source.clone();
        original.host().put_file(
            context.shine_dir.join("shell-manifest.toml"),
            toml::to_string(&manifest).unwrap().into_bytes(),
        );
        let launcher = command_path_for_name(&context.bin_dir, "demo".as_ref());
        original.host().remove_file(&launcher).await.unwrap();
        original.host().put_file(
            &launcher,
            format!(
                "#!/bin/sh\n# shine-managed\n# shine-target: {}\n",
                source.display()
            )
            .into_bytes(),
        );
        let runtime = CoreRuntime::new(original.host().clone(), context, shell_launcher_snapshot());
        assert!(!runtime.inspect_shells().await.unwrap()[0].link_conflict);
        let plan = runtime
            .plan_shells(ShellPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: None,
                ..shell_install_request()
            })
            .await
            .unwrap();
        assert!(plan.is_ready(), "{plan:?}");
        assert!(!plan.steps.iter().any(|step| {
            step.diagnostic_codes
                .contains(&"shell_foreign_launcher_conflict".to_string())
        }));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn shell_legacy_symlink_repair_stays_inside_managed_roots() {
    for managed in [true, false] {
        let runtime = installed_shell_runtime().await;
        let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
        let target = if managed {
            runtime.context().shine_dir.join("legacy.sh")
        } else {
            runtime.context().home_dir.join("user.sh")
        };
        runtime.host().put_file(&target, b"#!/bin/sh\n".to_vec());
        runtime.host().remove_file(&launcher).await.unwrap();
        runtime.host().symlink(&target, &launcher).await.unwrap();
        assert_eq!(
            runtime.inspect_shells().await.unwrap()[0].link_conflict,
            !managed
        );
        let plan = runtime
            .plan_shells(ShellPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: Some("demo".to_string()),
                ..shell_install_request()
            })
            .await
            .unwrap();
        assert_eq!(plan.is_ready(), managed, "{plan:?}");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn shell_missing_preset_is_preserved_and_can_be_uninstalled() {
    let original = installed_shell_runtime().await;
    let mut context = original.context().clone();
    context.is_external_presets = true;
    let runtime = CoreRuntime::new(
        original.host().clone(),
        context,
        PresetSnapshot::builder(PresetSourceKind::Embedded).build(),
    );
    let inspection = runtime.inspect_shells().await.unwrap();
    assert_eq!(inspection.len(), 1);
    assert!(inspection[0].preset_missing);
    assert!(!inspection[0].link_conflict);
    let launcher = inspection[0].link_path.clone();
    let before = runtime.host().read_link(&launcher).await.unwrap();
    let request = ShellPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo/demo".to_string()),
        ..shell_install_request()
    };
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready(), "{plan:?}");
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Preserve
            && step
                .diagnostic_codes
                .contains(&"shell_preset_missing".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .upgrade_shells_approved(request, &approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read_link(&launcher).await.unwrap(), before);
    assert_eq!(
        load_shell_manifest(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .0
            .entries
            .len(),
        1
    );
    let request = shell_uninstall_request();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready(), "{plan:?}");
    runtime
        .uninstall_shells_approved(request, &PlanApprovalV1::for_reviewed_plan(&plan).unwrap())
        .await
        .unwrap();
    assert!(runtime.host().metadata(&launcher).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn shell_missing_preset_blocks_shared_snapshot_replacement() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file("shell/demo/shine.toml", b"[[files]]\nsource = 'one.sh'\n[files.permissions]\nschema_version = 1\n[[files]]\nsource = 'two.sh'\n[files.permissions]\nschema_version = 1\n".to_vec())
            .file("shell/demo/one.sh", b"#!/bin/sh\necho one\n".to_vec())
            .file("shell/demo/two.sh", b"#!/bin/sh\necho two\n".to_vec()).build();
    let original = external_shell_runtime(snapshot);
    for (logical, bytes) in original.presets().files() {
        original
            .host()
            .put_file(original.context().presets_dir.join(logical), bytes.clone());
    }
    let install = ShellPlanRequest {
        target: Some("demo".to_string()),
        ..shell_install_request()
    };
    let plan = original.plan_shells(install.clone()).await.unwrap();
    original
        .install_shells_approved(install, &PlanApprovalV1::for_reviewed_plan(&plan).unwrap())
        .await
        .unwrap();
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file(
            "shell/demo/shine.toml",
            b"[[files]]\nsource = 'two.sh'\n[files.permissions]\nschema_version = 1\n".to_vec(),
        )
        .file("shell/demo/two.sh", b"#!/bin/sh\necho changed\n".to_vec())
        .build();
    let mut runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        snapshot,
    );
    trust_current_external_shell(&mut runtime);
    for (logical, bytes) in runtime.presets().files() {
        runtime
            .host()
            .put_file(runtime.context().presets_dir.join(logical), bytes.clone());
    }
    runtime
        .host()
        .remove_file(&runtime.context().presets_dir.join("shell/demo/one.sh"))
        .await
        .unwrap();
    let request = ShellPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: Some("demo".to_string()),
        ..shell_install_request()
    };
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.diagnostic_codes
            .contains(&"shell_snapshot_contains_missing_preset".to_string())
    }));
    let uninstall = ShellPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo/one".to_string()),
        ..shell_install_request()
    };
    let plan = runtime.plan_shells(uninstall.clone()).await.unwrap();
    assert!(plan.is_ready(), "{plan:?}");
    runtime
        .uninstall_shells_approved(
            uninstall,
            &PlanApprovalV1::for_reviewed_plan(&plan).unwrap(),
        )
        .await
        .unwrap();
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(plan.is_ready(), "{plan:?}");
    runtime
        .upgrade_shells_approved(request, &PlanApprovalV1::for_reviewed_plan(&plan).unwrap())
        .await
        .unwrap();
}

#[tokio::test]
async fn shell_missing_preset_foreign_launcher_remains_blocked() {
    let original = installed_shell_runtime().await;
    let runtime = CoreRuntime::new(
        original.host().clone(),
        original.context().clone(),
        PresetSnapshot::builder(PresetSourceKind::Embedded).build(),
    );
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    runtime.host().remove_file(&launcher).await.unwrap();
    runtime.host().put_file(&launcher, b"user owned".to_vec());
    let rows = runtime.inspect_shells().await.unwrap();
    assert!(rows[0].preset_missing && rows[0].link_conflict);
    let plan = runtime
        .plan_shells(ShellPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: None,
            ..shell_install_request()
        })
        .await
        .unwrap();
    assert!(!plan.is_ready());
    assert_eq!(runtime.host().read(&launcher).await.unwrap(), b"user owned");
}

#[tokio::test]
async fn foreign_shell_launcher_blocks_without_mutation() {
    if !RuntimePlatform::current().is_unix() {
        return;
    }
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file("shell/demo/shine.toml", b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n[files.permissions]\nschema_version = 1\n".to_vec())
            .file("shell/demo/demo.sh", b"#!/bin/sh\n".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let launcher = command_path_for_name(&runtime.context().bin_dir, "demo".as_ref());
    let request = ShellPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo/demo".to_string()),
        force: false,
        purge: false,
        input_versions: PlanningInputVersions::default(),
    };
    let missing = runtime.plan_shells(request.clone()).await.unwrap();
    runtime
        .host()
        .put_file(&launcher, b"#!/bin/sh\necho user\n".to_vec());
    let plan = runtime.plan_shells(request.clone()).await.unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"shell_foreign_launcher_conflict".to_string())
    }));
    assert_ne!(missing.inputs.state, plan.inputs.state);
    let forced = runtime
        .plan_shells(ShellPlanRequest {
            force: true,
            ..request
        })
        .await
        .unwrap();
    assert!(forced.is_ready());
    assert!(forced.steps.iter().any(|step| {
        step.action == PlanActionV1::Update
            && step
                .diagnostic_codes
                .contains(&"shell_foreign_launcher_override".to_string())
    }));
    assert_ne!(plan.fingerprint().unwrap(), forced.fingerprint().unwrap());
}

#[tokio::test]
async fn targeted_shell_uninstall_preserves_shared_category_state_and_updates_profile() {
    if !RuntimePlatform::current().is_unix() {
        return;
    }
    let runtime = runtime(PresetSnapshot::builder(PresetSourceKind::Embedded).build());
    let source_root = runtime.context().shine_dir.join("installed/shell/demo");
    let entries = ["one", "two"]
        .into_iter()
        .map(|command| ShellManifestEntry {
            launcher_format: None,
            launcher_config_dir: None,

            category: "demo".to_string(),
            command: command.to_string(),
            mode: ExternalShellMode::Snapshot,
            source_path: source_root.join(format!("{command}.sh")),
            rendered_path: runtime
                .context()
                .shine_dir
                .join(format!("rendered/shell/demo/{command}.sh")),
            runtime: "native".to_string(),
            bun_dependencies: None,
            dependency_hash: None,
            transforms: Vec::new(),
            env: Vec::new(),
            needs_source: true,
            content_hash: 1,
        })
        .collect::<Vec<_>>();
    runtime.host().put_file(
        runtime.context().shine_dir.join("shell-manifest.toml"),
        toml::to_string(&ShellManifest {
            schema_version: super::super::SHELL_MANIFEST_SCHEMA_VERSION,
            entries,
        })
        .unwrap()
        .into_bytes(),
    );
    let launcher = command_path_for_name(&runtime.context().bin_dir, "one".as_ref());
    runtime.host().put_file(
        launcher,
        format!(
            "#!/bin/sh\n# shine-managed\n# shine-target:{}\n",
            source_root.join("one.sh").display()
        )
        .into_bytes(),
    );

    let plan = runtime
        .plan_shells(ShellPlanRequest {
            operation: LifecycleOperation::Uninstall,
            target: Some("demo/one".to_string()),
            force: false,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(
        plan.steps
            .iter()
            .any(|step| { step.target == "shell/profile" && step.action == PlanActionV1::Update })
    );
    assert!(!plan.steps.iter().any(|step| {
        step.resource.as_deref() == Some("shared-category-state")
            && step.action == PlanActionV1::Remove
    }));
}

#[tokio::test]
async fn managed_sys_uninstall_can_use_receipt_without_original_preset() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file("app/placeholder/file", b"placeholder".to_vec())
        .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/managed.txt");
    runtime.host().put_file(&destination, b"managed".to_vec());
    let manifest = SysRunManifest {
        schema_version: super::super::SYS_MANIFEST_SCHEMA_VERSION,
        entries: vec![SysRunEntry {
            os_id: "test".to_string(),
            item_id: "managed".to_string(),
            label: "Managed".to_string(),
            status: super::super::SysItemStatus::Installed,
            detail: String::new(),
            updated_at: "1".to_string(),
            managed: true,
            profile_enabled: false,
            receipt: Some(SystemReceipt::ManagedFile(
                super::super::ManagedFileReceipt {
                    version: super::super::RECEIPT_VERSION,
                    destination: destination.clone(),
                    backup: None,
                    content_hash: crate::install::hash_content(b"managed"),
                    privileged: false,
                    restart_hint: None,
                },
            )),
        }],
    };
    runtime.host().put_file(
        runtime.context().shine_dir.join("sys-manifest.toml"),
        toml::to_string(&manifest).unwrap().into_bytes(),
    );
    let plan = runtime
        .plan_managed_sys(SysManagedPlanRequest {
            operation: LifecycleOperation::Uninstall,
            os_id: "test".to_string(),
            target: Some("managed".to_string()),
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(plan.is_ready());
    assert!(
        plan.steps
            .iter()
            .any(|step| step.action == PlanActionV1::Remove)
    );
}

#[tokio::test]
async fn managed_sys_manifest_failure_restores_file_and_receipt_on_explicit_recovery() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file("app/placeholder/file", b"placeholder".to_vec())
        .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/managed.txt");
    let rollback = crate::action::managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, b"managed".to_vec());
    let previous_entry = SysRunEntry {
        os_id: "test".to_string(),
        item_id: "managed".to_string(),
        label: "Managed".to_string(),
        status: super::super::SysItemStatus::Installed,
        detail: String::new(),
        updated_at: "1".to_string(),
        managed: true,
        profile_enabled: false,
        receipt: Some(SystemReceipt::ManagedFile(
            super::super::ManagedFileReceipt {
                version: super::super::RECEIPT_VERSION,
                destination: destination.clone(),
                backup: None,
                content_hash: crate::install::hash_content(b"managed"),
                privileged: false,
                restart_hint: None,
            },
        )),
    };
    runtime.host().put_file(
        runtime.context().shine_dir.join("sys-manifest.toml"),
        toml::to_string(&SysRunManifest {
            schema_version: super::super::SYS_MANIFEST_SCHEMA_VERSION,
            entries: vec![previous_entry.clone()],
        })
        .unwrap()
        .into_bytes(),
    );
    let request = SysManagedPlanRequest {
        operation: LifecycleOperation::Uninstall,
        os_id: "test".to_string(),
        target: Some("managed".to_string()),
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_managed_sys(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("sys-manifest.toml"), 0);
    let mut observer = super::super::NullObserver;
    assert!(
        runtime
            .run_managed_sys_approved(request, &approval, &mut Interaction, &mut observer,)
            .await
            .is_err()
    );
    assert!(runtime.host().read(&destination).await.is_err());
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), b"managed");
    assert!(
        runtime
            .host()
            .read(
                &runtime
                    .context()
                    .shine_dir
                    .join(super::super::SYS_OPERATION_JOURNAL_FILE)
            )
            .await
            .is_ok()
    );

    let recovery_plan = runtime.plan_sys_operation_recovery().await.unwrap();
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::Sys, true).await;
    assert!(recovery_plan.is_ready());
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_sys_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), b"managed");
    assert!(runtime.host().read(&rollback).await.is_err());
    let manifest = SysRunManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(manifest.entries, vec![previous_entry]);
}

#[tokio::test]
async fn split_dns_manifest_failure_removes_created_resource_on_explicit_recovery() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "sys/macos/shine.toml",
                br#"version = 2
[[items]]
id = 'split-dns'
label = 'Split DNS'
mode = 'managed'
driver = 'split-dns'
requires_admin = true
required_env = ['PRIVATE_DNS_DOMAIN', 'PRIVATE_DNS_SERVERS']
permissions = { schema_version = 1, administrator = true, environment = [{ name = 'PRIVATE_DNS_DOMAIN', sensitivity = 'plain' }, { name = 'PRIVATE_DNS_SERVERS', sensitivity = 'plain' }], system = [{ capability = 'split-dns', resource = 'private-domain' }] }
[items.config]
domain_env = 'PRIVATE_DNS_DOMAIN'
servers_env = 'PRIVATE_DNS_SERVERS'
"#
                .to_vec(),
            )
            .build();
    let mut runtime = runtime(snapshot);
    runtime
        .context_mut_for_cli()
        .env
        .insert("PRIVATE_DNS_DOMAIN".to_string(), "corp.test".to_string());
    runtime
        .context_mut_for_cli()
        .env
        .insert("PRIVATE_DNS_SERVERS".to_string(), "10.0.0.53".to_string());
    let request = SysManagedPlanRequest {
        operation: LifecycleOperation::Install,
        os_id: "macos".to_string(),
        target: Some("split-dns".to_string()),
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_managed_sys(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("sys-manifest.toml"), 0);
    let mut observer = super::super::NullObserver;
    assert!(
        runtime
            .run_managed_sys_approved(request, &approval, &mut Interaction, &mut observer,)
            .await
            .is_err()
    );
    let receipt = split_dns_receipt(&super::super::SplitDnsDomainRequest {
        os_id: "macos".to_string(),
        item_id: "split-dns".to_string(),
        domain: "corp.test".to_string(),
        servers: "10.0.0.53".to_string(),
        dry_run: false,
    })
    .unwrap();
    let resource = PathBuf::from(receipt.resource);
    assert!(runtime.host().read(&resource).await.is_ok());

    let recovery_plan = runtime.plan_sys_operation_recovery().await.unwrap();
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::Sys, true).await;
    assert!(recovery_plan.is_ready());
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_sys_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert!(runtime.host().read(&resource).await.is_err());
    assert!(
        SysRunManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap()
            .entries
            .is_empty()
    );
}

#[tokio::test]
async fn managed_sys_missing_env_blocks_and_admin_requirement_is_explicit() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "sys/test/shine.toml",
                br#"version = 2
[[items]]
id = 'managed'
label = 'Managed'
mode = 'managed'
driver = 'managed-file'
requires_admin = true
required_env = ['TOKEN']
permissions = { schema_version = 1, administrator = true, environment = [{ name = 'TOKEN', sensitivity = 'plain' }] }
[items.config]
source = 'managed.txt'
target = '$HOME/.config/managed.txt'
"#
                .to_vec(),
            )
            .file("sys/test/managed.txt", b"managed".to_vec())
            .build();
    let mut runtime = runtime(snapshot);
    let request = SysManagedPlanRequest {
        operation: LifecycleOperation::Install,
        os_id: "test".to_string(),
        target: Some("managed".to_string()),
        input_versions: PlanningInputVersions::default(),
    };

    let missing = runtime.plan_managed_sys(request.clone()).await.unwrap();
    assert!(!missing.is_ready());
    assert!(missing.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"sys_missing_required_env".to_string())
    }));
    assert!(
        missing
            .permissions
            .required
            .contains(&PermissionV1::Administrator)
    );

    runtime
        .context_mut_for_cli()
        .env
        .insert("TOKEN".to_string(), "plain-value".to_string());
    let ready = runtime.plan_managed_sys(request).await.unwrap();
    assert!(ready.is_ready());
    assert_ne!(missing.inputs.state, ready.inputs.state);
    assert!(
        !serde_json::to_string(&ready)
            .unwrap()
            .contains("plain-value")
    );
}

#[tokio::test]
async fn split_dns_plan_binds_receipt_and_live_ownership() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "sys/macos/shine.toml",
                br#"version = 2
[[items]]
id = 'split-dns'
label = 'Split DNS'
mode = 'managed'
driver = 'split-dns'
requires_admin = true
required_env = ['PRIVATE_DNS_DOMAIN', 'PRIVATE_DNS_SERVERS']
permissions = { schema_version = 1, administrator = true, environment = [{ name = 'PRIVATE_DNS_DOMAIN', sensitivity = 'plain' }, { name = 'PRIVATE_DNS_SERVERS', sensitivity = 'plain' }], system = [{ capability = 'split-dns', resource = 'private-domain' }] }
[items.config]
domain_env = 'PRIVATE_DNS_DOMAIN'
servers_env = 'PRIVATE_DNS_SERVERS'
"#
                .to_vec(),
            )
            .build();
    let mut runtime = runtime(snapshot);
    runtime
        .context_mut_for_cli()
        .env
        .insert("PRIVATE_DNS_DOMAIN".to_string(), "corp.test".to_string());
    runtime
        .context_mut_for_cli()
        .env
        .insert("PRIVATE_DNS_SERVERS".to_string(), "10.0.0.53".to_string());
    let receipt = split_dns_receipt(&super::super::SplitDnsDomainRequest {
        os_id: "macos".to_string(),
        item_id: "split-dns".to_string(),
        domain: "corp.test".to_string(),
        servers: "10.0.0.53".to_string(),
        dry_run: true,
    })
    .unwrap();
    let resource = PathBuf::from(&receipt.resource);
    runtime
        .host()
        .put_file(&resource, split_dns_content_for_plan(&receipt));
    runtime.host().put_file(
        runtime.context().shine_dir.join("sys-manifest.toml"),
        toml::to_string(&SysRunManifest {
            schema_version: super::super::SYS_MANIFEST_SCHEMA_VERSION,
            entries: vec![SysRunEntry {
                os_id: "macos".to_string(),
                item_id: "split-dns".to_string(),
                label: "Split DNS".to_string(),
                status: super::super::SysItemStatus::Installed,
                detail: String::new(),
                updated_at: "1".to_string(),
                managed: true,
                profile_enabled: false,
                receipt: Some(SystemReceipt::SplitDns(receipt)),
            }],
        })
        .unwrap()
        .into_bytes(),
    );
    let request = SysManagedPlanRequest {
        operation: LifecycleOperation::Install,
        os_id: "macos".to_string(),
        target: Some("split-dns".to_string()),
        input_versions: PlanningInputVersions::default(),
    };
    let current = runtime.plan_managed_sys(request.clone()).await.unwrap();
    assert!(current.is_ready());
    assert!(
        current
            .steps
            .iter()
            .any(|step| step.action == PlanActionV1::None)
    );

    assert!(current.permissions.required.is_empty());
    assert!(!current.author_capabilities.is_empty());

    runtime
        .context_mut_for_cli()
        .env
        .insert("PRIVATE_DNS_SERVERS".into(), "10.0.0.54".into());
    let update = runtime.plan_managed_sys(request.clone()).await.unwrap();
    assert!(
        update
            .steps
            .iter()
            .any(|step| step.action == PlanActionV1::Update)
    );
    assert!(
        update
            .permissions
            .required
            .contains(&PermissionV1::Administrator)
    );
    assert!(
        update
            .permissions
            .required
            .iter()
            .any(|p| matches!(p, PermissionV1::System { .. }))
    );
    assert_ne!(
        current.fingerprint().unwrap(),
        update.fingerprint().unwrap()
    );
    runtime
        .context_mut_for_cli()
        .env
        .insert("PRIVATE_DNS_SERVERS".into(), "10.0.0.53".into());

    runtime.host().put_file(&resource, b"foreign".to_vec());
    let conflicted = runtime.plan_managed_sys(request).await.unwrap();
    assert!(conflicted.steps.iter().any(|step| {
        step.action == PlanActionV1::Preserve
            && step
                .diagnostic_codes
                .contains(&"sys_resource_user_modified".to_string())
    }));
    assert_ne!(current.inputs.state, conflicted.inputs.state);
}

#[tokio::test]
async fn invalid_operation_flag_combinations_are_rejected() {
    let runtime = runtime(PresetSnapshot::builder(PresetSourceKind::Embedded).build());
    let app = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Install,
            target: None,
            force: false,
            purge: true,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await;
    assert!(app.is_err());
    let shell = runtime
        .plan_shells(ShellPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: None,
            force: true,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await;
    assert!(shell.is_err());
}

#[tokio::test]
async fn app_uninstall_can_use_manifest_without_original_preset() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file("app/other/config", b"other".to_vec())
        .build();
    let runtime = runtime(snapshot);
    let destination = runtime
        .context()
        .home_dir
        .join(".config/retired/config.toml");
    runtime.host().put_file(&destination, b"installed".to_vec());
    let manifest = AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/retired/config.toml".to_string(),
            destination,
            backup: None,
            content_hash: crate::install::hash_content(b"installed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    };
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&manifest).unwrap().into_bytes(),
    );

    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Uninstall,
            target: Some("retired".to_string()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(plan.is_ready());
    assert!(
        plan.steps
            .iter()
            .any(|step| step.action == PlanActionV1::Remove)
    );
    assert!(!plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Execute
            && step
                .diagnostic_codes
                .contains(&"app_teardown_execution".to_string())
    }));
}

#[tokio::test]
async fn approved_app_upgrade_prune_journals_stale_static_copy_removal() {
    let runtime = runtime(PresetSnapshot::builder(PresetSourceKind::Embedded).build());
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let rollback = crate::action::managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, b"managed".to_vec());
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination: destination.clone(),
            backup: None,
            content_hash: crate::install::hash_content(b"managed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: None,
        force: false,
        purge: false,
        prune_stale: true,
        input_versions: PlanningInputVersions::default(),
    };

    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Remove
            && step
                .diagnostic_codes
                .contains(&"app_stale_source_pruned".to_string())
    }));
    for (access, path) in [
        (FilesystemAccessV1::Remove, destination.clone()),
        (FilesystemAccessV1::Write, rollback.clone()),
        (FilesystemAccessV1::Remove, rollback.clone()),
    ] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: review_path(runtime.context(), &path),
                })
        );
    }
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::RemoveManagedFile { .. }))
    ));

    let mut observer = super::super::NullObserver;
    let report = runtime
        .upgrade_apps_approved(
            request,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();
    assert_eq!(report.lifecycle.summary().changed, 1);
    assert!(runtime.host().read(&destination).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
    assert!(
        runtime
            .host()
            .read(
                &runtime
                    .context()
                    .shine_dir
                    .join(super::super::APP_OPERATION_JOURNAL_FILE)
            )
            .await
            .is_err()
    );
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.entries.is_empty());
}

#[tokio::test]
async fn app_upgrade_prune_blocks_stale_copy_with_noncanonical_backup() {
    let runtime = runtime(PresetSnapshot::builder(PresetSourceKind::Embedded).build());
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let backup = runtime
        .context()
        .home_dir
        .join(".config/demo/config.legacy");
    runtime.host().put_file(&destination, b"managed".to_vec());
    runtime.host().put_file(&backup, b"original".to_vec());
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination,
            backup: Some(backup),
            content_hash: crate::install::hash_content(b"managed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();

    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Upgrade,
            target: None,
            force: false,
            purge: false,
            prune_stale: true,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();

    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_stale_removal_unsupported".to_string())
    }));
}

#[tokio::test]
async fn app_upgrade_prune_receipt_failure_recovers_stale_static_copy() {
    let runtime = runtime(PresetSnapshot::builder(PresetSourceKind::Embedded).build());
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let rollback = crate::action::managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, b"managed".to_vec());
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.toml".to_string(),
            destination: destination.clone(),
            backup: None,
            content_hash: crate::install::hash_content(b"managed"),
            install_strategy: crate::install::AppInstallStrategy::Copy,
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: None,
        force: false,
        purge: false,
        prune_stale: true,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("app-manifest.toml"), 0);

    let mut observer = super::super::NullObserver;
    assert!(
        runtime
            .upgrade_apps_approved(
                request,
                &approval,
                AppApprovedUpgradeOptions::default(),
                &mut observer,
                &mut Interaction,
            )
            .await
            .is_err()
    );
    assert!(runtime.host().read(&destination).await.is_err());
    assert_eq!(runtime.host().read(&rollback).await.unwrap(), b"managed");

    let recovery_plan = runtime.plan_app_operation_recovery().await.unwrap();
    assert!(recovery_plan.is_ready());
    let recovery_approval = PlanApprovalV1::for_reviewed_plan(&recovery_plan).unwrap();
    runtime
        .recover_app_operation_approved(&recovery_approval)
        .await
        .unwrap();
    assert_eq!(runtime.host().read(&destination).await.unwrap(), b"managed");
    assert!(runtime.host().read(&rollback).await.is_err());
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(manifest.entries.len(), 1);
}

#[tokio::test]
async fn approved_app_upgrade_prune_journals_stale_json_keys() {
    let runtime = runtime(PresetSnapshot::builder(PresetSourceKind::Embedded).build());
    let destination = runtime.context().home_dir.join(".config/demo/config.json");
    let current = br#"{"proxy":{"mode":"managed"},"theme":"dark"}"#;
    let managed_source = br#"{"proxy":{"mode":"managed"}}"#;
    let managed_keys = vec!["proxy".to_string()];
    runtime.host().put_file(&destination, current.to_vec());
    AppManifest {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        entries: vec![AppEntry {
            source: "app/demo/config.json".to_string(),
            destination: destination.clone(),
            backup: None,
            content_hash: crate::runtime::app::managed_json_hash(managed_source, &managed_keys)
                .unwrap(),
            install_strategy: crate::install::AppInstallStrategy::JsonMerge { managed_keys },
            uses_env: false,
            requires_admin: false,
        }],
    }
    .save(runtime.host(), &runtime.context().shine_dir)
    .await
    .unwrap();
    let request = AppPlanRequest {
        operation: LifecycleOperation::Upgrade,
        target: None,
        force: false,
        purge: false,
        prune_stale: true,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::RemoveManagedJson { .. }))
    ));

    let mut observer = super::super::NullObserver;
    runtime
        .upgrade_apps_approved(
            request,
            &approval,
            AppApprovedUpgradeOptions::default(),
            &mut observer,
            &mut Interaction,
        )
        .await
        .unwrap();
    let remaining = runtime.host().read(&destination).await.unwrap();
    let remaining: serde_json::Value = serde_json::from_slice(&remaining).unwrap();
    assert_eq!(remaining, serde_json::json!({ "theme": "dark" }));
}

#[tokio::test]
async fn approved_app_uninstall_journals_static_managed_removal() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let rollback = crate::action::managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, b"managed".to_vec());
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: vec![AppEntry {
                source: "app/demo/config.toml".to_string(),
                destination: destination.clone(),
                backup: None,
                content_hash: crate::install::hash_content(b"managed"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
        })
        .unwrap()
        .into_bytes(),
    );
    let request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    for access in [FilesystemAccessV1::Write, FilesystemAccessV1::Remove] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: review_path(runtime.context(), &rollback),
                })
        );
    }
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::RemoveManagedFile { .. }))
    ));

    let mut observer = super::super::NullObserver;
    let report = runtime
        .uninstall_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();
    assert_eq!(report.lifecycle.summary().changed, 1);
    assert!(runtime.host().read(&destination).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
    assert!(
        runtime
            .host()
            .read(
                &runtime
                    .context()
                    .shine_dir
                    .join(super::super::APP_OPERATION_JOURNAL_FILE)
            )
            .await
            .is_err()
    );
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.entries.is_empty());
}

#[tokio::test]
async fn approved_app_uninstall_journals_backup_restoration() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    let backup = crate::install::backup_path(&destination);
    let rollback = crate::action::managed_file_rollback_path(&destination);
    runtime.host().put_file(&destination, b"managed".to_vec());
    runtime.host().put_file(&backup, b"user-original".to_vec());
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: vec![AppEntry {
                source: "app/demo/config.toml".to_string(),
                destination: destination.clone(),
                backup: Some(backup.clone()),
                content_hash: crate::install::hash_content(b"managed"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
        })
        .unwrap()
        .into_bytes(),
    );
    let request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    for (access, path) in [
        (FilesystemAccessV1::Write, destination.clone()),
        (FilesystemAccessV1::Remove, backup.clone()),
        (FilesystemAccessV1::Write, rollback.clone()),
        (FilesystemAccessV1::Remove, rollback.clone()),
    ] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: review_path(runtime.context(), &path),
                })
        );
    }
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::RemoveManagedFileWithBackup { .. }))
    ));

    let mut observer = super::super::NullObserver;
    let report = runtime
        .uninstall_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();
    assert_eq!(
        report.files[0].action,
        super::super::AppFileAction::Restored
    );
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user-original"
    );
    assert!(runtime.host().read(&backup).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
    assert!(
        runtime
            .host()
            .read(
                &runtime
                    .context()
                    .shine_dir
                    .join(super::super::APP_OPERATION_JOURNAL_FILE)
            )
            .await
            .is_err()
    );
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.entries.is_empty());
}

#[tokio::test]
async fn approved_privileged_app_uninstall_journals_static_copy_removal() {
    let runtime = runtime(privileged_static_copy_app_snapshot());
    let (destination, _) = seed_privileged_static_copy_app(&runtime, b"managed", None).await;
    let rollback = crate::action::managed_file_rollback_path(&destination);
    let request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(
        plan.permissions
            .required
            .contains(&PermissionV1::Administrator)
    );
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(
            ir.actions.as_slice(),
            [action] if matches!(
                action.kind,
                crate::action::ActionKindV1::RemoveManagedFile {
                    requires_admin: true,
                    ..
                }
            )
        )
    ));

    let mut observer = super::super::NullObserver;
    let report = runtime
        .uninstall_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();
    assert_eq!(report.files[0].action, super::super::AppFileAction::Removed);
    assert!(runtime.host().read(&destination).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
    let operations = runtime.host().operations();
    assert!(operations.contains(&HostOperation::MovePrivileged {
        from: destination,
        to: rollback.clone(),
    }));
    assert!(operations.contains(&HostOperation::RemovePrivileged(rollback)));
}

#[tokio::test]
async fn preserved_privileged_app_file_does_not_request_admin() {
    let runtime = runtime(privileged_static_copy_app_snapshot());
    let (destination, _) = seed_privileged_static_copy_app(&runtime, b"user-modified", None).await;
    let request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo".to_string()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Preserve
            && step
                .diagnostic_codes
                .contains(&"app_user_modified".to_string())
    }));
    assert!(
        !plan
            .permissions
            .required
            .contains(&PermissionV1::Administrator)
    );
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let mut observer = super::super::NullObserver;
    let report = runtime
        .uninstall_apps_approved(request, &approval, &mut observer, &mut NoAdminInteraction)
        .await
        .unwrap();
    assert_eq!(
        report.files[0].action,
        super::super::AppFileAction::UserModified
    );
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user-modified"
    );
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert_eq!(manifest.entries.len(), 1);
}

#[tokio::test]
async fn approved_privileged_forced_app_uninstall_journals_backup_restoration() {
    let runtime = runtime(privileged_static_copy_app_snapshot());
    let (destination, backup) =
        seed_privileged_static_copy_app(&runtime, b"user-modified", Some(b"user-original")).await;
    let backup = backup.unwrap();
    let rollback = crate::action::managed_file_rollback_path(&destination);
    let request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo".to_string()),
        force: true,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(
        plan.permissions
            .required
            .contains(&PermissionV1::Administrator)
    );
    for access in [FilesystemAccessV1::Write, FilesystemAccessV1::Remove] {
        assert!(
            plan.permissions
                .required
                .contains(&PermissionV1::Filesystem {
                    access,
                    path: "absolute:/etc/demo/config.toml".into(),
                })
        );
    }
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        &actions[0].actions[0].kind,
        crate::action::ActionKindV1::ForceRemoveManagedFile {
            persistent_backup: Some(_),
            requires_admin: true,
            ..
        }
    ));

    let mut observer = super::super::NullObserver;
    let report = runtime
        .uninstall_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();
    assert_eq!(
        report.files[0].action,
        super::super::AppFileAction::ForceRestored
    );
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user-original"
    );
    assert!(runtime.host().read(&backup).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
    let operations = runtime.host().operations();
    assert!(operations.contains(&HostOperation::MovePrivileged {
        from: backup,
        to: destination,
    }));
    assert!(operations.contains(&HostOperation::RemovePrivileged(rollback)));
}

#[tokio::test]
async fn approved_forced_app_uninstall_journals_modified_static_copy_removal() {
    let runtime = runtime(static_copy_app_snapshot());
    let (destination, _) = seed_static_copy_app(&runtime, b"user-modified", None).await;
    let rollback = crate::action::managed_file_rollback_path(&destination);
    let request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo".to_string()),
        force: true,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Remove
            && step
                .diagnostic_codes
                .contains(&"app_user_modification_override".to_string())
    }));
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::ForceRemoveManagedFile { .. }))
    ));

    let mut observer = super::super::NullObserver;
    let report = runtime
        .uninstall_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();
    assert_eq!(
        report.files[0].action,
        super::super::AppFileAction::ForceRemoved
    );
    assert!(runtime.host().read(&destination).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
    let manifest = AppManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    assert!(manifest.entries.is_empty());
}

#[tokio::test]
async fn approved_forced_app_uninstall_journals_backup_restoration() {
    let runtime = runtime(static_copy_app_snapshot());
    let (destination, backup) =
        seed_static_copy_app(&runtime, b"user-modified", Some(b"user-original")).await;
    let backup = backup.unwrap();
    let rollback = crate::action::managed_file_rollback_path(&destination);
    let request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo".to_string()),
        force: true,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };

    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    assert!(plan.is_ready());
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        &actions[0].actions[0].kind,
        crate::action::ActionKindV1::ForceRemoveManagedFile {
            persistent_backup: Some(_),
            ..
        }
    ));

    let mut observer = super::super::NullObserver;
    let report = runtime
        .uninstall_apps_approved(request, &approval, &mut observer, &mut Interaction)
        .await
        .unwrap();
    assert_eq!(
        report.files[0].action,
        super::super::AppFileAction::ForceRestored
    );
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user-original"
    );
    assert!(runtime.host().read(&backup).await.is_err());
    assert!(runtime.host().read(&rollback).await.is_err());
}

#[tokio::test]
async fn force_keeps_unchanged_static_copy_on_the_ordinary_remove_action() {
    let runtime = runtime(static_copy_app_snapshot());
    seed_static_copy_app(&runtime, b"managed", None).await;
    let request = AppPlanRequest {
        operation: LifecycleOperation::Uninstall,
        target: Some("demo".to_string()),
        force: true,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    let actions = runtime
        .approved_app_file_action_irs(&request, &plan, &approval)
        .await
        .unwrap();
    assert!(matches!(
        actions.as_slice(),
        [ir] if matches!(ir.actions.as_slice(), [action] if matches!(action.kind, crate::action::ActionKindV1::RemoveManagedFile { .. }))
    ));
}

#[tokio::test]
async fn forced_app_remove_blocks_an_occupied_transaction_rollback_path() {
    let runtime = runtime(static_copy_app_snapshot());
    let (destination, _) = seed_static_copy_app(&runtime, b"user-modified", None).await;
    runtime.host().put_file(
        crate::action::managed_file_rollback_path(&destination),
        b"foreign".to_vec(),
    );

    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Uninstall,
            target: Some("demo".to_string()),
            force: true,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_remove_rollback_occupied".to_string())
    }));
}

#[tokio::test]
async fn app_managed_remove_blocks_an_occupied_transaction_rollback_path() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n[[files]]\nsource = 'config.toml'\n".to_vec(),
            )
            .file("app/demo/config.toml", b"managed".to_vec())
            .build();
    let runtime = runtime(snapshot);
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    runtime.host().put_file(&destination, b"managed".to_vec());
    runtime.host().put_file(
        crate::action::managed_file_rollback_path(&destination),
        b"foreign".to_vec(),
    );
    runtime.host().put_file(
        runtime.context().shine_dir.join("app-manifest.toml"),
        toml::to_string(&AppManifest {
            schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entries: vec![AppEntry {
                source: "app/demo/config.toml".to_string(),
                destination,
                backup: None,
                content_hash: crate::install::hash_content(b"managed"),
                install_strategy: crate::install::AppInstallStrategy::Copy,
                uses_env: false,
                requires_admin: false,
            }],
        })
        .unwrap()
        .into_bytes(),
    );
    let plan = runtime
        .plan_apps(AppPlanRequest {
            operation: LifecycleOperation::Uninstall,
            target: Some("demo".to_string()),
            force: false,
            purge: false,
            prune_stale: false,
            input_versions: PlanningInputVersions::default(),
        })
        .await
        .unwrap();
    assert!(!plan.is_ready());
    assert!(plan.steps.iter().any(|step| {
        step.action == PlanActionV1::Blocked
            && step
                .diagnostic_codes
                .contains(&"app_remove_rollback_occupied".to_string())
    }));
}

#[tokio::test]
async fn frontend_app_operation_state_preserves_post_interruption_user_changes() {
    let runtime = runtime(static_copy_app_snapshot());
    let request = AppPlanRequest {
        operation: LifecycleOperation::Install,
        target: Some("demo".into()),
        force: false,
        purge: false,
        prune_stale: false,
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_apps(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime
        .host()
        .fail_write_after(runtime.context().shine_dir.join("app-manifest.toml"), 0);
    assert!(
        runtime
            .install_apps_approved(
                request,
                &approval,
                &mut super::super::NullObserver,
                &mut Interaction
            )
            .await
            .is_err()
    );
    let destination = runtime.context().home_dir.join(".config/demo/config.toml");
    runtime
        .host()
        .put_file(&destination, b"user content after interruption".to_vec());
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::App, false).await;
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user content after interruption"
    );
}

#[tokio::test]
async fn frontend_sys_operation_state_reports_committed_cleanup_and_blocks_changed_resource() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file(
            "sys/test/shine.toml",
            br#"version = 2
[[items]]
id = "managed"
label = "Managed"
mode = "managed"
driver = "managed-file"
permissions = { schema_version = 1 }
[items.config]
source = "managed.txt"
target = "~/.config/managed.txt"
"#
            .to_vec(),
        )
        .file("sys/test/managed.txt", b"managed".to_vec())
        .build();
    let runtime = runtime(snapshot);
    let request = SysManagedPlanRequest {
        operation: LifecycleOperation::Install,
        os_id: "test".into(),
        target: Some("managed".into()),
        input_versions: PlanningInputVersions::default(),
    };
    let plan = runtime.plan_managed_sys(request.clone()).await.unwrap();
    let approval = PlanApprovalV1::for_reviewed_plan(&plan).unwrap();
    runtime.host().fail_remove_after(
        runtime
            .context()
            .shine_dir
            .join(super::super::SYS_OPERATION_JOURNAL_FILE),
        0,
    );
    let _result = runtime
        .run_managed_sys_approved(
            request,
            &approval,
            &mut Interaction,
            &mut super::super::NullObserver,
        )
        .await;
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::Sys, true).await;
    let observation = runtime
        .inspect_sys_operation_journal()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(observation.receipt_committed_actions, 1);
    let destination = runtime.context().home_dir.join(".config/managed.txt");
    runtime
        .host()
        .put_file(&destination, b"user content".to_vec());
    assert_frontend_journal(&runtime, crate::frontend::CapabilityKindV1::Sys, false).await;
    assert_eq!(
        runtime.host().read(&destination).await.unwrap(),
        b"user content"
    );
}
