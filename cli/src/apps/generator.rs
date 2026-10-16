//! CLI adapter for the Core-owned App generator executor.

use super::metadata::{AppCategory, AppFile};
use crate::config::Config;
use anyhow::Result;
use shine_core::runtime::{AppGeneratorRequest, RuntimeEvent, RuntimeObserver};
use std::collections::BTreeMap;

pub(super) async fn generate(
    config: &Config,
    category: &AppCategory,
    file: &AppFile,
    env: &BTreeMap<String, String>,
) -> Result<Option<Vec<u8>>> {
    let Some(generator) = &file.generator else {
        return Ok(None);
    };
    if !env.contains_key(&generator.when_env) {
        return Ok(None);
    }
    let mut invocation = config.clone();
    invocation.env = env.clone();
    let runtime = crate::core_runtime::from_config(&invocation).await?;
    let mut observer = GeneratorObserver {
        category: &category.name,
    };
    runtime
        .run_app_generator(
            AppGeneratorRequest {
                category: category.name.clone(),
                source: file.source_rel.display().to_string(),
                generator: generator.clone(),
                // Callers retain the existing auto/manual routing. Reaching
                // this adapter means execution was explicitly selected.
                explicit: true,
            },
            &mut observer,
        )
        .await
}

struct GeneratorObserver<'a> {
    category: &'a str,
}

impl RuntimeObserver for GeneratorObserver<'_> {
    fn emit(&mut self, event: RuntimeEvent) {
        if let RuntimeEvent::ProcessOutput { text, .. } = event {
            eprintln!(
                "  {} {}: {}",
                crate::colors::symbol("!"),
                self.category,
                text
            );
        }
    }
}
