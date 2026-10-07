//! Execute App code and relative dependencies from one captured category.

use super::{
    ArtifactRuntime, CoreRuntime, FileSystemHost, ProcessHost, ProcessOutput, ProcessRequest,
};
use anyhow::{Context, Result, ensure};
use std::path::{Component, Path};

impl<H: FileSystemHost + ProcessHost> CoreRuntime<H> {
    pub(super) async fn run_app_script(
        &self,
        category: &str,
        logical: &str,
        runtime: ArtifactRuntime,
        mut request: ProcessRequest,
    ) -> Result<ProcessOutput> {
        validate_relative(category)?;
        ensure!(
            Path::new(category).components().count() == 1,
            "invalid App category"
        );
        let prefix = format!("app/{category}/");
        let relative_script = logical
            .strip_prefix(&prefix)
            .context("App script is outside its category")?;
        validate_relative(relative_script)?;
        ensure!(
            self.presets().file(logical).is_some(),
            "App script is missing: {logical}"
        );
        let dependency_arg = if runtime == ArtifactRuntime::Bun {
            Some(self.bun_dependency_arg(logical)?)
        } else {
            None
        };
        // Validate all paths before the first write. Unique invocation roots keep
        // concurrent operations and a previous script's writes out of this copy.
        for logical in self
            .presets()
            .files()
            .keys()
            .filter(|path| path.starts_with(&prefix))
        {
            validate_relative(&logical[prefix.len()..])?;
        }
        let invocation = self
            .context()
            .shine_dir
            .join("runtime/app")
            .join(category)
            .join(uuid::Uuid::new_v4().to_string());
        let app_dir = invocation.join("source");
        let result = async {
            self.host()
                .create_dir_all(&invocation)
                .await
                .map_err(|error| error.into_anyhow("creating App execution snapshot"))?;
            self.host()
                .set_mode(&invocation, 0o700)
                .await
                .map_err(|error| error.into_anyhow("protecting App execution snapshot"))?;
            for (logical, bytes) in self
                .presets()
                .files()
                .iter()
                .filter(|(path, _)| path.starts_with(&prefix))
            {
                let relative = &logical[prefix.len()..];
                let executable = self
                    .presets()
                    .file(logical)
                    .is_some_and(|file| file.executable);
                self.host()
                    .write_atomic(&app_dir.join(relative), bytes)
                    .await
                    .map_err(|error| error.into_anyhow("materializing App execution snapshot"))?;
                if executable {
                    self.host()
                        .set_executable(&app_dir.join(relative))
                        .await
                        .map_err(|error| {
                            error.into_anyhow("making App snapshot helper executable")
                        })?;
                }
                if self.presets().is_overlay(logical) {
                    self.host()
                        .write_atomic(&invocation.join("overlay").join(relative), bytes)
                        .await
                        .map_err(|error| error.into_anyhow("materializing App overlay snapshot"))?;
                    if executable {
                        self.host()
                            .set_executable(&invocation.join("overlay").join(relative))
                            .await
                            .map_err(|error| {
                                error.into_anyhow("making App overlay helper executable")
                            })?;
                    }
                }
            }
            let script = app_dir.join(relative_script);
            let mut args = if let Some(dependency_arg) = dependency_arg {
                request.program = "bun".to_string();
                vec![dependency_arg, script.display().to_string()]
            } else {
                self.host()
                    .set_executable(&script)
                    .await
                    .map_err(|error| error.into_anyhow("making App snapshot script executable"))?;
                request.program = script.display().to_string();
                Vec::new()
            };
            args.append(&mut request.args);
            request.args = args;
            request.cwd = Some(app_dir.clone());
            request
                .env
                .extend(self.fixed_app_contract_env(category, &app_dir));
            // Never inherit a caller-supplied overlay override when none was captured.
            if self.context().overlay_dir.is_none() {
                request
                    .env
                    .insert("SHINE_APP_OVERLAY_DIR".into(), String::new());
            }
            self.host().run(request).await
        }
        .await;
        let cleanup = self.host().remove_dir_all(&invocation).await;
        match (result, cleanup) {
            (Err(error), _) => Err(error),
            (Ok(output), Ok(())) => Ok(output),
            (Ok(_), Err(error)) => Err(error.into_anyhow("removing App execution snapshot")),
        }
    }
}

fn validate_relative(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty()
            && Path::new(path)
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
        "App snapshot path must stay inside its category"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn app_without_overlay_overrides_the_inherited_overlay_path() {
        const CHILD: &str = "SHINE_TEST_APP_OVERLAY_CHILD";
        if std::env::var_os(CHILD).is_none() {
            // Give only this subprocess an ambient overlay; no process-global env mutation.
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "runtime::app_script::tests::app_without_overlay_overrides_the_inherited_overlay_path",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env("SHINE_APP_OVERLAY_DIR", "/uncaptured/overlay")
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            return;
        }
        let root = std::env::temp_dir().join(format!("shine-app-overlay-{}", uuid::Uuid::new_v4()));
        let context = RuntimeContext::isolated(
            root.join("home"),
            root.join("state"),
            root.join("presets"),
            root.join("bin"),
            RuntimePlatform::current(),
        );
        let runtime = CoreRuntime::new(
            RealHost,
            context,
            PresetSnapshot::builder(PresetSourceKind::Embedded)
                .file(
                    "app/demo/probe.sh",
                    b"#!/bin/sh\nprintf '%s' \"$SHINE_APP_OVERLAY_DIR\"\n".to_vec(),
                )
                .build(),
        );
        let output = runtime
            .run_app_script(
                "demo",
                "app/demo/probe.sh",
                ArtifactRuntime::Native,
                ProcessRequest {
                    env: std::collections::BTreeMap::from([(
                        "SHINE_APP_OVERLAY_DIR".into(),
                        "/injected/overlay".into(),
                    )]),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(output.exit_code, Some(0));
        assert!(
            output.stdout.is_empty(),
            "uncaptured overlay reached the App script"
        );
    }

    #[tokio::test]
    async fn failed_spawn_cleans_only_its_own_snapshot() {
        let host = InMemoryHost::new();
        let root = std::env::temp_dir().join("shine-app-script-cleanup");
        let context = RuntimeContext::isolated(
            root.clone(),
            root.join("state"),
            root.join("presets"),
            root.join("bin"),
            RuntimePlatform::current(),
        );
        let retained = context.shine_dir.join("runtime/app/demo/other-run/keep");
        host.put_file(&retained, b"keep".to_vec());
        host.queue_process_output(Err(anyhow::anyhow!("spawn failure")));
        let runtime = CoreRuntime::new(
            host,
            context,
            PresetSnapshot::builder(PresetSourceKind::Embedded)
                .file("app/demo/build.ts", b"export {};".to_vec())
                .build(),
        );
        let error = runtime
            .run_app_script(
                "demo",
                "app/demo/build.ts",
                ArtifactRuntime::Bun,
                ProcessRequest::default(),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("spawn failure"));
        assert_eq!(runtime.host().read(&retained).await.unwrap(), b"keep");
        let runs = runtime
            .host()
            .read_dir(retained.parent().unwrap().parent().unwrap())
            .await
            .unwrap();
        assert_eq!(runs, [retained.parent().unwrap().to_path_buf()]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn app_entrypoints_and_helpers_execute_captured_bytes_after_source_changes() {
        use crate::trust::TrustGrantV1;
        use std::collections::BTreeSet;
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("shine-app-snapshot-{}", uuid::Uuid::new_v4()));
        let presets = root.join("presets");
        let category = presets.join("app/demo");
        std::fs::create_dir_all(category.join("scripts")).unwrap();
        std::fs::write(
            category.join("shine.toml"),
            r#"
metadata_schema_version = 2
dest = "~/.config/demo"
post_install = { script = "hook.sh" }
[artifact]
script = "build.sh"
teardown = "unbuild.sh"
[permissions]
schema_version = 1
environment = [{ name = "ENABLE", sensitivity = "plain" }]
[[files]]
source = "fallback.txt"
generator = { script = "scripts/generate.sh", env = ["ENABLE", "ENABLE=SHINE_APP_SOURCE_DIR", "ENABLE=SHINE_APP_ID"], when_env = "ENABLE" }
"#,
        )
        .unwrap();
        std::fs::write(category.join("fallback.txt"), "fallback").unwrap();
        std::fs::write(category.join("helper.sh"), "emit() { printf reviewed; }\n").unwrap();
        for (script, suffix) in [
            ("scripts/generate.sh", "emit"),
            (
                "hook.sh",
                "mkdir -p \"$SHINE_STATE_DIR\"; emit > \"$SHINE_STATE_DIR/hook\"",
            ),
            ("build.sh", "emit > \"$SHINE_STATE_DIR/build\""),
            ("unbuild.sh", "emit > \"$SHINE_STATE_DIR/unbuild\""),
        ] {
            std::fs::write(
                category.join(script),
                format!("#!/bin/sh\nset -e\n[ \"$SHINE_APP_ID\" = demo ] || exit 2\n[ ! -x fallback.txt ]\n[ \"$(./native-helper)\" = captured-base ]\n[ \"$(./overlay-helper)\" = captured-overlay ]\n[ \"$(\"$SHINE_APP_OVERLAY_DIR/overlay-helper\")\" = captured-overlay ]\n. \"$SHINE_APP_SOURCE_DIR/helper.sh\"\n. \"$SHINE_APP_OVERLAY_DIR/helper.sh\"\n{suffix}\n"),
            )
            .unwrap();
        }
        let overlay = root.join("overlay");
        std::fs::create_dir_all(overlay.join("app/demo")).unwrap();
        std::fs::write(
            overlay.join("app/demo/helper.sh"),
            "emit() { printf reviewed; }\n",
        )
        .unwrap();
        std::fs::write(category.join("helper.sh"), "emit() { printf shadowed; }\n").unwrap();
        for (path, text) in [
            (category.join("native-helper"), "captured-base"),
            (overlay.join("app/demo/overlay-helper"), "captured-overlay"),
        ] {
            std::fs::write(&path, format!("#!/bin/sh\nprintf {text}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(category.join("overlay-helper"), "shadowed non-executable").unwrap();
        let snapshot = capture_preset_snapshot(
            &RealHost,
            PresetSnapshotRequest {
                source: PresetSnapshotSource::External(presets.clone()),
                overlay_root: Some(overlay.clone()),
            },
        )
        .await
        .unwrap();
        // Changing source permissions after capture must not change execution either.
        for path in [
            category.join("native-helper"),
            overlay.join("app/demo/overlay-helper"),
        ] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        let mut context = RuntimeContext::isolated(
            root.join("home"),
            root.join("state"),
            presets,
            root.join("bin"),
            RuntimePlatform::current(),
        );
        context.is_external_presets = true;
        context.overlay_dir = Some(overlay.clone());
        context.env.insert("ENABLE".into(), "yes".into());
        let mut runtime = CoreRuntime::new(RealHost, context, snapshot);
        let requirements = runtime
            .external_code_requirements("app/demo")
            .await
            .unwrap();
        runtime.context_mut_for_cli().trust_grants = requirements
            .requirements
            .iter()
            .map(TrustGrantV1::for_reviewed_requirement)
            .collect();
        for script in [
            "scripts/generate.sh",
            "hook.sh",
            "build.sh",
            "unbuild.sh",
            "helper.sh",
        ] {
            std::fs::write(category.join(script), "#!/bin/sh\nprintf unreviewed\n").unwrap();
        }
        std::fs::write(
            overlay.join("app/demo/helper.sh"),
            "emit() { printf unreviewed; }\n",
        )
        .unwrap();
        let categories = runtime.app_categories(Some("demo")).unwrap();
        let generated = runtime
            .run_app_generator(
                AppGeneratorRequest {
                    category: "demo".into(),
                    source: "fallback.txt".into(),
                    generator: categories[0].files[0].generator.clone().unwrap(),
                    explicit: true,
                },
                &mut NullObserver,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(generated, b"reviewed");
        let hooks = runtime
            .run_app_hooks(
                AppHookRequest {
                    categories: categories.clone(),
                    changed: BTreeSet::from(["demo".into()]),
                    phase: AppHookPhase::PostInstall,
                    show_success: false,
                },
                &mut NullObserver,
            )
            .await;
        assert_eq!(
            hooks.outcomes[0].status,
            crate::lifecycle::LifecycleStatus::Changed
        );
        for action in [AppArtifactAction::Apply, AppArtifactAction::Remove] {
            runtime
                .run_app_artifact(
                    AppArtifactRequest {
                        category: "demo".into(),
                        artifact: categories[0].artifact.clone().unwrap(),
                        action,
                        implicit: true,
                        dry_run: false,
                    },
                    &mut NullObserver,
                )
                .await
                .unwrap();
        }
        for marker in ["hook", "build", "unbuild"] {
            assert_eq!(
                std::fs::read(root.join("state/state/app/demo").join(marker)).unwrap(),
                b"reviewed"
            );
        }
        assert_eq!(
            std::fs::read_dir(root.join("state/runtime/app/demo"))
                .unwrap()
                .count(),
            0
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
