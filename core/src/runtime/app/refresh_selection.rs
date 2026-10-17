//! Shared refresh selection over captured metadata and the caller's fresh manifest.
//! No IO, approval, or cached observations cross the planning/execution boundary.
use super::*;

pub(in crate::runtime) struct SelectedRefreshFile {
    pub(in crate::runtime) file: AppFile,
    pub(in crate::runtime) destination: PathBuf,
    pub(in crate::runtime) entry: AppEntry,
}

pub(in crate::runtime) fn select_refresh_files(
    category: &AppCategory,
    selector: Option<&Path>,
) -> Result<Vec<AppFile>> {
    let candidates = if let Some(selector) = selector {
        let file = category
            .files
            .iter()
            .find(|file| file.source_rel == selector)
            .with_context(|| {
                format!(
                    "app '{}' file not found: {}",
                    category.name,
                    selector.display()
                )
            })?;
        if file.generator.is_none() {
            bail!(
                "app '{}' file is not generated: {}",
                category.name,
                selector.display()
            );
        }
        vec![file.clone()]
    } else {
        category
            .files
            .iter()
            .filter(|file| file.generator.is_some())
            .cloned()
            .collect::<Vec<_>>()
    };
    if candidates.is_empty() {
        bail!("app '{}' has no generated files", category.name);
    }
    Ok(candidates)
}

impl<H: FileSystemObservationHost> CoreRuntime<H> {
    pub(in crate::runtime) fn select_installed_refresh_files(
        &self,
        category: &AppCategory,
        candidates: Vec<AppFile>,
        manifest: &AppManifest,
        explicit_file: bool,
    ) -> Result<Vec<SelectedRefreshFile>> {
        let mut selected = Vec::new();
        for file in candidates {
            let destination = self.app_destination(category, &file)?;
            let Some(entry) = manifest
                .find_by_dest(&destination)
                .filter(|entry| {
                    entry.source == format!("app/{}/{}", category.name, file.source_rel.display())
                })
                .cloned()
            else {
                if explicit_file {
                    bail!(generated_file_not_installed_message(
                        &category.name,
                        &file.source_rel
                    ));
                }
                continue;
            };
            selected.push(SelectedRefreshFile {
                file,
                destination,
                entry,
            });
        }
        if selected.is_empty() {
            bail!(
                "app '{}' has no installed generated files; run `shine install app/{}` first",
                category.name,
                category.name
            );
        }
        Ok(selected)
    }
}
