//! Embedded source selection and ordered transactional cache preparation.
use super::*;

impl<H: FileSystemHost> CoreRuntime<H> {
    pub(super) async fn planned_embedded_shell_source(
        &self,
        category: &ShellCategory,
        file: &ShellFile,
        overwrite: bool,
    ) -> Result<(Vec<u8>, Option<u32>)> {
        let logical = format!(
            "shell/{}/{}",
            category.name,
            shell_logical_path(&file.source_rel)
        );
        let desired = self
            .presets()
            .get(&logical)
            .context("missing desired embedded Shell source")?;
        let source_path = self.shell_deployment_source_path(&category.name, &file.source_rel);
        match self.host().metadata(&source_path).await {
            Ok(metadata) if metadata.kind == FileKind::File => {
                let current =
                    self.host().read(&source_path).await.map_err(|error| {
                        error.into_anyhow("reading embedded Shell cache source")
                    })?;
                if overwrite && current != *desired {
                    Ok((desired.to_vec(), embedded_shell_cache_mode(&logical)))
                } else {
                    Ok((current, metadata.unix_mode))
                }
            }
            Ok(_) => bail!(
                "embedded Shell cache source is not a regular file: {}",
                source_path.display()
            ),
            Err(error) if error.is_not_found() => {
                Ok((desired.to_vec(), embedded_shell_cache_mode(&logical)))
            }
            Err(error) => Err(error.into_anyhow("inspecting embedded Shell cache source")),
        }
    }

    pub(super) async fn planned_embedded_shell_manifest_entry(
        &self,
        category: &ShellCategory,
        file: &ShellFile,
        overwrite: bool,
    ) -> Result<ShellManifestEntry> {
        let source_path = self.shell_deployment_source_path(&category.name, &file.source_rel);
        let (bytes, _) = self
            .planned_embedded_shell_source(category, file, overwrite)
            .await?;
        let transforms = if !file.transforms.is_empty() {
            file.transforms.clone()
        } else if has_template_annotation(&bytes) {
            vec!["template".to_string()]
        } else {
            Vec::new()
        };
        self.shell_manifest_entry_for_content(category, file, source_path, transforms, &bytes)
    }

    pub(super) async fn prepare_shell_cache_replacements(
        &self,
        categories: &[ShellCategory],
        manifest_before: &ShellManifest,
        overwrite: bool,
    ) -> Result<(Vec<ShellCacheReplacement>, ShellCacheReport)> {
        let mut replacements = Vec::new();
        let mut report = ShellCacheReport::default();
        for category in categories {
            let prefix = format!("shell/{}/", category.name);
            let effective_logicals = self.effective_shell_cache_logicals(category)?;
            let mut files = Vec::new();
            for (logical, bytes) in self.presets().files().iter().filter(|(logical, _)| {
                logical.starts_with(&prefix) && effective_logicals.contains(*logical)
            }) {
                let destination = self.context().presets_dir.join(logical);
                let previous = match self.host().metadata(&destination).await {
                    Ok(metadata) if metadata.kind == FileKind::File => {
                        let current = self.host().read(&destination).await.map_err(|error| {
                            error.into_anyhow("reading embedded Shell cache file")
                        })?;
                        if current == *bytes || !overwrite {
                            report.skipped.push(destination);
                            continue;
                        }
                        report.overwritten.push(destination.clone());
                        Some(ShellFileIdentityV1 {
                            content_hash: crate::install::hash_content(&current),
                            unix_mode: metadata.unix_mode,
                        })
                    }
                    Ok(_) => bail!(
                        "embedded Shell cache destination is not a regular file: {}",
                        destination.display()
                    ),
                    Err(error) if error.is_not_found() => {
                        report.created.push(destination.clone());
                        None
                    }
                    Err(error) => {
                        return Err(error.into_anyhow("inspecting embedded Shell cache file"));
                    }
                };
                let rollback = managed_file_rollback_path(&destination);
                match self.host().metadata(&rollback).await {
                    Err(error) if error.is_not_found() => {}
                    Ok(_) => bail!(
                        "embedded Shell cache rollback path is occupied: {}",
                        rollback.display()
                    ),
                    Err(error) => {
                        return Err(error.into_anyhow("inspecting embedded Shell cache rollback"));
                    }
                }
                let unix_mode = embedded_shell_cache_mode(logical);
                let desired = ShellFileIdentityV1 {
                    content_hash: crate::install::hash_content(bytes),
                    unix_mode,
                };
                if previous.as_ref() == Some(&desired) {
                    report.skipped.push(destination);
                    continue;
                }
                files.push(ShellCacheReplacementFile {
                    destination,
                    bytes: bytes.clone(),
                    unix_mode,
                });
            }
            if files.is_empty() {
                continue;
            }
            let mut receipt_transitions = Vec::new();
            for file in &category.files {
                let target = format!("shell/{}/{}", category.name, file.command_name);
                receipt_transitions.push(ShellReceiptTransition {
                    target: target.clone(),
                    previous: manifest_before.find(&target).cloned(),
                    desired: self
                        .planned_embedded_shell_manifest_entry(category, file, overwrite)
                        .await?,
                });
            }
            replacements.push(ShellCacheReplacement {
                target: format!("shell/{}", category.name),
                files,
                receipt_transitions,
            });
        }
        Ok((replacements, report))
    }
}
