//! CLI adapter for Core-owned App artifact and teardown execution.

use super::metadata;
use crate::config::Config;
use anyhow::{Context, Result, bail};
use shine_core::runtime::{AppArtifactAction, AppArtifactRequest, RuntimeEvent, RuntimeObserver};

pub async fn handle_build(config: &Config, app_id: &str) -> Result<()> {
    run_explicit(config, app_id, AppArtifactAction::Apply).await
}

pub async fn handle_unbuild(config: &Config, app_id: &str) -> Result<()> {
    run_explicit(config, app_id, AppArtifactAction::Remove).await
}

async fn run_explicit(config: &Config, app_id: &str, action: AppArtifactAction) -> Result<()> {
    let categories = metadata::load_active_categories(config, Some(app_id)).await?;
    let category = categories
        .iter()
        .find(|category| category.name == app_id)
        .with_context(|| format!("app preset category not found: {app_id}"))?;
    let artifact = category
        .artifact
        .clone()
        .with_context(|| format!("app '{app_id}' does not define an artifact script"))?;
    if action == AppArtifactAction::Remove && artifact.teardown.is_none() {
        bail!("app '{app_id}' does not define an artifact teardown script");
    }
    let mut runtime = crate::core_runtime::from_config(config).await?;
    if let Ok(env) = crate::env::EnvConfig::load_or_init(config).await {
        runtime.context_mut_for_cli().env = env.as_map().clone();
    }
    runtime
        .run_app_artifact(
            AppArtifactRequest {
                category: app_id.to_string(),
                artifact,
                action,
                implicit: false,
                dry_run: false,
            },
            &mut ExplicitObserver,
        )
        .await?;
    Ok(())
}

struct ExplicitObserver;

impl RuntimeObserver for ExplicitObserver {
    fn emit(&mut self, _event: RuntimeEvent) {}
}
