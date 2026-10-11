//! Invocation-time preparation, separate from approved installation mutations.
use super::shell::{
    canonical_target, load_shell_manifest_with_host, shell_link_spec_from_manifest_entry,
};
use super::{BunDependencyMode, CoreRuntime, FileKind, FileSystemHost, PrivilegedFileSystemHost};
use anyhow::{Context, Result, bail};
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
pub struct LiveBunLaunch {
    pub script: PathBuf,
    pub dependencies: BunDependencyMode,
    pub env: Vec<String>,
}

pub fn validate_live_shell_target(target: &str) -> Result<()> {
    let parts = target.split('/').collect::<Vec<_>>();
    if parts.len() != 3
        || parts[0] != "shell"
        || parts[1..].iter().any(|part| {
            part.is_empty()
                || matches!(*part, "." | "..")
                || part.contains('\\')
                || part.chars().any(char::is_control)
        })
    {
        bail!("expected canonical shell/<category>/<command> target");
    }
    Ok(())
}

fn absolute_without_parent(path: &Path) -> bool {
    path.is_absolute() && !path.components().any(|part| part == Component::ParentDir)
}

impl<H: FileSystemHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    /// Capture and render one receipt while holding the lifecycle operation lock.
    /// The returned description owns no lock, configuration values, or runtime.
    pub async fn prepare_live_bun_launch(&self, target: &str) -> Result<LiveBunLaunch> {
        validate_live_shell_target(target)?;
        let _lifecycle_guard = self.acquire_shell_lifecycle_operation().await?;
        let _guard = self.host().acquire_privileged_operation().await?;
        if self.shell_operation_journal_bytes().await?.is_some() {
            bail!("an interrupted Shell operation requires explicit recovery");
        }
        let manifest =
            load_shell_manifest_with_host(self.host(), &self.context().shine_dir).await?;
        let mut entries = manifest
            .entries
            .iter()
            .filter(|entry| canonical_target(entry) == target);
        let entry = entries
            .next()
            .context("live shell command is not installed")?;
        if entries.next().is_some() {
            bail!("duplicate installed Shell target");
        }
        if entry.mode != super::ExternalShellMode::Live
            || entry.runtime != "bun"
            || entry.transforms.is_empty()
            || entry.needs_source
        {
            bail!("installed command is not a transformed Live Bun command");
        }
        let spec = shell_link_spec_from_manifest_entry(entry)?;
        if spec
            .live_launch_config
            .as_ref()
            .is_some_and(|root| root != &self.context().shine_dir)
        {
            bail!("Shell launcher installation directory does not match runtime");
        }
        crate::install::transforms::validate(&entry.transforms)?;
        crate::env::parse_env_specs(&entry.env)?;
        let rendered_root = self.context().shine_dir.join("rendered");
        if !absolute_without_parent(&entry.source_path)
            || !absolute_without_parent(&entry.rendered_path)
            || !entry.rendered_path.starts_with(&rendered_root)
            || entry.rendered_path == rendered_root
        {
            bail!("invalid recorded Live Shell source or rendered path");
        }
        let source = self
            .host()
            .metadata(&entry.source_path)
            .await
            .map_err(|error| error.into_anyhow("inspecting live source"))?;
        if source.kind != FileKind::File {
            bail!("Live Shell source must be a regular file");
        }
        // Selected state roots may have their established spelling. No symlink
        // inside the managed output subtree may redirect a rendered write.
        let mut path = self.context().shine_dir.clone();
        for part in entry
            .rendered_path
            .strip_prefix(&self.context().shine_dir)?
            .components()
        {
            path.push(part);
            match self.host().metadata(&path).await {
                Ok(metadata) if path == entry.rendered_path && metadata.kind == FileKind::File => {}
                Ok(metadata)
                    if path != entry.rendered_path && metadata.kind == FileKind::Directory => {}
                Ok(_) => bail!("Live Shell rendered path contains a non-regular destination"),
                Err(error) if error.is_not_found() => {}
                Err(error) => return Err(error.into_anyhow("inspecting live rendered path")),
            }
        }
        self.render_live_shell_entry(entry, target).await?;
        Ok(LiveBunLaunch {
            script: entry.rendered_path.clone(),
            dependencies: spec.bun_dependencies,
            env: entry.env.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{
        FileSystemObservationHost, HostOperation, InMemoryHost, PresetSnapshot, PresetSourceKind,
        RuntimeContext, RuntimePlatform, ShellManifest, ShellManifestEntry,
    };

    fn effects(host: &InMemoryHost) -> Vec<HostOperation> {
        host.operations()
            .into_iter()
            .filter(|operation| {
                !matches!(
                    operation,
                    HostOperation::Read(_)
                        | HostOperation::AcquirePrivilegedOperation
                        | HostOperation::AcquireOperationLock(_)
                )
            })
            .collect()
    }

    fn fixture() -> (CoreRuntime<InMemoryHost>, ShellManifestEntry) {
        let root = std::env::temp_dir().join("shine-live-launch-fixture");
        let host = InMemoryHost::new();
        let mut context = RuntimeContext::isolated(
            root.join("home"),
            root.join(".shine"),
            root.join("presets"),
            root.join(".shine/bin"),
            RuntimePlatform::current(),
        );
        context.env.insert("VALUE".into(), "current".into());
        let entry = ShellManifestEntry {
            launcher_format: Some("live-bun-v2".into()),
            launcher_config_dir: Some(context.shine_dir.clone()),
            category: "demo".into(),
            command: "run".into(),
            mode: super::super::ExternalShellMode::Live,
            source_path: root.join("external/run.ts"),
            rendered_path: context.shine_dir.join("rendered/shell/demo/run.ts"),
            runtime: "bun".into(),
            bun_dependencies: None,
            dependency_hash: None,
            transforms: vec!["template".into()],
            env: vec!["TOKEN=ALIAS".into()],
            needs_source: false,
            content_hash: 0,
        };
        host.put_file(&entry.source_path, b"@@VALUE@@".to_vec());
        let runtime = CoreRuntime::new(
            host,
            context,
            PresetSnapshot::builder(PresetSourceKind::External).build(),
        );
        (runtime, entry)
    }

    async fn save(runtime: &CoreRuntime<InMemoryHost>, entries: Vec<ShellManifestEntry>) {
        ShellManifest {
            entries,
            ..ShellManifest::default()
        }
        .save(runtime.host(), &runtime.context().shine_dir)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn live_bun_prepare_uses_receipt_and_does_not_rewrite_identical_output() {
        let (runtime, entry) = fixture();
        save(&runtime, vec![entry.clone()]).await;
        let launch = runtime
            .prepare_live_bun_launch("shell/demo/run")
            .await
            .unwrap();
        assert_eq!(launch.env, ["TOKEN=ALIAS"]);
        assert_eq!(launch.dependencies, BunDependencyMode::Disabled);
        assert_eq!(
            runtime.host().read(&launch.script).await.unwrap(),
            b"current"
        );
        let mut locked = entry.clone();
        locked.bun_dependencies = Some("locked".into());
        locked.dependency_hash = Some(123);
        save(&runtime, vec![locked]).await;
        assert_eq!(
            runtime
                .prepare_live_bun_launch("shell/demo/run")
                .await
                .unwrap()
                .dependencies,
            BunDependencyMode::Locked
        );
        let before = effects(runtime.host());
        runtime
            .prepare_live_bun_launch("shell/demo/run")
            .await
            .unwrap();
        assert_eq!(effects(runtime.host()), before);
        runtime
            .host()
            .put_file(&entry.source_path, b"@@MISSING@@".to_vec());
        assert!(
            runtime
                .prepare_live_bun_launch("shell/demo/run")
                .await
                .is_err()
        );
        assert_eq!(
            runtime.host().read(&launch.script).await.unwrap(),
            b"current"
        );
    }

    #[tokio::test]
    async fn live_bun_invalid_receipts_fail_before_rendering() {
        let (runtime, entry) = fixture();
        for case in 0..12 {
            let mut invalid = entry.clone();
            match case {
                0 => invalid.runtime = "native".into(),
                1 => invalid.mode = super::super::ExternalShellMode::Snapshot,
                2 => invalid.needs_source = true,
                3 => invalid.transforms.clear(),
                4 => invalid.transforms = vec!["unknown".into()],
                5 => invalid.env = vec!["A=X".into(), "B=X".into()],
                6 => invalid.env = vec!["BAD-NAME".into()],
                7 => invalid.bun_dependencies = Some("unknown".into()),
                8 => invalid.launcher_format = Some("unknown".into()),
                9 => {
                    invalid.rendered_path =
                        runtime.context().shine_dir.join("rendered/../escape.ts")
                }
                10 => invalid.source_path = PathBuf::from("relative.ts"),
                11 => invalid.launcher_config_dir = Some(runtime.context().shine_dir.join("other")),
                _ => unreachable!(),
            }
            save(&runtime, vec![invalid]).await;
            let before = effects(runtime.host());
            assert!(
                runtime
                    .prepare_live_bun_launch("shell/demo/run")
                    .await
                    .is_err(),
                "case {case}"
            );
            assert_eq!(effects(runtime.host()), before, "case {case} wrote state");
        }
        save(&runtime, vec![entry.clone(), entry.clone()]).await;
        assert!(
            runtime
                .prepare_live_bun_launch("shell/demo/run")
                .await
                .is_err()
        );
        save(&runtime, vec![entry]).await;
        for target in [
            "demo/run",
            "shell/demo",
            "shell//run",
            "shell/../run",
            "shell/demo/.",
            "shell/demo/run/extra",
            "shell/demo/\\run",
            "shell/demo/\nrun",
        ] {
            assert!(
                runtime.prepare_live_bun_launch(target).await.is_err(),
                "{target:?}"
            );
        }
        runtime.host().put_file(
            runtime
                .context()
                .shine_dir
                .join(super::super::SHELL_OPERATION_JOURNAL_FILE),
            b"corrupt journal".to_vec(),
        );
        let before = effects(runtime.host());
        assert!(
            runtime
                .prepare_live_bun_launch("shell/demo/run")
                .await
                .is_err()
        );
        assert_eq!(effects(runtime.host()), before);
        assert!(
            before
                .iter()
                .all(|operation| !matches!(operation, HostOperation::Run { .. }))
        );
    }

    #[tokio::test]
    async fn live_bun_rendered_symlink_cannot_redirect_output() {
        let (runtime, entry) = fixture();
        save(&runtime, vec![entry.clone()]).await;
        runtime
            .host()
            .symlink(
                &runtime.context().shine_dir.join("outside"),
                entry.rendered_path.parent().unwrap(),
            )
            .await
            .unwrap();
        let before = effects(runtime.host());
        assert!(
            runtime
                .prepare_live_bun_launch("shell/demo/run")
                .await
                .is_err()
        );
        assert_eq!(effects(runtime.host()), before);
    }

    #[tokio::test]
    async fn live_bun_legacy_format_upgrade_is_transactional_and_recoverable() {
        use super::super::launcher::{
            apply_prepared_launcher_resource, prepare_launcher_resources,
            prepared_launcher_resource_is_exact,
        };
        use crate::lifecycle::LifecycleOperation;
        use crate::plan::PlanApprovalV1;
        use crate::runtime::{PlanningInputVersions, ShellPlanRequest};
        let (base, _) = fixture();
        let mut context = base.context().clone();
        context.is_external_presets = true;
        context.external_shell_mode = super::super::ExternalShellMode::Live;
        let metadata = b"[permission_defaults]\nschema_version = 2\nopaque_code = 'unrestricted'\nenvironment = [{ name = 'VALUE', sensitivity = 'plain' }]\n[[files]]\nsource = 'run.ts'\ntarget = 'run'\nruntime = 'bun'\ntransforms = ['template']\nenv = ['VALUE']\n";
        let snapshot = PresetSnapshot::builder(PresetSourceKind::External)
            .base_root(&context.presets_dir)
            .file("shell/demo/shine.toml", metadata.to_vec())
            .file("shell/demo/run.ts", b"@@VALUE@@".to_vec())
            .build();
        base.host().put_file(
            context.presets_dir.join("shell/demo/shine.toml"),
            metadata.to_vec(),
        );
        base.host().put_file(
            context.presets_dir.join("shell/demo/run.ts"),
            b"@@VALUE@@".to_vec(),
        );
        let mut runtime = CoreRuntime::new(base.host().clone(), context, snapshot);
        runtime.context_mut_for_cli().trust_grants = runtime
            .external_code_requirements("shell/demo/run")
            .await
            .unwrap()
            .requirements
            .iter()
            .map(|requirement| {
                crate::trust::TrustGrantV1::for_development_requirement(requirement).unwrap()
            })
            .collect();
        let mut request = ShellPlanRequest {
            operation: LifecycleOperation::Install,
            target: Some("demo/run".into()),
            force: false,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        };
        let plan = runtime.plan_shells(request.clone()).await.unwrap();
        assert!(plan.is_ready(), "{plan:?}");
        runtime
            .install_shells_approved(
                request.clone(),
                &PlanApprovalV1::for_reviewed_plan(&plan).unwrap(),
            )
            .await
            .unwrap();
        let mut manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap();
        let old = &mut manifest.entries[0];
        old.launcher_format = None;
        old.launcher_config_dir = None;
        let old_resources = prepare_launcher_resources(
            &runtime.context().bin_dir,
            &shell_link_spec_from_manifest_entry(old).unwrap(),
        );
        for resource in &old_resources {
            apply_prepared_launcher_resource(runtime.host(), resource)
                .await
                .unwrap();
        }
        manifest.schema_version = 1;
        let manifest_path = runtime.context().shine_dir.join("shell-manifest.toml");
        runtime.host().put_file(
            &manifest_path,
            toml::to_string(&manifest).unwrap().into_bytes(),
        );
        request.operation = LifecycleOperation::Upgrade;
        let plan = runtime.plan_shells(request.clone()).await.unwrap();
        assert!(plan.is_ready(), "{plan:?}");
        assert!(
            plan.steps.iter().any(|step| step
                .diagnostic_codes
                .iter()
                .any(|code| code == "shell_managed_launcher_update_transaction")),
            "{plan:?}"
        );
        runtime.host().fail_write_after(&manifest_path, 0);
        assert!(
            runtime
                .upgrade_shells_approved(
                    request.clone(),
                    &PlanApprovalV1::for_reviewed_plan(&plan).unwrap()
                )
                .await
                .is_err()
        );
        assert!(
            runtime
                .prepare_live_bun_launch("shell/demo/run")
                .await
                .is_err()
        );
        let recovery = runtime.plan_shell_operation_recovery().await.unwrap();
        assert!(recovery.is_ready(), "{recovery:?}");
        runtime
            .recover_shell_operation_approved(
                &PlanApprovalV1::for_reviewed_plan(&recovery).unwrap(),
            )
            .await
            .unwrap();
        for resource in &old_resources {
            assert!(
                prepared_launcher_resource_is_exact(runtime.host(), resource)
                    .await
                    .unwrap()
            );
        }
        let plan = runtime.plan_shells(request.clone()).await.unwrap();
        runtime
            .upgrade_shells_approved(
                request.clone(),
                &PlanApprovalV1::for_reviewed_plan(&plan).unwrap(),
            )
            .await
            .unwrap();
        let manifest = ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
            .await
            .unwrap();
        assert_eq!(
            manifest.entries[0].launcher_format.as_deref(),
            Some("live-bun-v2")
        );
        let plan = runtime.plan_shells(request).await.unwrap();
        assert!(
            plan.steps.iter().all(|step| step.target != "shell/demo/run"
                || step.action != crate::plan::PlanActionV1::Update),
            "{plan:?}"
        );
        let uninstall = ShellPlanRequest {
            operation: LifecycleOperation::Uninstall,
            target: Some("demo/run".into()),
            force: false,
            purge: false,
            input_versions: PlanningInputVersions::default(),
        };
        let plan = runtime.plan_shells(uninstall.clone()).await.unwrap();
        runtime
            .uninstall_shells_approved(
                uninstall.clone(),
                &PlanApprovalV1::for_reviewed_plan(&plan).unwrap(),
            )
            .await
            .unwrap();
        assert!(
            ShellManifest::load(runtime.host(), &runtime.context().shine_dir)
                .await
                .unwrap()
                .entries
                .is_empty()
        );
    }
}
