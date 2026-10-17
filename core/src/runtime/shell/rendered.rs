//! Rendered resource preparation and shared-destination consistency.
use super::*;

impl<H: FileSystemHost> CoreRuntime<H> {
    pub(super) async fn prepare_shell_rendered_replacements(
        &self,
        categories: &[ShellCategory],
        manifest_before: &ShellManifest,
        overwrite_embedded: bool,
    ) -> Result<(Vec<ShellRenderedFileReplacement>, ShellTemplateReport)> {
        let mut replacements = BTreeMap::<PathBuf, ShellRenderedFileReplacement>::new();
        let mut report = ShellTemplateReport::default();
        for category in categories {
            for file in &category.files {
                let source = self.shell_deployment_source_path(&category.name, &file.source_rel);
                let (content, source_mode, transforms) = if self.context().is_external_presets {
                    let logical = format!(
                        "shell/{}/{}",
                        category.name,
                        shell_logical_path(&file.source_rel)
                    );
                    let desired = self
                        .presets()
                        .get(&logical)
                        .context("missing desired Shell rendered-file source")?;
                    let transforms = if !file.transforms.is_empty() {
                        file.transforms.clone()
                    } else if has_template_annotation(desired) {
                        vec!["template".to_string()]
                    } else {
                        continue;
                    };
                    let content =
                        self.host().read(&source).await.map_err(|error| {
                            error.into_anyhow("reading Shell rendered-file source")
                        })?;
                    let mode = self
                        .host()
                        .metadata(&source)
                        .await
                        .ok()
                        .and_then(|metadata| metadata.unix_mode);
                    (content, mode, transforms)
                } else {
                    let (content, mode) = self
                        .planned_embedded_shell_source(category, file, overwrite_embedded)
                        .await?;
                    let transforms = if !file.transforms.is_empty() {
                        file.transforms.clone()
                    } else if has_template_annotation(&content) {
                        vec!["template".to_string()]
                    } else {
                        continue;
                    };
                    (content, mode, transforms)
                };
                let rendered =
                    crate::install::apply_transforms(&transforms, &content, &self.context().env)
                        .with_context(|| {
                            format!("template substitution failed for {}", source.display())
                        })?;
                let destination = self.shell_rendered_path(&category.name, &file.source_rel);
                let unix_mode = source_mode.or_else(|| cfg!(unix).then_some(0o755));
                let current = match self.host().metadata(&destination).await {
                    Ok(metadata) if metadata.kind == FileKind::File => {
                        let bytes = self.host().read(&destination).await.map_err(|error| {
                            error.into_anyhow("reading current Shell rendered file")
                        })?;
                        bytes == rendered && metadata.unix_mode == unix_mode
                    }
                    Ok(_) => false,
                    Err(error) if error.is_not_found() => false,
                    Err(error) => {
                        return Err(error.into_anyhow("inspecting Shell rendered file"));
                    }
                };
                if current {
                    continue;
                }
                let target = format!("shell/{}/{}", category.name, file.command_name);
                let desired_receipt = if self.context().is_external_presets {
                    self.shell_manifest_entry(category, file).await?
                } else {
                    self.planned_embedded_shell_manifest_entry(category, file, overwrite_embedded)
                        .await?
                };
                let transition = ShellReceiptTransition {
                    target: target.clone(),
                    previous: manifest_before.find(&target).cloned(),
                    desired: desired_receipt,
                };
                if let Some(existing) = replacements.get_mut(&destination) {
                    if existing.bytes != rendered || existing.unix_mode != unix_mode {
                        bail!(
                            "Shell commands sharing rendered path {} produce different output",
                            destination.display()
                        );
                    }
                    existing.receipt_transitions.push(transition);
                } else {
                    replacements.insert(
                        destination.clone(),
                        ShellRenderedFileReplacement {
                            target,
                            destination,
                            bytes: rendered,
                            unix_mode,
                            receipt_transitions: vec![transition],
                        },
                    );
                }
                report
                    .updated
                    .push(format!("{}/{}", category.name, file.command_name));
            }
        }
        report.updated.sort();
        report.updated.dedup();
        Ok((replacements.into_values().collect(), report))
    }
}
