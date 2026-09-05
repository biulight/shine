use shine_core::runtime::{
    CoreRuntime, InMemoryHost, PresetSnapshot, PresetSourceKind, RuntimeContext, RuntimePlatform,
    validate_preset_path,
};
use std::path::{Path, PathBuf};

#[test]
fn cli_lifecycle_authority_routes_through_frontend_service() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let review = std::fs::read_to_string(root.join("cli/src/lifecycle_plan.rs")).unwrap();
    assert!(review.contains(".approve_after_human_confirmation()"));
    assert!(review.contains(".validate_approved("));
    assert!(!review.contains("PlanApprovalV1"));
    for adapter in [
        "apps/install.rs",
        "apps/uninstall.rs",
        "apps/upgrade.rs",
        "apps/refresh.rs",
        "apps/build.rs",
        "apps/recovery.rs",
        "shells/install.rs",
        "shells/uninstall.rs",
        "shells/recovery.rs",
        "sys/managed.rs",
        "sys/profile_commands.rs",
        "sys/commands.rs",
        "sys/recovery.rs",
    ] {
        let source = std::fs::read_to_string(root.join("cli/src").join(adapter)).unwrap();
        assert!(
            source.contains("lifecycle_plan::execute_reviewed("),
            "{adapter}"
        );
        for method in [
            "install_apps_approved",
            "uninstall_apps_approved",
            "upgrade_apps_approved",
            "refresh_app_generators_approved",
            "run_app_artifact_approved",
            "install_shells_approved",
            "uninstall_shells_approved",
            "upgrade_shells_approved",
            "run_managed_sys_approved",
            "set_sys_profile_approved",
            "run_sys_bootstrap_approved",
            "recover_app_operation_approved",
            "recover_shell_operation_approved",
            "recover_sys_operation_approved",
        ] {
            assert!(
                !source.contains(&format!(".{method}(")),
                "{adapter} bypasses shared execution"
            );
        }
    }
}

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

    let bin_links = std::fs::read_to_string(repository_root.join("cli/src/bin_links.rs")).unwrap();
    assert!(!bin_links.contains("launcher::*"));
    for forbidden in [
        "link_executables_with_names",
        "unlink_managed_command",
        "unlink_managed",
    ] {
        assert!(
            !bin_links.contains(forbidden),
            "CLI Shell adapter re-exports mutation fallback `{forbidden}`"
        );
    }

    let app_file_ops =
        std::fs::read_to_string(repository_root.join("cli/src/install_core/file_ops.rs")).unwrap();
    for forbidden in [
        "install_bytes_admin",
        "uninstall_entry_admin",
        " install_bytes,",
        " uninstall_entry,",
    ] {
        assert!(
            !app_file_ops.contains(forbidden),
            "CLI App mutation fallback remains: `{forbidden}`"
        );
    }
}

#[test]
fn core_domain_sources_do_not_bypass_captured_hosts() {
    let core_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository_root = core_root.parent().unwrap();
    let cli_assembly =
        std::fs::read_to_string(repository_root.join("cli/src/core_runtime.rs")).unwrap();
    for forbidden in ["fn collect_tree", "std::fs::read_dir", "std::fs::read("] {
        assert!(
            !cli_assembly.contains(forbidden),
            "CLI duplicates host-backed preset discovery with `{forbidden}`"
        );
    }

    let bootstrap = std::fs::read_to_string(core_root.join("src/runtime/bootstrap.rs")).unwrap();
    for forbidden in ["std::fs::", "std::env::"] {
        assert!(
            !bootstrap.contains(forbidden),
            "shared runtime bootstrap bypasses its host with `{forbidden}`"
        );
    }

    let validation = std::fs::read_to_string(core_root.join("src/runtime/validation.rs")).unwrap();
    for forbidden in ["std::fs::", "std::env::current_dir"] {
        assert!(
            !validation.contains(forbidden),
            "Core validation bypasses its host with `{forbidden}`"
        );
    }

    let sys_bootstrap =
        std::fs::read_to_string(core_root.join("src/runtime/sys_bootstrap.rs")).unwrap();
    assert!(
        !sys_bootstrap.contains("script.is_file()"),
        "Sys preflight reads the ambient preset tree"
    );

    let exports = std::fs::read_to_string(core_root.join("src/runtime/mod.rs")).unwrap();
    for forbidden in [
        "link_executables_with_names",
        "link_is_current,",
        "unlink_managed,",
        "unlink_managed_command,",
    ] {
        assert!(
            !exports.contains(forbidden),
            "Core exports a no-host Shell mutation fallback: `{forbidden}`"
        );
    }

    let install_exports = std::fs::read_to_string(core_root.join("src/install/mod.rs")).unwrap();
    for forbidden in [" install_bytes,", " uninstall_entry,"] {
        assert!(
            !install_exports.contains(forbidden),
            "Core exports a no-host App mutation fallback: `{forbidden}`"
        );
    }
}

#[test]
fn security_planners_use_observation_bounds_and_no_raw_mutation_calls() {
    let core_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let planner = std::fs::read_to_string(core_root.join("src/runtime/planner.rs")).unwrap();
    let planner = planner.replace("\r\n", "\n");
    // Inline tests may mutate their virtual host to arrange observed state.
    let (planner, _) = planner
        .split_once("\n#[cfg(test)]\nmod tests {")
        .expect("planner must retain its explicit inline test-module boundary");

    assert!(planner.contains("impl<H: FileSystemObservationHost> CoreRuntime<H>"));
    assert!(planner.contains("impl<H: FileSystemObservationHost + SplitDnsObservationHost>"));
    for forbidden in [
        ".write_atomic(",
        ".remove_file(",
        ".remove_dir_all(",
        ".run_process(",
        ".apply_split_dns(",
        ".remove_split_dns(",
    ] {
        assert!(
            !planner.contains(forbidden),
            "security planner contains a raw mutation call `{forbidden}`"
        );
    }
}

#[tokio::test]
async fn core_only_harness_uses_explicit_inputs_and_virtual_state() {
    let host = InMemoryHost::new();
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file(
            "shell/tools/shine.toml",
            b"description = \"tools\"\n".to_vec(),
        )
        .build();
    let context = RuntimeContext::isolated(
        PathBuf::from("/virtual/home"),
        PathBuf::from("/virtual/home/.shine"),
        PathBuf::from("/virtual/home/.shine/presets"),
        PathBuf::from("/virtual/home/.shine/bin"),
        RuntimePlatform::Linux,
    );
    let runtime = CoreRuntime::new(host, context, snapshot);

    assert!(runtime.validate().valid);
    let inspection = runtime
        .inspect_snapshot(Path::new("/virtual/installed"))
        .await
        .unwrap();
    assert_eq!(inspection.resources.len(), 1);
    assert!(!inspection.resources[0].installed);
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
