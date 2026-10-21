//! Consumer regressions for read-only Shell inspection and its status precedence.
use crate::runtime::*;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Mutex;

type ObservationFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, HostError>> + Send + 'a>>;

// Deliberately provides no mutation, process, privilege or interaction ports.
struct ObservationHost {
    inner: InMemoryHost,
    fail_read: Option<(PathBuf, usize)>,
    fail_directory: Option<PathBuf>,
    reads: Mutex<Vec<PathBuf>>,
}

impl ObservationHost {
    fn new(inner: InMemoryHost) -> Self {
        Self {
            inner,
            fail_read: None,
            fail_directory: None,
            reads: Mutex::new(Vec::new()),
        }
    }
}

impl FileSystemObservationHost for ObservationHost {
    fn canonicalize<'a>(&'a self, path: &'a Path) -> ObservationFuture<'a, PathBuf> {
        self.inner.canonicalize(path)
    }

    fn read<'a>(&'a self, path: &'a Path) -> ObservationFuture<'a, Vec<u8>> {
        Box::pin(async move {
            let fail = {
                let mut reads = self.reads.lock().unwrap();
                let previous = reads.iter().filter(|read| read.as_path() == path).count();
                reads.push(path.to_path_buf());
                self.fail_read
                    .as_ref()
                    .is_some_and(|(target, after)| target == path && previous >= *after)
            };
            if fail {
                return Err(HostError::new(
                    std::io::ErrorKind::PermissionDenied,
                    anyhow::anyhow!("denied by inspection test host"),
                ));
            }
            self.inner.read(path).await
        })
    }

    fn metadata<'a>(&'a self, path: &'a Path) -> ObservationFuture<'a, FileMetadata> {
        self.inner.metadata(path)
    }

    fn read_dir<'a>(&'a self, path: &'a Path) -> ObservationFuture<'a, Vec<PathBuf>> {
        Box::pin(async move {
            if self.fail_directory.as_deref() == Some(path) {
                return Err(HostError::new(
                    std::io::ErrorKind::PermissionDenied,
                    anyhow::anyhow!("snapshot directory denied"),
                ));
            }
            self.inner.read_dir(path).await
        })
    }

    fn read_link<'a>(&'a self, path: &'a Path) -> ObservationFuture<'a, PathBuf> {
        self.inner.read_link(path)
    }
}

fn snapshot(source: PresetSourceKind, transformed: bool) -> PresetSnapshot {
    let transforms = if transformed {
        "transforms = ['template']"
    } else {
        ""
    };
    PresetSnapshot::builder(source)
        .file(
            "shell/demo/shine.toml",
            format!(
                "[[files]]\nsource = 'run.ts'\ntarget = 'run'\nruntime = 'bun'\n{transforms}\n[files.permissions]\nschema_version = 1\nenvironment = [{{ name = 'VALUE', sensitivity = 'plain' }}]\n\n[[files]]\nsource = 'available.ts'\ntarget = 'available'\nruntime = 'bun'\n[files.permissions]\nschema_version = 1\n"
            ).into_bytes(),
        )
        .file("shell/demo/run.ts", b"console.log('@@VALUE@@');\n".to_vec())
        .file("shell/demo/available.ts", b"console.log('available');\n".to_vec())
        .build()
}

async fn installed_runtime(transformed: bool) -> CoreRuntime<InMemoryHost> {
    let root = std::env::temp_dir().join("shine-shell-inspection-memory");
    let mut context = RuntimeContext::isolated(
        root.join("home"),
        root.join("state"),
        root.join("presets"),
        root.join("bin"),
        RuntimePlatform::current(),
    );
    context.env.insert("VALUE".into(), "captured".into());
    let runtime = CoreRuntime::new(
        InMemoryHost::new(),
        context,
        snapshot(PresetSourceKind::Embedded, transformed),
    );
    runtime
        .install_shells(ShellLifecycleRequest {
            target: Some("demo/run".into()),
            dry_run: false,
            force: false,
        })
        .await
        .unwrap();
    runtime
}

async fn inspect_read_only(runtime: &CoreRuntime<InMemoryHost>) -> Vec<ShellFileInspection> {
    let before = runtime.host().operations().len();
    let reader = CoreRuntime::new(
        ObservationHost::new(runtime.host().clone()),
        runtime.context().clone(),
        runtime.presets().clone(),
    );
    let rows = reader.inspect_shells().await.unwrap();
    assert!(
        runtime.host().operations()[before..]
            .iter()
            .all(|operation| matches!(operation, HostOperation::Read(_)))
    );
    rows
}

#[tokio::test]
async fn shell_inspection_keeps_change_order_and_receipt_only_rows_without_effects() {
    let runtime = installed_runtime(false).await;
    let mut manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    let source = manifest.entries[0].source_path.clone();
    let old_source = source.with_file_name("old.ts");
    let mut missing = manifest.entries[0].clone();
    missing.category = "removed".into();
    missing.command = "orphan".into();
    manifest.entries[0].source_path = old_source.clone();
    manifest.entries[0].mode = ExternalShellMode::Live;
    manifest.entries[0].transforms = vec!["template".into()];
    manifest.entries[0].env = vec!["OLD".into()];
    manifest.entries.push(missing);
    manifest
        .save(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    runtime
        .host()
        .put_file(&source, b"locally changed".to_vec());
    let manifest_path = runtime.context().shine_dir.join(SHELL_MANIFEST_FILE);
    let receipt_bytes = runtime.host().read(&manifest_path).await.unwrap();

    let rows = inspect_read_only(&runtime).await;
    assert_eq!(
        rows.iter()
            .map(|row| row.file.command_name.as_str())
            .collect::<Vec<_>>(),
        ["available", "run", "orphan"]
    );
    let available = &rows[0];
    assert_eq!(
        (available.status, available.status_text),
        (InspectionFileStatus::NotInstalled, "not installed")
    );
    assert!(!available.installed);
    assert!(available.changes.is_empty());
    let run = &rows[1];
    assert_eq!(
        (run.status, run.status_text),
        (InspectionFileStatus::UpdateAvail, "update available")
    );
    assert_eq!(run.source_path, source);
    assert_eq!(run.installed_source_path, source);
    assert_eq!(
        run.desired_content.as_deref(),
        Some(b"console.log('@@VALUE@@');\n".as_slice())
    );
    assert_eq!(
        run.current_content.as_deref(),
        Some(b"locally changed".as_slice())
    );
    assert_eq!(
        run.changes,
        vec![
            InspectionChange::ContentChanged,
            InspectionChange::SourceRelocated {
                from: old_source,
                to: source
            },
            InspectionChange::DeploymentChanged {
                field: "mode",
                from: "live".into(),
                to: "snapshot".into()
            },
            InspectionChange::DeploymentChanged {
                field: "transforms",
                from: "template".into(),
                to: "none".into()
            },
            InspectionChange::DeploymentChanged {
                field: "env",
                from: "OLD".into(),
                to: "none".into()
            },
        ]
    );
    let orphan = &rows[2];
    assert_eq!(
        (orphan.status, orphan.status_text),
        (
            InspectionFileStatus::Missing,
            "preset missing; installed entry preserved"
        )
    );
    assert!(orphan.installed && orphan.preset_missing && !orphan.link_conflict);
    assert!(orphan.category.files.is_empty() && !orphan.category.uses_metadata);
    assert!(orphan.file.permissions.is_none() && orphan.file.env.is_empty());
    assert!(orphan.desired_content.is_none() && orphan.current_content.is_none());
    assert!(orphan.changes.is_empty());
    assert_eq!(
        runtime.host().read(&manifest_path).await.unwrap(),
        receipt_bytes
    );
    let orphan_launcher = command_path_for_name(&runtime.context().bin_dir, "orphan".as_ref());
    runtime
        .host()
        .put_file(&orphan_launcher, b"foreign orphan".to_vec());
    let rows = inspect_read_only(&runtime).await;
    let orphan = rows.last().unwrap();
    assert_eq!(
        (orphan.status, orphan.status_text),
        (
            InspectionFileStatus::UserModified,
            "preset missing; launcher ownership conflict"
        )
    );
    assert!(orphan.preset_missing && orphan.link_conflict && orphan.installed);
    assert_eq!(
        runtime.host().read(&orphan_launcher).await.unwrap(),
        b"foreign orphan"
    );
    assert_eq!(
        runtime.host().read(&manifest_path).await.unwrap(),
        receipt_bytes
    );
}

#[tokio::test]
async fn shell_inspection_conflict_precedes_live_trust_and_trust_precedes_missing_launcher() {
    let original = installed_runtime(false).await;
    let mut context = original.context().clone();
    context.is_external_presets = true;
    context.external_shell_mode = ExternalShellMode::Live;
    let mut manifest = ShellManifest::load(original.host(), &context.shine_dir)
        .await
        .unwrap();
    manifest.entries[0].mode = ExternalShellMode::Live;
    manifest
        .save(original.host(), &context.shine_dir)
        .await
        .unwrap();
    let runtime = CoreRuntime::new(
        original.host().clone(),
        context,
        snapshot(PresetSourceKind::External, false),
    );
    let launcher = command_path_for_name(&runtime.context().bin_dir, "run".as_ref());
    let original_launcher = runtime.host().read(&launcher).await.unwrap();
    runtime
        .host()
        .put_file(&launcher, b"foreign launcher".to_vec());
    let rows = inspect_read_only(&runtime).await;
    let row = rows
        .iter()
        .find(|row| row.file.command_name == "run")
        .unwrap();
    assert_eq!(
        (row.status, row.status_text),
        (
            InspectionFileStatus::UserModified,
            "launcher ownership conflict"
        )
    );
    assert!(row.link_conflict && row.installed);
    assert!(row.changes.iter().any(|change| matches!(
        change,
        InspectionChange::DeploymentChanged { field: "trust", .. }
    )));
    assert!(
        !row.changes
            .iter()
            .any(|change| matches!(change, InspectionChange::CommandEntryOutdated { .. }))
    );
    assert_eq!(
        runtime.host().read(&launcher).await.unwrap(),
        b"foreign launcher"
    );

    runtime.host().put_file(&launcher, original_launcher);
    let rows = inspect_read_only(&runtime).await;
    let row = rows
        .iter()
        .find(|row| row.file.command_name == "run")
        .unwrap();
    assert_eq!(
        (row.status, row.status_text),
        (
            InspectionFileStatus::UpdateAvail,
            "development trust required"
        )
    );
    runtime.host().remove_file(&launcher).await.unwrap();
    let rows = inspect_read_only(&runtime).await;
    let row = rows
        .iter()
        .find(|row| row.file.command_name == "run")
        .unwrap();
    assert_eq!(row.status_text, "development trust required");
    assert!(
        row.changes
            .contains(&InspectionChange::CommandEntryMissing { path: launcher })
    );
}

#[tokio::test]
async fn shell_inspection_missing_rendered_status_yields_to_launcher_repairs() {
    let runtime = installed_runtime(true).await;
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    let entry = &manifest.entries[0];
    runtime
        .host()
        .remove_file(&entry.rendered_path)
        .await
        .unwrap();
    let rows = inspect_read_only(&runtime).await;
    let row = rows
        .iter()
        .find(|row| row.file.command_name == "run")
        .unwrap();
    assert_eq!(
        (row.status, row.status_text),
        (InspectionFileStatus::Missing, "rendered script missing")
    );
    assert_eq!(
        row.desired_content.as_deref(),
        Some(b"console.log('captured');\n".as_slice())
    );
    assert!(row.current_content.is_none());
    assert!(row.changes.is_empty());
    let launcher = command_path_for_name(&runtime.context().bin_dir, "run".as_ref());
    runtime.host().remove_file(&launcher).await.unwrap();
    let rows = inspect_read_only(&runtime).await;
    let row = rows
        .iter()
        .find(|row| row.file.command_name == "run")
        .unwrap();
    assert_eq!(
        (row.status, row.status_text),
        (InspectionFileStatus::UpdateAvail, "update available")
    );
    assert_eq!(
        row.changes,
        vec![InspectionChange::CommandEntryMissing { path: launcher }]
    );
}

#[tokio::test]
async fn shell_inspection_preserves_read_error_context_and_order() {
    let runtime = installed_runtime(true).await;
    let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    let entry = &manifest.entries[0];
    for (path, after, expected) in [
        (&entry.rendered_path, 0, "reading installed Shell content"),
        (&entry.source_path, 0, "reading deployed Shell source"),
        (&entry.rendered_path, 1, "reading rendered Shell source"),
    ] {
        let mut host = ObservationHost::new(runtime.host().clone());
        host.fail_read = Some((path.clone(), after));
        let reader = CoreRuntime::new(host, runtime.context().clone(), runtime.presets().clone());
        let error = reader.inspect_shells().await.unwrap_err();
        assert_eq!(error.to_string(), expected);
        let reads = reader.host().reads.lock().unwrap();
        assert_eq!(reads.last().unwrap(), path);
        assert_eq!(reads.iter().filter(|read| *read == path).count(), after + 1);
    }
}

#[tokio::test]
async fn shell_inspection_snapshot_observation_failure_stays_a_pending_update() {
    let original = installed_runtime(false).await;
    let mut context = original.context().clone();
    context.is_external_presets = true;
    let snapshot = snapshot(PresetSourceKind::External, false);
    let root = context.shine_dir.join("installed/shell/demo");
    for (logical, bytes) in snapshot.files() {
        original.host().put_file(
            root.join(logical.strip_prefix("shell/demo/").unwrap()),
            bytes.clone(),
        );
    }
    let mut manifest = ShellManifest::load(original.host(), &context.shine_dir)
        .await
        .unwrap();
    manifest.entries[0].source_path = root.join("run.ts");
    manifest
        .save(original.host(), &context.shine_dir)
        .await
        .unwrap();
    link_executables_with_host(
        original.host(),
        &context.bin_dir,
        &[LinkSpec {
            native_cmd_literal_path: false,
            live_launch_config: None,
            source: root.join("run.ts"),
            link_name: "run".into(),
            runtime: LinkRuntime::Bun,
            bun_dependencies: BunDependencyMode::Disabled,
            env: Vec::new(),
            render_target: None,
        }],
        true,
    )
    .await
    .unwrap();
    let mut host = ObservationHost::new(original.host().clone());
    host.fail_directory = Some(root);
    let runtime = CoreRuntime::new(host, context, snapshot);
    assert_eq!(
        runtime
            .shell_snapshot_current("demo")
            .await
            .unwrap_err()
            .to_string(),
        "observing planned Shell snapshot tree"
    );
    let rows = runtime.inspect_shells().await.unwrap();
    let run = rows
        .iter()
        .find(|row| row.file.command_name == "run")
        .unwrap();
    assert_eq!(
        (run.status, run.status_text),
        (InspectionFileStatus::UpdateAvail, "update available")
    );
    assert_eq!(run.desired_content, run.current_content);
    assert_eq!(
        run.changes,
        vec![InspectionChange::DeploymentChanged {
            field: "snapshot",
            from: "installed layout".into(),
            to: "active preset layout".into(),
        }]
    );
    assert!(
        rows.iter()
            .find(|row| row.file.command_name == "available")
            .unwrap()
            .changes
            .is_empty()
    );
}
