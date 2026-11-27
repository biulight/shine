use shine_core::runtime::{
    CoreRuntime, InMemoryHost, PresetSnapshot, PresetSourceKind, RuntimeContext, RuntimePlatform,
    ShellType, validate_preset_path,
};
use std::path::Path;

#[test]
fn core_manifest_excludes_frontend_and_distribution_dependencies() {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    for forbidden in ["clap", "dialoguer", "console", "tauri", "rust-embed"] {
        assert!(
            !manifest.lines().any(|line| {
                line.split_once('=')
                    .is_some_and(|(name, _)| name.trim() == forbidden)
            }),
            "shine-core must not depend on {forbidden}"
        );
    }
}

#[test]
fn cli_domain_adapters_do_not_retain_legacy_mutation_or_metadata_fallbacks() {
    let core_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository_root = core_root.parent().unwrap();

    for removed in [
        "cli/src/apps/hooks.rs",
        "cli/src/sys/bootstrap.rs",
        "cli/src/sys/resources.rs",
        "cli/src/sys/drivers/mod.rs",
        "cli/src/sys/drivers/managed_file.rs",
        "cli/src/sys/drivers/split_dns.rs",
        "cli/src/sys/profile.rs",
        "cli/src/sys/profile_blocks.rs",
        "cli/src/sys/profile_compose.rs",
    ] {
        assert!(
            !repository_root.join(removed).exists(),
            "legacy CLI domain implementation still exists: {removed}"
        );
    }

    for adapter in [
        "cli/src/apps/metadata.rs",
        "cli/src/shells/metadata.rs",
        "cli/src/preset_validation.rs",
        "cli/src/sys/profile_commands.rs",
    ] {
        let source = std::fs::read_to_string(repository_root.join(adapter)).unwrap();
        assert!(
            source.contains("core_runtime") || source.contains("shine_core::runtime"),
            "{adapter} must route through Core"
        );
        for forbidden in [
            "serde::Deserialize",
            "tokio::fs::write",
            "SysRunManifest::save",
        ] {
            assert!(
                !source.contains(forbidden),
                "{adapter} retains forbidden domain implementation `{forbidden}`"
            );
        }
    }

    assert_source_excludes(
        &repository_root.join("cli/src/bin_links.rs"),
        &[
            "launcher::*",
            "link_executables_with_names",
            "unlink_managed_command",
            "unlink_managed",
        ],
    );
    assert_source_excludes(
        &repository_root.join("cli/src/install_core/file_ops.rs"),
        &[
            "install_bytes_admin",
            "uninstall_entry_admin",
            " install_bytes,",
            " uninstall_entry,",
        ],
    );
}

fn assert_source_excludes(path: &Path, forbidden: &[&str]) {
    let source =
        std::fs::read_to_string(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    for &pattern in forbidden {
        assert!(
            !source.contains(pattern),
            "{} contains forbidden `{pattern}`",
            path.display()
        );
    }
}

#[test]
fn core_domain_sources_do_not_bypass_captured_hosts() {
    let core_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository_root = core_root.parent().unwrap();
    let runtime = core_root.join("src/runtime");
    assert_source_excludes(
        &repository_root.join("cli/src/core_runtime.rs"),
        &["fn collect_tree", "std::fs::read_dir", "std::fs::read("],
    );
    assert_source_excludes(&runtime.join("bootstrap.rs"), &["std::fs::", "std::env::"]);
    for module in [
        "validation.rs",
        "validation/source.rs",
        "validation/permissions.rs",
        "validation/destinations.rs",
        "validation/sys.rs",
    ] {
        assert_source_excludes(
            &runtime.join(module),
            &["std::fs::", "std::env::current_dir"],
        );
    }
    assert_source_excludes(&runtime.join("sys_bootstrap.rs"), &["script.is_file()"]);
    assert_source_excludes(
        &runtime.join("mod.rs"),
        &[
            "link_executables_with_names",
            "link_is_current,",
            "unlink_managed,",
            "unlink_managed_command,",
        ],
    );
    assert_source_excludes(
        &core_root.join("src/install/mod.rs"),
        &[" install_bytes,", " uninstall_entry,"],
    );
}

fn bash_context(name: &str) -> RuntimeContext {
    let root = std::env::temp_dir().join(name);
    let mut context = RuntimeContext::isolated(
        root.join("home"),
        root.join("state"),
        root.join("presets"),
        root.join("bin"),
        RuntimePlatform::Linux,
    );
    // Match the simulated platform and .sh fixtures regardless of the host OS.
    context.shell = ShellType::Bash;
    context
}

#[tokio::test]
async fn security_planners_require_only_observation_capabilities() {
    use shine_core::lifecycle::LifecycleOperation;
    use shine_core::runtime::*;
    // A generic observation-only bound is checked by Rust for every planner.
    // This remains valid when implementations or test modules move between files.
    async fn assess<H: FileSystemObservationHost + SplitDnsObservationHost>(
        runtime: &CoreRuntime<H>,
    ) {
        let input_versions = PlanningInputVersions::default();
        runtime
            .plan_apps(AppPlanRequest {
                operation: LifecycleOperation::Install,
                target: Some("demo".into()),
                force: false,
                purge: false,
                prune_stale: false,
                input_versions: input_versions.clone(),
            })
            .await
            .unwrap();
        runtime
            .plan_shells(ShellPlanRequest {
                operation: LifecycleOperation::Install,
                target: Some("demo".into()),
                force: false,
                purge: false,
                input_versions: input_versions.clone(),
            })
            .await
            .unwrap();
        runtime
            .plan_managed_sys(SysManagedPlanRequest {
                operation: LifecycleOperation::Install,
                os_id: "test".into(),
                target: Some("managed".into()),
                input_versions: input_versions.clone(),
            })
            .await
            .unwrap();
        runtime
            .plan_sys_bootstrap(SysBootstrapPlanRequest {
                os_id: "test".into(),
                item_ids: vec!["bootstrap".into()],
                sys_shell: "bash".into(),
                force_profile: false,
                input_versions: input_versions.clone(),
            })
            .await
            .unwrap();
        runtime
            .plan_app_refresh(AppRefreshPlanRequest {
                category: "demo".into(),
                file: None,
                force: false,
                input_versions: input_versions.clone(),
            })
            .await
            .unwrap_err();
        runtime
            .plan_app_artifact(AppArtifactPlanRequest {
                category: "demo".into(),
                action: AppArtifactAction::Apply,
                input_versions,
            })
            .await
            .unwrap_err();
        runtime
            .plan_sys_profile(SysProfilePlanRequest {
                os_id: "test".into(),
                item_id: "managed".into(),
                enabled: true,
            })
            .await
            .unwrap_err();
        runtime.plan_app_operation_recovery().await.unwrap_err();
        runtime.plan_shell_operation_recovery().await.unwrap_err();
        runtime.plan_sys_operation_recovery().await.unwrap_err();
    }
    let host = InMemoryHost::new();
    let context = bash_context("shine-planner-boundary");
    let snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded)
        .file("app/demo/shine.toml", b"metadata_schema_version = 2\ndest = '~/.config/demo'\n[[files]]\nsource = 'config.toml'\n".to_vec())
        .file("app/demo/config.toml", b"static data".to_vec())
        .file("shell/demo/shine.toml", b"[[files]]\nsource = 'demo.sh'\ntarget = 'demo'\n".to_vec())
        .file("shell/demo/demo.sh", b"#!/bin/sh\n".to_vec())
        .file("sys/test/shine.toml", br#"version = 2
[[items]]
id = 'managed'
label = 'Managed'
mode = 'managed'
driver = 'managed-file'
config = { source = 'config.toml', target = '~/managed.toml' }
[[items]]
id = 'bootstrap'
label = 'Bootstrap'
detect = { kind = 'path', path = '~/detected' }
install = { kind = 'script', path = 'install.sh' }
"#.to_vec())
        .file("sys/test/config.toml", b"managed data".to_vec())
        .file("sys/test/install.sh", b"#!/bin/sh\n".to_vec()).build();
    let runtime = CoreRuntime::new(host, context, snapshot);
    assess(&runtime).await;
    assert!(
        runtime
            .host()
            .operations()
            .iter()
            .all(|operation| matches!(operation, HostOperation::Read(_)))
    );
}

#[tokio::test]
async fn core_only_harness_uses_explicit_inputs_and_virtual_state() {
    use shine_core::frontend::{FrontendService, InspectionOperationV1, InspectionStateV1};
    use shine_core::runtime::{HostOperation, InspectionFileStatus};

    let host = InMemoryHost::new();
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file(
            "shell/tools/shine.toml",
            b"description = 'tools'\n[[files]]\nsource = 'tool.sh'\ntarget = 'tool'\n".to_vec(),
        )
        .file("shell/tools/tool.sh", b"#!/bin/sh\n".to_vec())
        .build();
    let context = bash_context("shine-runtime-boundary-harness");
    let runtime = CoreRuntime::new(host.clone(), context, snapshot);

    assert!(runtime.validate().valid);
    let before = host.operations().len();
    let inspection = FrontendService::new(runtime)
        .inspect_shells()
        .await
        .unwrap();
    assert_eq!(inspection.files.len(), 1);
    assert!(!inspection.files[0].installed);
    assert_eq!(
        inspection.files[0].status,
        InspectionFileStatus::NotInstalled
    );
    assert_eq!(inspection.report.items.len(), 1);
    let item = &inspection.report.items[0];
    assert_eq!(item.target, "shell/tools/tool");
    assert_eq!(item.state, InspectionStateV1::NotInstalled);
    assert_eq!(item.operations, [InspectionOperationV1::Install]);
    assert!(
        host.operations()[before..]
            .iter()
            .all(|op| matches!(op, HostOperation::Read(_)))
    );
}

#[tokio::test]
async fn preset_validation_uses_virtual_filesystem_and_captured_cwd() {
    let host = InMemoryHost::new();
    host.put_file(
        "/virtual/presets/shell/tools/shine.toml",
        b"[[files]]\nsource = \"tool.sh\"\ntarget = \"tool\"\n".to_vec(),
    );
    host.put_file(
        "/virtual/presets/shell/tools/tool.sh",
        b"#!/bin/sh\n".to_vec(),
    );

    let report = validate_preset_path(
        &host,
        Path::new("/virtual"),
        Path::new("presets/shell/tools"),
    )
    .await;

    assert!(report.valid, "{:#?}", report.diagnostics);
    assert_eq!(report.path, Path::new("/virtual/presets/shell/tools"));
}
