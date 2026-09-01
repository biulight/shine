use super::{AppCategory, CoreRuntime, PresetSourceKind, SysInstall, SysItem};
use crate::permission::PermissionDeclarationV1;
use crate::plan::{FilesystemAccessV1, PermissionSetV1, PermissionV1};
use crate::trust::{TrustCapabilityV1, TrustDecisionV1, TrustRequirementV1, evaluate_trust};
use anyhow::{Context, Result, bail};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalCodeRequirementReport {
    pub requirements: Vec<TrustRequirementV1>,
}

impl<H> CoreRuntime<H> {
    pub async fn external_code_requirements(
        &self,
        target: &str,
    ) -> Result<ExternalCodeRequirementReport> {
        if let Some(category_name) = target.strip_prefix("app/") {
            let category = self
                .app_categories(Some(category_name))?
                .into_iter()
                .find(|category| category.name == category_name)
                .with_context(|| format!("app preset category not found: {category_name}"))?;
            return Ok(ExternalCodeRequirementReport {
                requirements: self.app_external_code_requirements(&category)?,
            });
        }
        if let Some(item_id) = target.strip_prefix("sys/") {
            let os_id = match self.context().platform {
                super::RuntimePlatform::Macos => "macos",
                super::RuntimePlatform::Linux => "ubuntu",
                super::RuntimePlatform::Windows => "windows",
            };
            let loaded = self.load_sys_preset(os_id).await?;
            let item = loaded
                .manifest
                .items
                .iter()
                .find(|item| item.id == item_id)
                .with_context(|| format!("unknown sys item `{item_id}` for {os_id}"))?;
            return Ok(ExternalCodeRequirementReport {
                requirements: self.sys_external_code_requirements(os_id, item)?,
            });
        }
        bail!("trust target must be canonical app/<category> or sys/<item>: {target}")
    }

    pub(crate) fn app_external_code_requirements(
        &self,
        category: &AppCategory,
    ) -> Result<Vec<TrustRequirementV1>> {
        let target = format!("app/{}", category.name);
        let prefix = format!("app/{}/", category.name);
        let permissions = declared_permissions(category.permissions.as_ref())?;
        let generator_paths = category
            .files
            .iter()
            .filter_map(|file| file.generator.as_ref())
            .map(|generator| {
                format!(
                    "app/{}/{}",
                    category.name,
                    generator.script.to_string_lossy().replace('\\', "/")
                )
            })
            .collect::<Vec<_>>();
        let mut explicit_paths = generator_paths.clone();
        if let Some(artifact) = &category.artifact {
            explicit_paths.push(format!(
                "app/{}/{}",
                category.name,
                artifact.script.replace('\\', "/")
            ));
            if let Some(teardown) = &artifact.teardown {
                explicit_paths.push(format!(
                    "app/{}/{}",
                    category.name,
                    teardown.replace('\\', "/")
                ));
            }
        }
        let category_paths = code_paths(self, &prefix, &permissions, explicit_paths);
        let mut output = Vec::new();

        if (!category.post_install.is_empty() || !category.post_upgrade.is_empty())
            && any_external(self, category_paths.iter().copied())
        {
            output.push(self.requirement(
                &target,
                TrustCapabilityV1::AppHook,
                category_paths.iter().copied(),
                permissions.clone(),
            )?);
        }

        if !generator_paths.is_empty() && any_external(self, category_paths.iter().copied()) {
            output.push(self.requirement(
                &target,
                TrustCapabilityV1::AppGenerator,
                category_paths.iter().copied(),
                permissions.clone(),
            )?);
        }

        if category.artifact.is_some() && any_external(self, category_paths.iter().copied()) {
            output.push(self.requirement(
                &target,
                TrustCapabilityV1::AppArtifact,
                category_paths.iter().copied(),
                permissions,
            )?);
        }
        Ok(output)
    }

    pub(crate) fn sys_external_code_requirements(
        &self,
        os_id: &str,
        item: &SysItem,
    ) -> Result<Vec<TrustRequirementV1>> {
        let target = format!("sys/{}", item.id);
        let prefix = format!("sys/{os_id}/");
        let permissions = declared_permissions(item.permissions.as_ref())?;
        let mut explicit_paths = Vec::new();
        if let Some(SysInstall::Script { path, .. }) = &item.install {
            explicit_paths.push(format!("sys/{os_id}/{}", path.replace('\\', "/")));
        }
        for integration in &item.shell {
            for path in [
                integration.source.as_deref(),
                integration.fragment.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                explicit_paths.push(format!("sys/{os_id}/{}", path.replace('\\', "/")));
            }
        }
        let category_paths = code_paths(self, &prefix, &permissions, explicit_paths);
        let mut output = Vec::new();
        if matches!(&item.install, Some(SysInstall::Script { .. }))
            && any_external(self, category_paths.iter().copied())
        {
            output.push(self.requirement(
                &target,
                TrustCapabilityV1::SysBootstrapScript,
                category_paths.iter().copied(),
                permissions.clone(),
            )?);
        }

        let executable_integration = item.shell.iter().any(|integration| {
            !integration.eval_argv.is_empty()
                || integration.source.is_some()
                || integration.fragment.is_some()
        });
        if (executable_integration || external_profile_base(self, os_id))
            && (any_external(self, category_paths.iter().copied())
                || (executable_integration && self.context().is_external_presets))
        {
            output.push(self.requirement(
                &target,
                TrustCapabilityV1::SysProfileCode,
                category_paths.iter().copied(),
                permissions,
            )?);
        }
        Ok(output)
    }

    pub(crate) fn trust_decision(&self, requirement: &TrustRequirementV1) -> TrustDecisionV1 {
        evaluate_trust(&self.context().trust_grants, requirement)
    }

    pub(crate) fn app_capability_trusted(
        &self,
        category: &AppCategory,
        capability: TrustCapabilityV1,
    ) -> Result<bool> {
        let requirements = self.app_external_code_requirements(category)?;
        Ok(requirements
            .iter()
            .filter(|requirement| requirement.capability == capability)
            .all(|requirement| self.trust_decision(requirement) == TrustDecisionV1::Trusted))
    }

    pub(crate) fn sys_capability_trusted(
        &self,
        os_id: &str,
        item: &SysItem,
        capability: TrustCapabilityV1,
    ) -> Result<bool> {
        let requirements = self.sys_external_code_requirements(os_id, item)?;
        Ok(requirements
            .iter()
            .filter(|requirement| requirement.capability == capability)
            .all(|requirement| self.trust_decision(requirement) == TrustDecisionV1::Trusted))
    }

    fn requirement<'a>(
        &self,
        target: &str,
        capability: TrustCapabilityV1,
        paths: impl IntoIterator<Item = &'a str>,
        permissions: PermissionSetV1,
    ) -> Result<TrustRequirementV1> {
        let paths = paths.into_iter().collect::<BTreeSet<_>>();
        Ok(TrustRequirementV1 {
            target: target.to_string(),
            capability,
            code_digest: self.presets().code_digest_v1(paths)?,
            permissions,
        })
    }
}

fn declared_permissions(declaration: Option<&PermissionDeclarationV1>) -> Result<PermissionSetV1> {
    Ok(declaration
        .map(PermissionDeclarationV1::permission_set)
        .transpose()?
        .unwrap_or_default())
}

fn is_external<H>(runtime: &CoreRuntime<H>, logical: &str) -> bool {
    runtime
        .presets()
        .origin(logical)
        .is_some_and(|origin| origin.source_kind != PresetSourceKind::Embedded)
}

fn any_external<'a, H>(runtime: &CoreRuntime<H>, paths: impl IntoIterator<Item = &'a str>) -> bool {
    paths.into_iter().any(|path| is_external(runtime, path))
}

fn external_profile_base<H>(runtime: &CoreRuntime<H>, os_id: &str) -> bool {
    let ext = if os_id == "windows" { "ps1" } else { "sh" };
    ["pre", "post"].into_iter().any(|phase| {
        let logical = format!("sys/{os_id}/profile/base.{phase}.{ext}");
        runtime.presets().get(&logical).is_some() && is_external(runtime, &logical)
    })
}

fn code_paths<'a, H>(
    runtime: &'a CoreRuntime<H>,
    prefix: &str,
    permissions: &PermissionSetV1,
    explicit_paths: impl IntoIterator<Item = String>,
) -> Vec<&'a str> {
    let mut selected = explicit_paths.into_iter().collect::<BTreeSet<_>>();
    selected.insert(format!("{prefix}shine.toml"));
    for permission in permissions.iter() {
        if let PermissionV1::Filesystem {
            access: FilesystemAccessV1::Execute,
            path,
        } = permission
            && let Some(relative) = path.strip_prefix("preset:")
        {
            selected.insert(format!("{prefix}{relative}"));
        }
    }
    runtime
        .presets()
        .files()
        .keys()
        .filter(|path| path.starts_with(prefix))
        .filter(|path| selected.contains(path.as_str()) || is_code_support_file(path))
        .map(String::as_str)
        .collect()
}

fn is_code_support_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    if matches!(name, "package.json" | "bun.lock" | "bun.lockb") {
        return true;
    }
    matches!(
        name.rsplit_once('.').map(|(_, extension)| extension),
        Some(
            "sh" | "bash"
                | "zsh"
                | "fish"
                | "ps1"
                | "cmd"
                | "bat"
                | "ts"
                | "js"
                | "mts"
                | "mjs"
                | "cjs"
        )
    )
}
