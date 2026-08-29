use anyhow::Result;
use std::path::PathBuf;

use crate::config::Config;
use crate::core_runtime;
use crate::env::EnvConfig;

pub(super) struct ScriptTemplate {
    pub(super) source_path: PathBuf,
    pub(super) rendered_path: PathBuf,
    pub(super) display_name: String,
    pub(super) transforms: Vec<String>,
}

#[derive(Default)]
pub(super) struct TemplateRenderReport {
    pub(super) updated: Vec<String>,
}

pub(super) async fn apply_template_to_scripts(
    config: &Config,
    script_pairs: &[ScriptTemplate],
) -> Result<TemplateRenderReport> {
    let mut runtime = core_runtime::from_config(config).await?;
    runtime.context_mut_for_cli().env = EnvConfig::load_or_init(config).await?.as_map().clone();
    let scripts = script_pairs
        .iter()
        .map(|script| shine_core::runtime::ShellScriptTemplate {
            source_path: script.source_path.clone(),
            rendered_path: script.rendered_path.clone(),
            display_name: script.display_name.clone(),
            transforms: script.transforms.clone(),
        })
        .collect::<Vec<_>>();
    let report = runtime.render_shell_templates(&scripts).await?;
    Ok(TemplateRenderReport {
        updated: report.updated,
    })
}
