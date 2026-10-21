use super::destinations::{is_portable_relative_path, validate_destination_path};
use super::*;
use crate::runtime::PresetSourceKind;

#[test]
fn destination_validation_is_independent_of_host_path_syntax() {
    for path in [
        "~",
        "~/.config/tool",
        r"~\AppData\Roaming\tool",
        "$HOME",
        "$HOME/.config/tool",
        "/etc/tool",
        r"C:\ProgramData\tool",
        r"\\server\share\tool",
    ] {
        validate_destination_path(path).unwrap_or_else(|error| {
            panic!("expected {path:?} to be a portable absolute destination: {error:#}")
        });
    }

    for path in ["relative/tool", r"C:relative\tool", r"\rooted\tool"] {
        assert!(
            validate_destination_path(path).is_err(),
            "expected {path:?} to be rejected"
        );
    }
    for path in ["~/../tool", r"C:\ProgramData\..\tool"] {
        assert!(
            validate_destination_path(path).is_err(),
            "expected {path:?} to reject a parent segment"
        );
    }
}

#[test]
fn relative_validation_rejects_foreign_platform_roots() {
    for path in ["nested/file", r"nested\file"] {
        assert!(
            is_portable_relative_path(path),
            "expected {path:?} to be relative"
        );
    }
    for path in [
        "",
        "/absolute/file",
        r"\rooted\file",
        r"C:\absolute\file",
        r"C:drive-relative\file",
        "../outside",
        r"..\outside",
    ] {
        assert!(
            !is_portable_relative_path(path),
            "expected {path:?} to be rejected"
        );
    }
}

#[test]
fn permission_validation_uses_the_effective_overlay_manifest() {
    let category = CategoryPath {
        kind: "app",
        name: "demo".to_string(),
        root: PathBuf::from("app/demo"),
    };
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
        .file(
            "app/demo/shine.toml",
            b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n".to_vec(),
        )
        .overlay_file("app/demo/shine.toml", b"dest = '~/.config/demo'\n".to_vec())
        .build();

    let diagnostics = permission_declaration_diagnostics(&snapshot, &category);
    assert!(diagnostics.is_empty());

    let invalid = PresetSnapshot::builder(PresetSourceKind::External)
        .file(
            "app/demo/shine.toml",
            b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n".to_vec(),
        )
        .overlay_file(
            "app/demo/shine.toml",
            b"dest = '~/.config/demo'\n[permissions]\nschema_version = 3\n".to_vec(),
        )
        .build();
    let diagnostics = permission_declaration_diagnostics(&invalid, &category);
    assert_eq!(diagnostics[0].code, "unsupported_permission_schema");
}

#[test]
fn permission_declarations_use_each_domain_target_placement() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permissions]\nschema_version = 1\n".to_vec(),
            )
            .file(
                "shell/demo/shine.toml",
                b"[[files]]\nsource = 'tool.sh'\n[files.permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .file(
                "sys/demo/shine.toml",
                b"version = 2\n[[items]]\nid = 'tool'\nlabel = 'Tool'\n[items.permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .build();

    for kind in ["app", "shell", "sys"] {
        let category = CategoryPath {
            kind,
            name: "demo".to_string(),
            root: PathBuf::from(format!("{kind}/demo")),
        };
        assert!(
            permission_declaration_diagnostics(&snapshot, &category).is_empty(),
            "{kind} declaration should be accepted at its target-local placement"
        );
    }
}

#[test]
fn shell_and_sys_permission_defaults_satisfy_target_declarations() {
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "shell/demo/shine.toml",
                b"[permission_defaults]\nschema_version = 2\nopaque_code = 'unrestricted'\n[[files]]\nsource = 'tool.sh'\n"
                    .to_vec(),
            )
            .file(
                "sys/demo/shine.toml",
                b"version = 2\n[permission_defaults]\nschema_version = 2\nopaque_code = 'unrestricted'\n[[items]]\nid = 'tool'\nlabel = 'Tool'\n"
                    .to_vec(),
            )
            .build();

    for (kind, name) in [("shell", "demo"), ("sys", "demo")] {
        let category = CategoryPath {
            kind,
            name: name.to_string(),
            root: PathBuf::from(format!("{kind}/{name}")),
        };
        assert!(
            permission_declaration_diagnostics(&snapshot, &category).is_empty(),
            "{kind} defaults should satisfy item-local declaration validation"
        );
    }
}

#[test]
fn app_rejects_shell_and_sys_permission_defaults_placement() {
    let category = CategoryPath {
        kind: "app",
        name: "demo".to_string(),
        root: PathBuf::from("app/demo"),
    };
    let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[permission_defaults]\nschema_version = 2\nopaque_code = 'unrestricted'\n[permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .build();

    let diagnostics = permission_declaration_diagnostics(&snapshot, &category);
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "invalid_permission_declaration"
            && diagnostic.message.contains("must use `[permissions]`")
    }));
}

#[test]
fn app_artifact_environment_sources_require_permission_declarations() {
    let category = CategoryPath {
        kind: "app",
        name: "demo".to_string(),
        root: PathBuf::from("app/demo"),
    };
    let missing = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[artifact]\nscript = 'build.sh'\nenv = ['TOKEN=API_TOKEN']\n[permissions]\nschema_version = 1\n"
                    .to_vec(),
            )
            .build();
    let diagnostics = permission_declaration_diagnostics(&missing, &category);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "undeclared_artifact_environment")
    );

    let declared = PresetSnapshot::builder(PresetSourceKind::External)
            .file(
                "app/demo/shine.toml",
                b"dest = '~/.config/demo'\n[artifact]\nscript = 'build.sh'\nenv = ['TOKEN=API_TOKEN']\n[permissions]\nschema_version = 1\nenvironment = [{ name = 'TOKEN', sensitivity = 'secret' }]\n"
                    .to_vec(),
            )
            .build();
    assert!(permission_declaration_diagnostics(&declared, &category).is_empty());
}

// This wrapper deliberately has no mutation or process capability.
struct ObservationOnly(InMemoryHost);
type ReadFuture<'a, T> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<T, crate::runtime::HostError>> + Send + 'a>,
>;
impl FileSystemObservationHost for ObservationOnly {
    fn canonicalize<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, PathBuf> {
        self.0.canonicalize(path)
    }
    fn read<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, Vec<u8>> {
        self.0.read(path)
    }
    fn metadata<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, crate::runtime::FileMetadata> {
        self.0.metadata(path)
    }
    fn read_dir<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, Vec<PathBuf>> {
        self.0.read_dir(path)
    }
    fn read_link<'a>(&'a self, path: &'a Path) -> ReadFuture<'a, PathBuf> {
        self.0.read_link(path)
    }
}

#[tokio::test]
async fn repository_category_and_manifest_inputs_share_read_only_snapshot_validation() {
    let root = std::env::temp_dir().join("shine-validation-entrypoints");
    let host = ObservationOnly(InMemoryHost::new());
    host.0.put_file(
        root.join("app/demo/shine.toml"),
        b"dest = '~/.config/demo'\n[[files]]\nsource = 'config'\n".to_vec(),
    );
    host.0
        .put_file(root.join("app/demo/config"), b"data".to_vec());
    host.0.put_file(
        root.join("shell/tools/shine.toml"),
        b"[[files]]\nsource = 'tool.sh'\ntarget = 'tool'\n".to_vec(),
    );
    host.0.put_file_with_mode(
        root.join("shell/tools/tool.sh"),
        b"#!/bin/sh\n".to_vec(),
        0o100755,
    );
    host.0.put_file(
        root.join("app/demo/node_modules/broken/shine.toml"),
        b"not TOML".to_vec(),
    );
    let repository = validate_preset_path(&host, &root, Path::new(".")).await;
    assert!(repository.valid, "{repository:#?}");
    assert_eq!(
        repository
            .categories
            .iter()
            .map(|category| (category.kind.as_str(), category.name.as_str()))
            .collect::<Vec<_>>(),
        [("app", "demo"), ("shell", "tools")]
    );
    for input in ["app/demo", "app/demo/shine.toml"] {
        let report = validate_preset_path(&host, &root, Path::new(input)).await;
        assert!(report.valid, "{report:#?}");
        assert_eq!(report.categories, [repository.categories[0].clone()]);
        let scope = load_preset_source_scope(&host, &root, Path::new(input))
            .await
            .unwrap();
        assert!(
            scope
                .snapshot
                .get("app/demo/node_modules/broken/shine.toml")
                .is_none()
        );
        assert!(
            scope
                .snapshot
                .file("shell/tools/tool.sh")
                .unwrap()
                .executable
        );
    }
    assert!(
        host.0
            .operations()
            .iter()
            .all(|op| matches!(op, crate::runtime::HostOperation::Read(_)))
    );
}

#[tokio::test]
async fn validation_retains_permission_then_destination_then_first_platform_error_order() {
    let root = std::env::temp_dir().join("shine-validation-error-order");
    let host = ObservationOnly(InMemoryHost::new());
    let path = root.join("app/demo/shine.toml");
    for (manifest, expected, message) in [
        (
            "dest = 'relative'\n[permission_defaults]\nschema_version = 1\n[[files]]\nsource = 'missing'\n",
            "invalid_permission_declaration",
            "one category-wide target",
        ),
        (
            "dest = { linux = '/valid', windows = 'relative' }\n[[files]]\nsource = 'missing'\n",
            "invalid_metadata",
            "invalid windows destination",
        ),
        (
            "dest = '~/.config/demo'\n[[files]]\nsource = 'missing'\n",
            "missing_reference",
            "for macos",
        ),
    ] {
        host.0.put_file(&path, manifest.as_bytes().to_vec());
        let report = validate_preset_path(&host, &root, &path).await;
        assert_eq!(report.summary.errors, 1, "{report:#?}");
        let diagnostics = &report.categories[0].diagnostics;
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, expected);
        assert!(diagnostics[0].message.contains(message), "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path.as_ref(), Some(&path));
    }
}
