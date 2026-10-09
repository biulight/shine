//! App code.

use super::*;

pub(super) fn select_refresh_files(
    category: &AppCategory,
    selector: Option<&Path>,
) -> Result<Vec<AppFile>> {
    let candidates = if let Some(selector) = selector {
        let file = category
            .files
            .iter()
            .find(|file| file.source_rel == selector)
            .with_context(|| {
                format!(
                    "app '{}' file not found: {}",
                    category.name,
                    selector.display()
                )
            })?;
        if file.generator.is_none() {
            bail!(
                "app '{}' file is not generated: {}",
                category.name,
                selector.display()
            );
        }
        vec![file.clone()]
    } else {
        category
            .files
            .iter()
            .filter(|file| file.generator.is_some())
            .cloned()
            .collect::<Vec<_>>()
    };
    if candidates.is_empty() {
        bail!("app '{}' has no generated files", category.name);
    }
    Ok(candidates)
}

pub(super) async fn plan_app_hooks<H: FileSystemObservationHost>(
    runtime: &CoreRuntime<H>,
    category: &AppCategory,
    hooks: &[crate::runtime::AppHook],
    input_versions: &PlanningInputVersions,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
    steps: &mut Vec<PlanStepV1>,
) -> Result<()> {
    for (index, hook) in hooks.iter().enumerate() {
        state.app_code(category, CodeEntryKindV2::AppHook);
        if is_recursive_app_artifact_hook(hook, &category.name) {
            steps.push(
                PlanStepV1::new(
                    format!("app/{}", category.name),
                    Some(format!("hook:{index}")),
                    PlanActionV1::Blocked,
                )
                .with_diagnostic_code(if category.metadata_schema_version >= 2 {
                    "app_recursive_artifact_hook_unsupported"
                } else if category.metadata_is_overlay {
                    "app_legacy_overlay_metadata"
                } else {
                    "app_legacy_metadata"
                }),
            );
            continue;
        }
        permissions.declaration(
            category.permissions.as_ref(),
            "app_permission_declaration_missing",
        );
        capture_app_hook_inputs(
            runtime.context(),
            input_versions,
            category.permissions.as_ref(),
            hook,
            state,
            permissions,
        )?;
        let snapshot_cleanup =
            add_app_hook_permissions(runtime, category, hook, index, state, permissions, steps)
                .await?;
        let blocked = !runtime.app_capability_trusted(category, TrustCapabilityV1::AppHook)?;
        steps.push(
            PlanStepV1::new(
                format!("app/{}", category.name),
                Some(format!("hook:{index}")),
                if blocked {
                    PlanActionV1::Blocked
                } else {
                    PlanActionV1::Execute
                },
            )
            .with_diagnostic_code(if blocked {
                "app_external_code_not_allowed"
            } else {
                "app_hook_execution"
            }),
        );
        steps.extend(snapshot_cleanup);
    }
    Ok(())
}

pub(super) fn is_recursive_app_artifact_hook(
    hook: &crate::runtime::AppHook,
    category: &str,
) -> bool {
    matches!(&hook.action, crate::runtime::AppHookAction::Command(command) if command == "shine")
        && hook.args.len() >= 3
        && hook.args[0] == "app"
        && hook.args[1] == "artifact"
        && hook.args[2] == "apply"
        && hook.args.get(3).is_some_and(|target| target == category)
}

pub(super) async fn add_app_hook_permissions<H: FileSystemObservationHost>(
    runtime: &CoreRuntime<H>,
    category: &AppCategory,
    hook: &crate::runtime::AppHook,
    index: usize,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
    steps: &mut Vec<PlanStepV1>,
) -> Result<Option<PlanStepV1>> {
    let crate::runtime::AppHookAction::Script {
        script,
        runtime: runtime_kind,
    } = &hook.action
    else {
        if let crate::runtime::AppHookAction::Command(command) = &hook.action {
            permissions.require(PermissionV1::Command {
                program: command.clone(),
            });
        }
        return Ok(None);
    };

    permissions.require(PermissionV1::Filesystem {
        access: FilesystemAccessV1::Execute,
        path: format!("preset:{}", script.display().to_string().replace('\\', "/")),
    });
    if *runtime_kind == ArtifactRuntime::Bun {
        permissions.require(PermissionV1::Command {
            program: "bun".to_string(),
        });
    }
    let logical = format!("app/{}/{}", category.name, script.display());
    runtime
        .presets()
        .file(&logical)
        .with_context(|| format!("app hook script is missing: {logical}"))?;
    let cleanup = add_app_snapshot_permissions(
        runtime,
        category,
        &format!("hook:{index}:snapshot"),
        state,
        permissions,
        steps,
    )
    .await?;
    Ok(Some(cleanup))
}

pub(super) async fn add_app_artifact_permissions<H: FileSystemObservationHost>(
    runtime: &CoreRuntime<H>,
    category: &AppCategory,
    script: &str,
    runtime_kind: ArtifactRuntime,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
    steps: &mut Vec<PlanStepV1>,
) -> Result<PlanStepV1> {
    state.app_code(category, CodeEntryKindV2::AppArtifact);
    permissions.require(PermissionV1::Filesystem {
        access: FilesystemAccessV1::Execute,
        path: format!("preset:{}", script.replace('\\', "/")),
    });
    if runtime_kind == ArtifactRuntime::Bun {
        permissions.require(PermissionV1::Command {
            program: "bun".to_string(),
        });
    }
    let logical = format!("app/{}/{script}", category.name);
    runtime
        .presets()
        .file(&logical)
        .with_context(|| format!("app script is missing: {logical}"))?;
    let cleanup = add_app_snapshot_permissions(
        runtime,
        category,
        "artifact:snapshot",
        state,
        permissions,
        steps,
    )
    .await?;
    for (label, directory) in [
        (
            "http-dir",
            runtime
                .context()
                .shine_dir
                .join("http")
                .join("app")
                .join(&category.name),
        ),
        (
            "cache-dir",
            runtime
                .context()
                .cache_dir
                .join("shine")
                .join("app")
                .join(&category.name),
        ),
        (
            "state-dir",
            runtime
                .context()
                .shine_dir
                .join("state")
                .join("app")
                .join(&category.name),
        ),
    ] {
        let exists = path_exists(runtime.host(), &directory).await?;
        capture_path_state(
            runtime.host(),
            state,
            format!("artifact:{label}"),
            &directory,
        )
        .await?;
        permissions.implicit_for(
            PermissionV1::Filesystem {
                access: FilesystemAccessV1::Write,
                path: review_path(runtime.context(), &directory),
            },
            FilesystemPurposeV1::Installation,
            format!("app/{}", category.name),
        );
        if !exists {
            steps.push(PlanStepV1::new(
                format!("app/{}", category.name),
                Some(format!("artifact:{label}")),
                PlanActionV1::Create,
            ));
        }
    }
    Ok(cleanup)
}

pub(super) async fn add_generator_permissions<H: FileSystemObservationHost>(
    runtime: &CoreRuntime<H>,
    permissions: &mut PermissionAccumulator,
    category: &AppCategory,
    generator: &crate::runtime::AppGenerator,
    state: &mut StateCapture,
    steps: &mut Vec<PlanStepV1>,
) -> Result<PlanStepV1> {
    state.app_code(category, CodeEntryKindV2::AppGenerator);
    permissions.require(PermissionV1::Filesystem {
        access: FilesystemAccessV1::Execute,
        path: format!("preset:{}", generator.script.display()),
    });
    if generator.runtime == ArtifactRuntime::Bun {
        permissions.require(PermissionV1::Command {
            program: "bun".to_string(),
        });
    }
    let logical = format!("app/{}/{}", category.name, generator.script.display());
    runtime
        .presets()
        .file(&logical)
        .with_context(|| format!("app generator script is missing: {logical}"))?;
    add_app_snapshot_permissions(
        runtime,
        category,
        &format!("generator-runtime:{}", generator.script.display()),
        state,
        permissions,
        steps,
    )
    .await
}

pub(super) async fn add_app_snapshot_permissions<H: FileSystemObservationHost>(
    runtime: &CoreRuntime<H>,
    category: &AppCategory,
    resource: &str,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
    steps: &mut Vec<PlanStepV1>,
) -> Result<PlanStepV1> {
    // Execution creates a fresh UUID child and removes only that child. No
    // existing category tree is replaced or removed, and no random ID is review data.
    let root = runtime
        .context()
        .shine_dir
        .join("runtime/app")
        .join(&category.name);
    capture_path_state(
        runtime.host(),
        state,
        format!("app/{}:{resource}:root", category.name),
        &root,
    )
    .await?;
    for access in [FilesystemAccessV1::Write, FilesystemAccessV1::Remove] {
        permissions.implicit_for(
            PermissionV1::Filesystem {
                access,
                path: review_path(runtime.context(), &root),
            },
            FilesystemPurposeV1::Maintenance,
            format!("app/{}", category.name),
        );
    }
    steps.push(
        PlanStepV1::new(
            format!("app/{}", category.name),
            Some(resource),
            PlanActionV1::Create,
        )
        .with_diagnostic_code("app_execution_snapshot_materialization"),
    );
    Ok(PlanStepV1::new(
        format!("app/{}", category.name),
        Some(format!("{resource}:cleanup")),
        PlanActionV1::Remove,
    )
    .with_diagnostic_code("app_execution_snapshot_cleanup"))
}

pub(super) fn app_code_blocked<H>(
    runtime: &CoreRuntime<H>,
    category: &AppCategory,
    script: &Path,
) -> Result<bool> {
    let logical = format!("app/{}/{}", category.name, script.display());
    if runtime
        .presets()
        .origin(&logical)
        .is_none_or(|origin| origin.source_kind == crate::runtime::PresetSourceKind::Embedded)
    {
        return Ok(false);
    }
    let script = script.to_string_lossy();
    let capability = if category.artifact.as_ref().is_some_and(|artifact| {
        artifact.script == script || artifact.teardown.as_deref() == Some(script.as_ref())
    }) {
        TrustCapabilityV1::AppArtifact
    } else {
        TrustCapabilityV1::AppGenerator
    };
    Ok(!runtime.app_capability_trusted(category, capability)?)
}
