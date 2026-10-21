use super::*;
use crate::runtime::{PresetSnapshot, PresetSourceKind, RuntimeContext};

fn runtime(metadata: &str) -> CoreRuntime<()> {
    let root = std::env::temp_dir().join("shine-shell-metadata");
    let mut context = RuntimeContext::isolated(
        root.join("home"),
        root.join("state"),
        root.join("presets"),
        root.join("bin"),
        RuntimePlatform::Linux,
    );
    context.shell = ShellType::Zsh;
    CoreRuntime::new(
        (),
        context,
        PresetSnapshot::builder(PresetSourceKind::External)
            .file("shell/demo/shine.toml", metadata.as_bytes().to_vec())
            .file(
                "shell/demo/a.sh",
                b"# native description\necho a\n".to_vec(),
            )
            .file("shell/demo/a.ps1", b"Write-Output a".to_vec())
            .file("shell/demo/b.sh", b"echo b".to_vec())
            .build(),
    )
}

#[test]
fn explicit_shell_entries_filter_before_runtime_and_source_validation() {
    let runtime = runtime(
        "[[files]]\nsource = '../missing.ts'\nruntime = 'unsupported'\nplatforms = ['windows']\ntarget = 'a'\n[[files]]\nsource = 'missing.ps1'\ntarget = '../invalid'\ntransforms = ['unsupported']\n[[files]]\nsource = 'a.sh'\ntarget = 'a'\n",
    );
    let categories = runtime.shell_categories(Some("demo")).unwrap();
    assert_eq!(categories[0].files.len(), 1);
    assert_eq!(categories[0].files[0].command_name, "a");
    assert_eq!(categories[0].files[0].description, ["native description"]);
}

#[test]
fn explicit_empty_shell_files_suppress_native_discovery_and_validate_defaults() {
    assert!(
        runtime("files = []").shell_categories(None).unwrap()[0]
            .files
            .is_empty()
    );
    let error = runtime("files = []\n[permission_defaults]\nschema_version = 99")
        .shell_categories(None)
        .unwrap_err();
    assert!(error.to_string().contains("invalid permission_defaults"));
    let discovered = runtime("").shell_categories(None).unwrap();
    assert_eq!(
        discovered[0]
            .files
            .iter()
            .map(|file| file.command_name.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert!(
        discovered[0]
            .files
            .iter()
            .all(|file| file.runtime == LinkRuntime::Native)
    );
}

#[test]
fn admitted_shell_entries_preserve_missing_reference_and_duplicate_diagnostics() {
    for (metadata, expected) in [
        (
            "[[files]]\nsource = 'missing.sh'",
            MetadataDiagnosticKind::MissingReference,
        ),
        (
            "[[files]]\nsource = 'a.sh'\ntarget = 'same'\n[[files]]\nsource = 'b.sh'\ntarget = 'same'",
            MetadataDiagnosticKind::DuplicateCommand,
        ),
    ] {
        let error = runtime(metadata).shell_categories(None).unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<MetadataDiagnosticError>()
                .unwrap()
                .kind,
            expected
        );
    }
}

#[test]
fn parsed_shell_entry_permissions_replace_defaults_without_inheriting_opaque_grants() {
    use crate::plan::{OpaqueCodeScopeV1, PermissionV1};
    let categories = runtime("[permission_defaults]\nschema_version = 2\nopaque_code = 'unrestricted'\n[[files]]\nsource = 'a.sh'\n[[files]]\nsource = 'b.sh'\n[files.permissions]\nschema_version = 1\ncommands = ['git']\n").shell_categories(None).unwrap();
    let files = &categories[0].files;
    let inherited = files[0]
        .permissions
        .as_ref()
        .unwrap()
        .permission_set()
        .unwrap();
    let overridden = files[1]
        .permissions
        .as_ref()
        .unwrap()
        .permission_set()
        .unwrap();
    assert!(inherited.contains(&PermissionV1::OpaqueCode {
        scope: OpaqueCodeScopeV1::Unrestricted
    }));
    assert!(!overridden.contains(&PermissionV1::OpaqueCode {
        scope: OpaqueCodeScopeV1::Unrestricted
    }));
    assert!(overridden.contains(&PermissionV1::Command {
        program: "git".into()
    }));
}

fn bun_snapshot(kind: PresetSourceKind) -> crate::runtime::preset::PresetSnapshotBuilder {
    PresetSnapshot::builder(kind)
        .file(
            "shell/demo/shine.toml",
            b"[[files]]\nsource = 'run.ts'\nruntime = 'bun'\n".to_vec(),
        )
        .file("shell/demo/run.ts", b"console.log('demo')".to_vec())
}

fn bun_spec(snapshot: PresetSnapshot) -> Result<BunRuntimeSpec> {
    let mut context = runtime("").context().clone();
    context.is_external_presets = snapshot.source_kind() == PresetSourceKind::External;
    let runtime = CoreRuntime::new((), context, snapshot);
    let categories = runtime.shell_categories(Some("demo"))?;
    runtime.shell_bun_runtime_spec("demo", &categories[0].files[0])
}

#[test]
fn built_in_scripts_ignore_package_files() {
    let snapshot = bun_snapshot(PresetSourceKind::Embedded)
        .file("shell/demo/package.json", b"invalid package".to_vec())
        .overlay_file("shell/demo/bun.lock", b"overlay lock".to_vec())
        .build();
    assert_eq!(bun_spec(snapshot).unwrap(), BunRuntimeSpec::default());
}

#[test]
fn external_scripts_require_a_complete_locked_pair() {
    assert_eq!(
        bun_spec(bun_snapshot(PresetSourceKind::External).build()).unwrap(),
        BunRuntimeSpec::default()
    );
    for (file, bytes, missing) in [
        ("package.json", b"{}".as_slice(), "bun.lock"),
        ("bun.lock", b"lock".as_slice(), "package.json"),
    ] {
        let snapshot = bun_snapshot(PresetSourceKind::External)
            .file(format!("shell/demo/{file}"), bytes.to_vec())
            .build();
        let error = bun_spec(snapshot).unwrap_err();
        assert!(error.to_string().contains(missing));
        assert_eq!(
            error
                .downcast_ref::<MetadataDiagnosticError>()
                .unwrap()
                .kind,
            MetadataDiagnosticKind::BunDependencyPolicy
        );
    }
}

#[test]
fn external_scripts_accept_locked_dependencies_and_hash_both_files() {
    let spec = |package: &[u8], lock: &[u8]| {
        bun_spec(
            bun_snapshot(PresetSourceKind::External)
                .file("shell/demo/package.json", package.to_vec())
                .file("shell/demo/bun.lock", lock.to_vec())
                .build(),
        )
        .unwrap()
    };
    let first = spec(br#"{"dependencies":{"zod":"4.0.0"}}"#, b"lock");
    assert_eq!(first.dependency_mode, BunDependencyMode::Locked);
    assert!(first.dependency_hash.is_some());
    assert_ne!(
        first.dependency_hash,
        spec(br#"{"dependencies":{"zod":"4.0.0"}}"#, b"changed lock").dependency_hash
    );
    assert_ne!(
        first.dependency_hash,
        spec(br#"{"dependencies":{"zod":"4.0.1"}}"#, b"lock").dependency_hash
    );
}

#[test]
fn external_scripts_reject_trusted_dependencies() {
    let snapshot = bun_snapshot(PresetSourceKind::External)
        .file(
            "shell/demo/package.json",
            br#"{"trustedDependencies":[]}"#.to_vec(),
        )
        .file("shell/demo/bun.lock", b"lock".to_vec())
        .build();
    let error = bun_spec(snapshot).unwrap_err();
    assert!(error.to_string().contains("trustedDependencies"));
    assert_eq!(
        error
            .downcast_ref::<MetadataDiagnosticError>()
            .unwrap()
            .kind,
        MetadataDiagnosticKind::BunDependencyPolicy
    );
}

#[test]
fn overlay_scripts_require_package_and_lock_from_their_own_layer() {
    let snapshot = || {
        bun_snapshot(PresetSourceKind::Embedded)
            .file("shell/demo/package.json", b"{}".to_vec())
            .file("shell/demo/bun.lock", b"embedded lock".to_vec())
            .overlay_file("shell/demo/run.ts", b"console.log('overlay')".to_vec())
    };
    assert_eq!(
        bun_spec(snapshot().build()).unwrap(),
        BunRuntimeSpec::default()
    );
    let error = bun_spec(
        snapshot()
            .overlay_file("shell/demo/package.json", b"{}".to_vec())
            .build(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("bun.lock"));
    let spec = bun_spec(
        snapshot()
            .overlay_file("shell/demo/package.json", b"{}".to_vec())
            .overlay_file("shell/demo/bun.lock", b"overlay lock".to_vec())
            .build(),
    )
    .unwrap();
    assert_eq!(spec.dependency_mode, BunDependencyMode::Locked);
    assert!(spec.dependency_hash.is_some());
}

// Fixture assembly only: descriptions come from the production metadata entry point.
fn production_description(bytes: &[u8], link_runtime: LinkRuntime) -> Vec<String> {
    let (source, runtime_name) = if link_runtime == LinkRuntime::Bun {
        ("run.ts", "bun")
    } else {
        ("run.sh", "native")
    };
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file(
            "shell/demo/shine.toml",
            format!("[[files]]\nsource = '{source}'\nruntime = '{runtime_name}'\n").into_bytes(),
        )
        .file(format!("shell/demo/{source}"), bytes.to_vec())
        .build();
    CoreRuntime::new((), runtime("").context().clone(), snapshot)
        .shell_categories(Some("demo"))
        .unwrap()[0]
        .files[0]
        .description
        .clone()
}

#[test]
fn parse_description_extracts_comment_block() {
    let script = b"#!/bin/bash\n# First line.\n# Second line.\n\nsome_command\n";
    let desc = production_description(script, LinkRuntime::Native);
    assert_eq!(desc, vec!["First line.", "Second line."]);
}

#[test]
fn parse_description_skips_shebang_only() {
    let script = b"#!/bin/bash\nsome_command\n";
    let desc = production_description(script, LinkRuntime::Native);
    assert!(desc.is_empty());
}

#[test]
fn parse_description_handles_bare_hash_as_empty_line() {
    let script = b"#!/bin/bash\n# First.\n#\n# Third.\n";
    let desc = production_description(script, LinkRuntime::Native);
    assert_eq!(desc, vec!["First.", "", "Third."]);
}

#[test]
fn parse_description_trims_trailing_empty_lines() {
    let script = b"#!/bin/bash\n# First.\n#\n#\n";
    let desc = production_description(script, LinkRuntime::Native);
    assert_eq!(desc, vec!["First."]);
}

#[test]
fn parse_bun_description_extracts_slash_comment_block() {
    let script = b"// First line.\n// Second line.\nconsole.log('hi')\n";
    let desc = production_description(script, LinkRuntime::Bun);
    assert_eq!(desc, vec!["First line.", "Second line."]);
}

#[test]
fn parse_bun_description_skips_shebang_and_stops_at_code() {
    let script = b"#!/usr/bin/env bun\n// Only line.\nexport const x = 1\n";
    let desc = production_description(script, LinkRuntime::Bun);
    assert_eq!(desc, vec!["Only line."]);
}

#[test]
fn parse_bun_description_handles_bare_slash_as_empty_line() {
    let script = b"// First.\n//\n// Third.\n";
    let desc = production_description(script, LinkRuntime::Bun);
    assert_eq!(desc, vec!["First.", "", "Third."]);
}

#[test]
fn parse_bun_description_empty_when_starts_with_code() {
    let script = b"import { foo } from './foo'\n// not a header\n";
    let desc = production_description(script, LinkRuntime::Bun);
    assert!(desc.is_empty());
}

#[test]
fn parse_description_empty_content() {
    let desc = production_description(b"", LinkRuntime::Native);
    assert!(desc.is_empty());
}

#[test]
fn descriptions_preserve_current_whitespace_and_annotation_rules() {
    for (link_runtime, bytes) in [
        (LinkRuntime::Native, b"#!/bin/sh\n  # First.\n\n# shine-dest: ignored\n# shine-template: true\n# shine-custom: ignored\n# Second.\n#\necho stop\n# hidden\n".as_slice()),
        (LinkRuntime::Bun, b"#!/usr/bin/env bun\n  // First.\n\n// shine-custom: ignored\n// Second.\n//\nexport const stop = 1\n// hidden\n".as_slice()),
    ] {
        assert_eq!(production_description(bytes, link_runtime), ["First.", "Second."]);
        assert!(production_description(&[0xff], link_runtime).is_empty());
    }
    let categories = runtime("[[files]]\nsource = 'a.sh'\ndescription = 'Declared description'\n")
        .shell_categories(Some("demo"))
        .unwrap();
    assert_eq!(categories[0].files[0].description, ["Declared description"]);
}
