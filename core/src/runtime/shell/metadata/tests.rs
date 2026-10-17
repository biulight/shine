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
