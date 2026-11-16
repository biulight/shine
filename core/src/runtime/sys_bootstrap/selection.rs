//! Selection policy only; no detection, installation or authorization.
use super::*;

impl<H> CoreRuntime<H> {
    pub async fn resolve_sys_bootstrap_selection(
        &self,
        os_id: &str,
        requested: &[String],
        preset: Option<&str>,
        interactive: bool,
        interaction: &mut impl RuntimeInteraction,
        observer: &mut impl RuntimeObserver,
    ) -> Result<ResolvedSelection> {
        let loaded = self.load_sys_preset(os_id).await?;
        resolve_selection(
            &loaded.manifest,
            requested,
            preset,
            interactive,
            self.context().shell.into(),
            interaction,
            observer,
        )
    }
}

pub(super) fn resolve_selection(
    manifest: &SysManifest,
    requested: &[String],
    preset: Option<&str>,
    interactive: bool,
    sys_shell: &str,
    interaction: &mut impl RuntimeInteraction,
    observer: &mut impl RuntimeObserver,
) -> Result<ResolvedSelection> {
    if !requested.is_empty() {
        if preset.is_some() {
            bail!("explicit sys bootstrap items cannot be combined with `--preset`");
        }
        let mut seen = BTreeSet::new();
        let mut item_ids = Vec::new();
        for item_id in requested {
            let item = manifest
                .items
                .iter()
                .find(|item| item.id == *item_id)
                .with_context(|| format!("unknown sys bootstrap item `{item_id}`"))?;
            if item.mode == SysItemMode::Managed {
                bail!("`{item_id}` is a managed system resource; use `shine sys apply {item_id}`");
            }
            if seen.insert(item_id.as_str()) {
                item_ids.push(item_id.clone());
            }
        }
        return Ok(ResolvedSelection {
            item_ids,
            source: SelectionSource::Items,
        });
    }
    if let Some(name) = preset {
        let profile = manifest
            .profiles
            .get(name)
            .with_context(|| format!("unknown sys bootstrap profile `{name}`"))?;
        return Ok(ResolvedSelection {
            item_ids: matching_items(manifest, &profile.items, sys_shell),
            source: SelectionSource::Profile(name.to_string()),
        });
    }
    let init_items = manifest
        .items
        .iter()
        .filter(|item| item.mode == SysItemMode::Init)
        .collect::<Vec<_>>();
    if init_items.is_empty() {
        return Ok(ResolvedSelection {
            item_ids: Vec::new(),
            source: SelectionSource::NoItems,
        });
    }
    if interactive {
        let choices = init_items
            .iter()
            .filter(|item| item.matches_shell(sys_shell))
            .map(|item| item.id.clone())
            .collect::<Vec<_>>();
        if choices.is_empty() {
            return Ok(ResolvedSelection {
                item_ids: Vec::new(),
                source: SelectionSource::Interactive,
            });
        }
        observer.emit(RuntimeEvent::Interaction {
            code: "sys_bootstrap_selection",
            target: manifest.default_profile.clone().unwrap_or_default(),
        });
        let defaults = matching_items(manifest, &default_selection(manifest), sys_shell);
        let item_ids = interaction.select_many("Select system init items", &choices, &defaults)?;
        return Ok(ResolvedSelection {
            item_ids,
            source: SelectionSource::Interactive,
        });
    }
    let name = manifest
        .default_profile
        .as_deref()
        .context("sys bootstrap requires `default_profile` for non-interactive runs")?;
    let profile = manifest
        .profiles
        .get(name)
        .with_context(|| format!("unknown sys bootstrap profile `{name}`"))?;
    Ok(ResolvedSelection {
        item_ids: matching_items(manifest, &profile.items, sys_shell),
        source: SelectionSource::DefaultProfile(name.to_string()),
    })
}

// The complete manifest and selection names have already been validated. Keep their order;
// never replace the manifest used for preserved integrations or approval with this projection.
fn matching_items(manifest: &SysManifest, items: &[String], sys_shell: &str) -> Vec<String> {
    items
        .iter()
        .filter(|id| {
            manifest
                .items
                .iter()
                .any(|item| item.id == **id && item.matches_shell(sys_shell))
        })
        .cloned()
        .collect()
}

fn default_selection(manifest: &SysManifest) -> Vec<String> {
    if let Some(name) = manifest.default_profile.as_deref()
        && let Some(profile) = manifest.profiles.get(name)
    {
        return profile.items.clone();
    }
    manifest
        .items
        .iter()
        .filter(|item| item.mode == SysItemMode::Init && item.default)
        .map(|item| item.id.clone())
        .collect()
}

#[cfg(test)]
mod tests;
