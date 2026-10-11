//! Inputs.

use super::*;

pub(super) fn capture_generator_inputs(
    context: &crate::runtime::RuntimeContext,
    versions: &PlanningInputVersions,
    declaration: Option<&PermissionDeclarationV1>,
    generator: &crate::runtime::AppGenerator,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
) -> Result<()> {
    let sensitivity = declaration_sensitivity(declaration);
    let mut names = generator
        .env
        .iter()
        .map(|spec| spec.source.as_str())
        .collect::<BTreeSet<_>>();
    names.insert(&generator.when_env);
    for name in names {
        capture_env_identity(
            context,
            versions,
            name,
            sensitivity.get(name).copied(),
            state,
            permissions,
        )?;
    }
    Ok(())
}

pub(super) fn capture_app_artifact_inputs(
    context: &crate::runtime::RuntimeContext,
    versions: &PlanningInputVersions,
    declaration: Option<&PermissionDeclarationV1>,
    artifact: &crate::runtime::AppArtifact,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
) -> Result<()> {
    let sensitivity = declaration_sensitivity(declaration);
    for spec in &artifact.env {
        let declared_sensitivity = sensitivity.get(&spec.source).copied();
        if context.env.contains_key(&spec.source) {
            capture_env_identity(
                context,
                versions,
                &spec.source,
                declared_sensitivity,
                state,
                permissions,
            )?;
        } else {
            permissions.require(PermissionV1::Environment {
                name: spec.source.clone(),
                sensitivity: declared_sensitivity.unwrap_or(EnvironmentSensitivityV1::Plain),
            });
            state.public(format!("env:{}", spec.source), "missing")?;
        }
    }
    Ok(())
}

pub(super) fn capture_declared_env_inputs(
    context: &crate::runtime::RuntimeContext,
    versions: &PlanningInputVersions,
    declaration: Option<&PermissionDeclarationV1>,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
) -> Result<()> {
    for entry in declaration
        .into_iter()
        .flat_map(|declaration| &declaration.environment)
    {
        capture_env_identity(
            context,
            versions,
            &entry.name,
            Some(entry.sensitivity),
            state,
            permissions,
        )?;
    }
    Ok(())
}

pub(super) fn capture_app_hook_inputs(
    context: &crate::runtime::RuntimeContext,
    versions: &PlanningInputVersions,
    declaration: Option<&PermissionDeclarationV1>,
    hook: &crate::runtime::AppHook,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
) -> Result<()> {
    let sensitivity = declaration_sensitivity(declaration);
    for spec in &hook.env {
        let declared_sensitivity = sensitivity.get(&spec.source).copied();
        if context.env.contains_key(&spec.source) {
            capture_env_identity(
                context,
                versions,
                &spec.source,
                declared_sensitivity,
                state,
                permissions,
            )?;
        } else if matches!(&hook.action, crate::runtime::AppHookAction::Script { .. }) {
            permissions.require(PermissionV1::Environment {
                name: spec.source.clone(),
                sensitivity: declared_sensitivity.unwrap_or(EnvironmentSensitivityV1::Plain),
            });
            state.public(format!("env:{}", spec.source), "missing")?;
        } else {
            capture_env_identity(
                context,
                versions,
                &spec.source,
                declared_sensitivity,
                state,
                permissions,
            )?;
            permissions
                .uncomputable
                .insert("app_hook_env_missing".to_string());
        }
    }
    Ok(())
}

pub(super) fn capture_shell_inputs(
    context: &crate::runtime::RuntimeContext,
    versions: &PlanningInputVersions,
    file: &ShellFile,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
) -> Result<()> {
    let sensitivity = declaration_sensitivity(file.permissions.as_ref());
    for spec in &file.env {
        capture_env_identity(
            context,
            versions,
            &spec.source,
            sensitivity.get(&spec.source).copied(),
            state,
            permissions,
        )?;
    }
    Ok(())
}

pub(super) fn capture_sys_env(
    context: &crate::runtime::RuntimeContext,
    versions: &PlanningInputVersions,
    item: &SysItem,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
) -> Result<()> {
    let sensitivity = declaration_sensitivity(item.permissions.as_ref());
    let names = item
        .required_env
        .iter()
        .map(String::as_str)
        .chain(sensitivity.keys().map(String::as_str))
        .collect::<BTreeSet<_>>();
    for name in names {
        capture_env_identity(
            context,
            versions,
            name,
            sensitivity.get(name).copied(),
            state,
            permissions,
        )?;
    }
    Ok(())
}

pub(super) fn capture_env_identity(
    context: &crate::runtime::RuntimeContext,
    versions: &PlanningInputVersions,
    name: &str,
    sensitivity: Option<EnvironmentSensitivityV1>,
    state: &mut StateCapture,
    permissions: &mut PermissionAccumulator,
) -> Result<()> {
    let sensitivity = sensitivity.unwrap_or(EnvironmentSensitivityV1::Plain);
    permissions.require(PermissionV1::Environment {
        name: name.to_string(),
        sensitivity,
    });
    let value = match sensitivity {
        EnvironmentSensitivityV1::Plain => context
            .env
            .get(name)
            .map(|value| format!("plain:{}", sha256_hex(value.as_bytes())))
            .unwrap_or_else(|| "missing".to_string()),
        EnvironmentSensitivityV1::Secret => match versions.secret_versions.get(name) {
            Some(version) if !version.identity().is_empty() => format!(
                "secret-version:{}",
                sha256_hex(version.identity().as_bytes())
            ),
            Some(_) | None => {
                permissions
                    .uncomputable
                    .insert("secret_input_identity_unavailable".to_string());
                "secret-version:missing".to_string()
            }
        },
    };
    state.public(format!("env:{name}"), value)
}

pub(super) fn declaration_sensitivity(
    declaration: Option<&PermissionDeclarationV1>,
) -> BTreeMap<String, EnvironmentSensitivityV1> {
    declaration
        .into_iter()
        .flat_map(|declaration| &declaration.environment)
        .map(|entry| (entry.name.clone(), entry.sensitivity))
        .collect()
}
