use anyhow::{Context, Result, bail};
use std::path::PathBuf;

use crate::config::Config;
use crate::env::EnvConfig;

/// `config.toml` `[env]`, and write the rendered result to `rendered_path`
/// (rendered_dir — always shine-managed).  File permissions are copied from source.
pub(super) struct ScriptTemplate {
    pub(super) source_path: PathBuf,
    pub(super) rendered_path: PathBuf,
    pub(super) display_name: String,
    /// Metadata-declared transforms (e.g. `["template"]`). When empty, a native
    /// `.sh`/`.ps1` script may still opt in via the `# shine-template: true`
    /// annotation; scripts with neither are left unrendered.
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
    let env = EnvConfig::load_or_init(config).await?;
    let env_map = env.as_map();
    let mut report = TemplateRenderReport::default();

    for script in script_pairs {
        let content = match tokio::fs::read(&script.source_path).await {
            Ok(b) => b,
            Err(_) => continue,
        };

        // Metadata-declared transforms take precedence; otherwise fall back to the
        // legacy `# shine-template: true` annotation (native scripts only). Scripts
        // with neither are left unrendered.
        let effective_transforms: Vec<String> = if !script.transforms.is_empty() {
            script.transforms.clone()
        } else if crate::presets::parse_template_annotation(&content) {
            vec!["template".to_string()]
        } else {
            continue;
        };

        let script_env_map = env_map_for_shell_template(&script.display_name, env_map);
        let rendered = match crate::install_core::apply_transforms(
            &effective_transforms,
            &content,
            &script_env_map,
        ) {
            Ok(b) => b,
            Err(e) => bail!(
                "template substitution failed for {}: {e:#}",
                script.source_path.display()
            ),
        };

        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::metadata(&script.source_path)
                .await
                .map(|m| m.permissions().mode())
                .unwrap_or(0o755)
        };

        let was_changed = tokio::fs::read(&script.rendered_path)
            .await
            .map(|current| current != rendered)
            .unwrap_or(true);

        if let Some(parent) = script.rendered_path.parent() {
            tokio::fs::create_dir_all(parent).await.with_context(|| {
                format!("creating rendered script directory: {}", parent.display())
            })?;
        }

        tokio::fs::write(&script.rendered_path, &rendered)
            .await
            .with_context(|| {
                format!(
                    "writing rendered script: {}",
                    script.rendered_path.display()
                )
            })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(mode);
            tokio::fs::set_permissions(&script.rendered_path, perms)
                .await
                .with_context(|| {
                    format!(
                        "setting rendered script permissions: {}",
                        script.rendered_path.display()
                    )
                })?;
        }

        if was_changed {
            report.updated.push(script.display_name.clone());
        }
    }

    Ok(report)
}

/// Supplies empty optional credentials for templates that defer their validation until runtime.
///
/// Status checks must use this same map as installation; otherwise a missing optional
/// credential makes the comparison render fail and hides a real preset update.
pub(crate) fn env_map_for_shell_template<'a>(
    display_name: &str,
    env_map: &'a std::collections::BTreeMap<String, String>,
) -> std::borrow::Cow<'a, std::collections::BTreeMap<String, String>> {
    if display_name == "agent/ccenv" {
        let mut map = env_map.clone();
        map.entry("DEEPSEEK_API_KEY".to_string()).or_default();
        map.entry("QWEN_API_KEY".to_string()).or_default();
        map.entry("CLIPROXYAPI_AUTH_TOKEN".to_string()).or_default();
        std::borrow::Cow::Owned(map)
    } else {
        std::borrow::Cow::Borrowed(env_map)
    }
}
