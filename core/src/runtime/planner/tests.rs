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
        .context_mut_for_test()
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
    runtime.context_mut_for_test().trust_grants = requirements
        .iter()
        .map(crate::trust::TrustGrantV1::for_reviewed_requirement)
        .collect();
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
        .context_mut_for_test()
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

mod app_planning;

mod app_recovery;

mod app_transactions;

mod contracts;

mod frontend;

mod shell_planning;

mod shell_recovery;

mod shell_transactions;

mod sys;
