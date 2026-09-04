use crate::{config::Config, core_runtime};
use anyhow::{Context, Result, bail};
use shine_core::persist::atomic_write_private;
use shine_core::trust::{
    TRUST_STORE_SCHEMA_VERSION, TrustGrantV1, TrustRequirementV1, TrustStoreV1, evaluate_trust,
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
    for requirement in &report.requirements {
        render_requirement(
            requirement,
            evaluate_trust(&runtime.context().trust_grants, requirement),
        );
    }
    Ok(())
}

pub async fn handle_grant(config: &Config, target: &str, yes: bool) -> Result<()> {
    let runtime = core_runtime::from_config(config).await?;
    let report = runtime.external_code_requirements(target).await?;
    if report.requirements.is_empty() {
        bail!("{target} has no external executable code to trust");
    }
    if report
        .requirements
        .iter()
        .any(|requirement| requirement.permissions.is_empty())
    {
        bail!(
            "{target} external code has no valid permission declaration; fix and validate the Preset before granting trust"
        );
    }
    for requirement in &report.requirements {
        render_requirement(
            requirement,
            evaluate_trust(&runtime.context().trust_grants, requirement),
        );
    }
    if !yes {
        if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
            bail!("trust enrollment requires an interactive terminal or explicit --yes");
        }
        if !dialoguer::Confirm::new()
            .with_prompt("Trust this target's current external code?")
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

pub async fn handle_revoke(config: &Config, target: &str) -> Result<()> {
    validate_target(target)?;
    let mut store = load_store(config).await?;
    let before = store.grants.len();
    store.grants.retain(|grant| grant.target != target);
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
    let valid_prefix = target.starts_with("app/") || target.starts_with("sys/");
    let suffix = target
        .split_once('/')
        .map(|(_, suffix)| suffix)
        .unwrap_or_default();
    if !valid_prefix
        || suffix.is_empty()
        || suffix.contains(['/', '\\'])
        || suffix == "."
        || suffix == ".."
    {
        bail!("trust target must be canonical app/<category> or sys/<item>: {target}");
    }
    Ok(())
}

fn render_requirement(
    requirement: &TrustRequirementV1,
    decision: shine_core::trust::TrustDecisionV1,
) {
    println!("External code trust:");
    println!("  Target:      {}", requirement.target);
    println!("  Capability:  {}", requirement.capability.as_str());
    println!("  Code digest: {}", requirement.code_digest.as_hex());
    println!("  Permissions:");
    if requirement.permissions.is_empty() {
        println!("    none");
    } else {
        for permission in requirement.permissions.iter() {
            println!("    {permission:?}");
        }
    }
    println!("  Status:      {}", decision.code());
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

    #[test]
    fn trust_targets_must_be_canonical_and_target_local() {
        assert!(validate_target("app/demo").is_ok());
        assert!(validate_target("sys/mise").is_ok());
        assert!(validate_target("demo").is_err());
        assert!(validate_target("app/demo/other").is_err());
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
