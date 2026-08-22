use anyhow::{Context, Result, bail};
use console::style;
use dialoguer::MultiSelect;
use std::collections::{BTreeMap, BTreeSet};

use crate::colors;

use super::{
    ResolvedSelection, SelectionSource, SysItem, SysItemMode, SysManifest, sys_init_theme,
};

pub(super) fn resolve_selection(
    manifest: &SysManifest,
    requested: &[String],
    preset: Option<&str>,
    interactive: bool,
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

    if let Some(profile_name) = preset {
        return Ok(ResolvedSelection {
            item_ids: profile_items(manifest, profile_name)?.to_vec(),
            source: SelectionSource::Profile(profile_name.to_string()),
        });
    }

    if manifest.items.is_empty() {
        return Ok(ResolvedSelection {
            item_ids: Vec::new(),
            source: SelectionSource::NoItems,
        });
    }

    if interactive {
        return select_items_interactively(manifest);
    }

    let Some(default_profile) = manifest.default_profile.as_deref() else {
        bail!("sys bootstrap requires `default_profile` for non-interactive runs");
    };

    Ok(ResolvedSelection {
        item_ids: profile_items(manifest, default_profile)?.to_vec(),
        source: SelectionSource::DefaultProfile(default_profile.to_string()),
    })
}

fn profile_items<'a>(manifest: &'a SysManifest, profile_name: &str) -> Result<&'a [String]> {
    let profile = manifest
        .profiles
        .get(profile_name)
        .with_context(|| format!("unknown sys bootstrap profile `{profile_name}`"))?;
    Ok(&profile.items)
}

fn default_flags(manifest: &SysManifest) -> Vec<bool> {
    if let Some(default_profile) = manifest.default_profile.as_deref()
        && let Some(profile) = manifest.profiles.get(default_profile)
    {
        let item_set: BTreeSet<&str> = profile.items.iter().map(String::as_str).collect();
        return manifest
            .items
            .iter()
            .map(|item| item_set.contains(item.id.as_str()))
            .collect();
    }

    manifest.items.iter().map(|item| item.default).collect()
}

fn select_items_interactively(manifest: &SysManifest) -> Result<ResolvedSelection> {
    print_interactive_header(manifest);

    let init_items = manifest
        .items
        .iter()
        .filter(|item| item.mode == SysItemMode::Init)
        .collect::<Vec<_>>();
    let default_by_id = manifest
        .items
        .iter()
        .zip(default_flags(manifest))
        .map(|(item, selected)| (item.id.as_str(), selected))
        .collect::<BTreeMap<_, _>>();
    let labels: Vec<String> = init_items
        .iter()
        .map(|item| format_interactive_item(item))
        .collect();
    let defaults = init_items
        .iter()
        .map(|item| {
            default_by_id
                .get(item.id.as_str())
                .copied()
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    let selection = MultiSelect::with_theme(&sys_init_theme())
        .with_prompt("Select system init items")
        .items(&labels)
        .defaults(&defaults)
        .report(false)
        .interact()?;

    let item_ids = selection
        .into_iter()
        .map(|index| init_items[index].id.clone())
        .collect();

    Ok(ResolvedSelection {
        item_ids,
        source: SelectionSource::Interactive,
    })
}

pub(super) fn format_interactive_item(item: &SysItem) -> String {
    let label = style(item.label.as_str()).for_stderr().bold().to_string();
    if item.description.is_empty() {
        return label;
    }

    let description = style(item.description.as_str())
        .for_stderr()
        .dim()
        .to_string();
    format!("{label}  ·  {description}")
}

fn print_interactive_header(manifest: &SysManifest) {
    if let Some(default_profile) = manifest.default_profile.as_deref() {
        println!(
            "{}",
            colors::dim(&format!("Default profile: {default_profile}"))
        );
    }
    println!("{}", colors::dim("Use Space to toggle, Enter to confirm."));
    println!();
}

pub(super) fn format_item_ids(item_ids: &[String]) -> String {
    if item_ids.is_empty() {
        "(none)".to_string()
    } else {
        item_ids.join(", ")
    }
}
