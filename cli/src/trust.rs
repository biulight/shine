use crate::{config::Config, core_runtime};
use anyhow::{Context, Result, bail};
use shine_core::persist::atomic_write_private;
use shine_core::trust::{
    TRUST_STORE_SCHEMA_VERSION, TrustDecisionV1, TrustGrantV1, TrustRequirementV1, TrustStoreV1,
    evaluate_trust,
};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

const TRUST_STORE_FILE: &str = "trust.toml";

pub(crate) async fn load_store(config: &Config) -> Result<TrustStoreV1> {
    load_store_path(&trust_store_path(config)).await
}

pub async fn handle_list(config: &Config) -> Result<()> {
    let store = load_store(config).await?;
    if store.grants.is_empty() {
        println!("No external-code trust grants.");
        return Ok(());
    }
    for grant in store.grants {
        println!(
            "{}\t{}\t{}",
            grant.target,
            grant.capability.as_str(),
            short_digest(&grant.code_digest.as_hex())
        );
    }
    Ok(())
}

pub async fn handle_inspect(config: &Config, target: &str) -> Result<()> {
    let runtime = core_runtime::from_config(config).await?;
    let report = runtime.external_code_requirements(target).await?;
    if report.requirements.is_empty() {
        println!("{target} has no external executable-code requirements.");
        return Ok(());
    }
    let decisions = report
        .requirements
        .iter()
        .map(|requirement| evaluate_trust(&runtime.context().trust_grants, requirement))
        .collect::<Vec<_>>();
    println!(
        "{}",
        render_requirements(target, &report.requirements, &decisions)
    );
    println!();
    println!(
        "{}",
        render_inspect_guidance(target, &report.requirements, &decisions)
    );
    Ok(())
}

fn render_inspect_guidance(
    target: &str,
    requirements: &[TrustRequirementV1],
    decisions: &[TrustDecisionV1],
) -> String {
    if requirements
        .iter()
        .any(|requirement| !requirement.permissions_declared)
    {
        let guidance = if target == "preset" {
            "Add a valid permission declaration to every listed target before granting trust."
                .to_string()
        } else {
            format!(
                "Add a valid permission declaration to the {target} Preset before granting trust."
            )
        };
        return format!("{}\n  {guidance}", crate::colors::yellow("Next"));
    }
    if decisions
        .iter()
        .any(|decision| *decision != TrustDecisionV1::Trusted)
    {
        return format!(
            "{}\n  Review the current code and permissions, then run:\n    shine trust grant {target}",
            crate::colors::cyan("Next")
        );
    }
    format!(
        "{} {}",
        crate::colors::symbol("✓"),
        crate::colors::green(&format!(
            "All current external code for {target} is trusted."
        ))
    )
}

pub async fn handle_grant(config: &Config, target: &str, yes: bool) -> Result<()> {
    let runtime = core_runtime::from_config(config).await?;
    let report = runtime.external_code_requirements(target).await?;
    if report.requirements.is_empty() {
        bail!("{target} has no external executable code to trust");
    }
    validate_grant_requirements(target, &report.requirements)?;
    let decisions = report
        .requirements
        .iter()
        .map(|requirement| evaluate_trust(&runtime.context().trust_grants, requirement))
        .collect::<Vec<_>>();
    println!(
        "{}",
        render_requirements(target, &report.requirements, &decisions)
    );
    if !yes {
        if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
            bail!("trust enrollment requires an interactive terminal or explicit --yes");
        }
        let prompt = if target == "preset" {
            "Trust every listed target's current external code?"
        } else {
            "Trust this target's current external code?"
        };
        if !dialoguer::Confirm::new()
            .with_prompt(prompt)
            .default(false)
            .interact()?
        {
            bail!("external-code trust was not granted");
        }
    }
    let mut store = load_store(config).await?;
    for requirement in report.requirements {
        store.grants.retain(|grant| {
            grant.target != requirement.target || grant.capability != requirement.capability
        });
        store
            .grants
            .push(TrustGrantV1::for_reviewed_requirement(&requirement));
    }
    store.grants.sort_by(|left, right| {
        (&left.target, left.capability.as_str()).cmp(&(&right.target, right.capability.as_str()))
    });
    save_store(config, &store).await?;
    println!("Trusted current external code for {target}.");
    Ok(())
}

fn validate_grant_requirements(target: &str, requirements: &[TrustRequirementV1]) -> Result<()> {
    if requirements
        .iter()
        .any(|requirement| !requirement.permissions_declared)
    {
        bail!(
            "{target} external code has no valid permission declaration; fix and validate the Preset before granting trust"
        );
    }
    Ok(())
}

pub async fn handle_revoke(config: &Config, target: &str) -> Result<()> {
    validate_target(target)?;
    let mut store = load_store(config).await?;
    let before = store.grants.len();
    if target == "preset" {
        store.grants.clear();
    } else {
        store.grants.retain(|grant| grant.target != target);
    }
    if store.grants.len() == before {
        println!("No external-code trust grants matched {target}.");
        return Ok(());
    }
    save_store(config, &store).await?;
    println!("Revoked external-code trust for {target}.");
    Ok(())
}

async fn load_store_path(path: &Path) -> Result<TrustStoreV1> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                bail!("trust store must be a regular file: {}", path.display());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o077 != 0 {
                    bail!(
                        "trust store permissions are too broad; expected 0600: {}",
                        path.display()
                    );
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(TrustStoreV1::default());
        }
        Err(error) => return Err(error).with_context(|| format!("inspecting {}", path.display())),
    }
    let contents = tokio::fs::read_to_string(path).await?;
    let store: TrustStoreV1 = toml::from_str(&contents)
        .with_context(|| format!("parsing trust store {}", path.display()))?;
    if store.schema_version != TRUST_STORE_SCHEMA_VERSION {
        bail!(
            "unsupported trust store schema version {}",
            store.schema_version
        );
    }
    Ok(store)
}

async fn save_store(config: &Config, store: &TrustStoreV1) -> Result<()> {
    let encoded = toml::to_string_pretty(store).context("serializing trust store")?;
    atomic_write_private(&trust_store_path(config), encoded.as_bytes()).await
}

fn trust_store_path(config: &Config) -> PathBuf {
    config.shine_dir().join(TRUST_STORE_FILE)
}

fn validate_target(target: &str) -> Result<()> {
    if target == "preset" {
        return Ok(());
    }
    let parts = target.split('/').collect::<Vec<_>>();
    let valid = match parts.as_slice() {
        ["app" | "sys", name] => valid_target_segment(name),
        ["shell", category, command] => {
            valid_target_segment(category) && valid_target_segment(command)
        }
        _ => false,
    };
    if !valid {
        bail!(
            "trust target must be preset, app/<category>, shell/<category>/<command>, or sys/<item>: {target}"
        );
    }
    Ok(())
}

fn valid_target_segment(value: &str) -> bool {
    !value.is_empty() && !value.contains('\\') && !matches!(value, "." | "..")
}

fn render_requirements(
    target: &str,
    requirements: &[TrustRequirementV1],
    decisions: &[TrustDecisionV1],
) -> String {
    debug_assert_eq!(requirements.len(), decisions.len());
    let mut scopes = Vec::<Vec<usize>>::new();
    for (index, requirement) in requirements.iter().enumerate() {
        if let Some(scope) = scopes.iter_mut().find(|scope| {
            let existing = &requirements[scope[0]];
            existing.target == requirement.target
                && existing.code_digest == requirement.code_digest
                && existing.permissions_declared == requirement.permissions_declared
                && existing.permissions == requirement.permissions
        }) {
            scope.push(index);
        } else {
            scopes.push(vec![index]);
        }
    }

    let mut lines = vec![crate::colors::bold(&format!(
        "External Code Trust · {target}"
    ))];
    for (scope_index, scope) in scopes.iter().enumerate() {
        let requirement = &requirements[scope[0]];
        lines.push(String::new());
        if scopes.len() > 1 {
            lines.push(format!(
                "  {}",
                crate::colors::bold(&format!("Scope {}", scope_index + 1))
            ));
        }
        if requirement.target != target {
            lines.push(format!(
                "  {}  {}",
                crate::colors::dim("Target"),
                requirement.target
            ));
        }
        lines.push(format!(
            "  {}  {}",
            crate::colors::dim("Code digest"),
            requirement.code_digest.as_hex()
        ));
        lines.push(String::new());
        lines.push(format!("  {}", crate::colors::bold("Capabilities")));
        let width = scope
            .iter()
            .map(|index| requirements[*index].capability.as_str().len())
            .max()
            .unwrap_or_default();
        for index in scope {
            let requirement = &requirements[*index];
            let decision = decisions[*index];
            let (symbol, status) = trust_status(decision);
            lines.push(format!(
                "    {} {:width$}  {} {}",
                crate::colors::symbol(symbol),
                requirement.capability.as_str(),
                styled_trust_status(decision, status),
                crate::colors::dim(&format!("[{}]", decision.code())),
            ));
        }
        lines.push(String::new());
        lines.push(format!("  {}", crate::colors::bold("Permissions")));
        if !requirement.permissions_declared {
            lines.push(format!(
                "    {} {}",
                crate::colors::symbol("!"),
                crate::colors::yellow("valid declaration missing")
            ));
        } else if requirement.permissions.is_empty() {
            lines.push(format!("    {}", crate::colors::dim("- none")));
        } else {
            let mut grouped = std::collections::BTreeMap::<String, Vec<String>>::new();
            for permission in requirement.permissions.iter() {
                let (group, value) = crate::lifecycle_plan::permission_group(permission);
                grouped.entry(group).or_default().push(value);
            }
            for (group, values) in grouped {
                lines.push(format!("    {group}"));
                for value in values {
                    lines.push(format!("      - {value}"));
                }
            }
        }
    }
    lines.join("\n")
}

fn trust_status(decision: TrustDecisionV1) -> (&'static str, &'static str) {
    match decision {
        TrustDecisionV1::Trusted => ("✓", "trusted"),
        TrustDecisionV1::Missing => ("✗", "not trusted"),
        TrustDecisionV1::CodeChanged => ("!", "code changed"),
        TrustDecisionV1::PermissionsChanged => ("!", "permissions changed"),
        TrustDecisionV1::UnsupportedGrantSchema => ("!", "unsupported grant schema"),
    }
}

fn styled_trust_status(decision: TrustDecisionV1, status: &str) -> String {
    match decision {
        TrustDecisionV1::Trusted => crate::colors::green(status),
        TrustDecisionV1::Missing | TrustDecisionV1::UnsupportedGrantSchema => {
            crate::colors::red(status)
        }
        TrustDecisionV1::CodeChanged | TrustDecisionV1::PermissionsChanged => {
            crate::colors::yellow(status)
        }
    }
}

fn short_digest(digest: &str) -> &str {
    digest.get(..12).unwrap_or(digest)
}

#[cfg(test)]
pub(crate) async fn grant_current_for_test(config: &Config, target: &str) {
    let runtime = core_runtime::from_config(config).await.unwrap();
    let report = runtime.external_code_requirements(target).await.unwrap();
    let mut store = load_store(config).await.unwrap();
    for requirement in report.requirements {
        store.grants.retain(|grant| {
            grant.target != requirement.target || grant.capability != requirement.capability
        });
        store
            .grants
            .push(TrustGrantV1::for_reviewed_requirement(&requirement));
    }
    save_store(config, &store).await.unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use shine_core::plan::{PermissionSetV1, PermissionV1, SnapshotDigestV1};
    use shine_core::trust::TrustCapabilityV1;

    fn requirement(permissions_declared: bool) -> TrustRequirementV1 {
        TrustRequirementV1 {
            target: "sys/package-only".to_string(),
            capability: TrustCapabilityV1::SysProfileCode,
            code_digest: SnapshotDigestV1::builder("code").finish(),
            permissions_declared,
            permissions: PermissionSetV1::default(),
        }
    }

    #[test]
    fn trust_targets_must_be_canonical_and_target_local() {
        assert!(validate_target("preset").is_ok());
        assert!(validate_target("app/demo").is_ok());
        assert!(validate_target("shell/demo/tool").is_ok());
        assert!(validate_target("sys/mise").is_ok());
        assert!(validate_target("demo").is_err());
        assert!(validate_target("app/demo/other").is_err());
        assert!(validate_target("shell/demo").is_err());
    }

    #[test]
    fn explicit_empty_permission_declaration_is_grantable() {
        assert!(validate_grant_requirements("sys/package-only", &[requirement(true)]).is_ok());
    }

    #[test]
    fn missing_permission_declaration_remains_ungrantable() {
        let error =
            validate_grant_requirements("sys/package-only", &[requirement(false)]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("no valid permission declaration")
        );
    }

    #[test]
    fn inspect_guidance_points_untrusted_targets_to_grant() {
        assert_eq!(
            render_inspect_guidance(
                "sys/package-only",
                &[requirement(true)],
                &[TrustDecisionV1::Missing]
            ),
            "Next\n  Review the current code and permissions, then run:\n    shine trust grant sys/package-only"
        );
    }

    #[test]
    fn inspect_guidance_does_not_suggest_an_ungrantable_action() {
        assert_eq!(
            render_inspect_guidance(
                "sys/package-only",
                &[requirement(false)],
                &[TrustDecisionV1::Missing]
            ),
            "Next\n  Add a valid permission declaration to the sys/package-only Preset before granting trust."
        );
    }

    #[test]
    fn inspect_guidance_confirms_when_every_requirement_is_trusted() {
        assert_eq!(
            render_inspect_guidance(
                "sys/package-only",
                &[requirement(true)],
                &[TrustDecisionV1::Trusted]
            ),
            "✓ All current external code for sys/package-only is trusted."
        );
    }

    #[test]
    fn requirement_renderer_groups_shared_scope_and_uses_human_readable_permissions() {
        let permissions = PermissionSetV1::new([
            PermissionV1::Command {
                program: "bun".to_string(),
            },
            PermissionV1::Network {
                scope: shine_core::plan::NetworkScopeV1::Any,
            },
        ]);
        let requirements = [
            TrustRequirementV1 {
                target: "app/surge".to_string(),
                capability: TrustCapabilityV1::AppHook,
                code_digest: SnapshotDigestV1::builder("surge-code").finish(),
                permissions_declared: true,
                permissions: permissions.clone(),
            },
            TrustRequirementV1 {
                target: "app/surge".to_string(),
                capability: TrustCapabilityV1::AppGenerator,
                code_digest: SnapshotDigestV1::builder("surge-code").finish(),
                permissions_declared: true,
                permissions,
            },
        ];

        let output = render_requirements(
            "app/surge",
            &requirements,
            &[TrustDecisionV1::Missing, TrustDecisionV1::Trusted],
        );

        assert!(output.starts_with("External Code Trust · app/surge"));
        assert_eq!(output.matches("Code digest").count(), 1);
        assert_eq!(output.matches("Permissions").count(), 1);
        assert!(output.contains("✗ app-hook       not trusted [external_code_trust_missing]"));
        assert!(output.contains("✓ app-generator  trusted [trusted]"));
        assert!(output.contains("    command\n      - bun"));
        assert!(output.contains("    network\n      - any"));
        assert!(!output.contains("PermissionV1"));
        assert!(!output.contains("Command {"));
    }

    #[test]
    fn requirement_renderer_keeps_different_security_scopes_separate() {
        let mut changed = requirement(true);
        changed.code_digest = SnapshotDigestV1::builder("changed-code").finish();
        let requirements = [requirement(true), changed];

        let output = render_requirements(
            "sys/package-only",
            &requirements,
            &[TrustDecisionV1::Trusted, TrustDecisionV1::CodeChanged],
        );

        assert!(output.contains("Scope 1"));
        assert!(output.contains("Scope 2"));
        assert_eq!(output.matches("Code digest").count(), 2);
        assert_eq!(output.matches("Permissions").count(), 2);
    }

    #[test]
    fn preset_requirement_renderer_keeps_targets_separate_and_visible() {
        let app = TrustRequirementV1 {
            target: "app/demo".to_string(),
            capability: TrustCapabilityV1::AppHook,
            code_digest: SnapshotDigestV1::builder("shared-code").finish(),
            permissions_declared: true,
            permissions: PermissionSetV1::default(),
        };
        let shell = TrustRequirementV1 {
            target: "shell/demo/tool".to_string(),
            capability: TrustCapabilityV1::ShellCommand,
            ..app.clone()
        };

        let output = render_requirements(
            "preset",
            &[app, shell],
            &[TrustDecisionV1::Missing, TrustDecisionV1::Missing],
        );

        assert_eq!(output.matches("Code digest").count(), 2);
        assert!(output.contains("Target  app/demo"));
        assert!(output.contains("Target  shell/demo/tool"));
        assert!(output.contains("app-hook"));
        assert!(output.contains("shell-command"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn trust_store_rejects_broad_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = crate::test_support::make_temp_dir("shine-trust-store").await;
        let path = dir.join(TRUST_STORE_FILE);
        tokio::fs::write(
            &path,
            toml::to_string_pretty(&TrustStoreV1::default()).unwrap(),
        )
        .await
        .unwrap();
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .await
            .unwrap();

        assert!(load_store_path(&path).await.is_err());
        tokio::fs::remove_dir_all(dir).await.unwrap();
    }
}
