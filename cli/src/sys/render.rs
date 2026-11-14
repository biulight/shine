use crate::colors;
use crate::output::escape_controls;
use crate::sys::execution::{status_symbol, status_text};
use crate::sys::run_manifest::SysRunEntry;
use crate::sys::{LoadedSysPreset, ResolvedSelection, SysDriverKind, SysItem, SysItemMode};
use anyhow::Result;

pub(crate) const PACMAN_PREPARATION: [&str; 3] = [
    "Arch package installation uses the existing pacman database.",
    "First complete a full system update: pacman -Syu (use sudo if needed).",
    "Native pacman installation does not refresh databases or upgrade the system; do not use pacman -Sy alone.",
];

fn preview_pacman_preparation(
    loaded: &LoadedSysPreset,
    selection: &ResolvedSelection,
) -> &'static [&'static str] {
    if loaded.manifest.items.iter().any(|item| {
        selection.item_ids.contains(&item.id)
            && matches!(
                item.install,
                Some(super::SysInstall::Package {
                    provider: super::SysPackageProvider::Pacman,
                    ..
                })
            )
    }) {
        &PACMAN_PREPARATION
    } else {
        &[]
    }
}

pub(in crate::sys) fn print_available_item(item: &SysItem, entry: Option<&SysRunEntry>) {
    let kind = item_mode_name(item.mode);
    let status = entry
        .map(|entry| {
            let symbol = status_symbol(entry.status);
            format!(
                "{} {}",
                colors::symbol(symbol),
                colors::status_label(status_text(entry.status), symbol)
            )
        })
        .unwrap_or_else(|| colors::dim("not recorded"));
    println!(
        "    {:<18} {:<9} {}  {}",
        item.id,
        format!("[{kind}]"),
        colors::bold(&escape_controls(&item.label)),
        status
    );
    if !item.description.is_empty() {
        println!("      {}", colors::dim(&escape_controls(&item.description)));
    }
}

pub(in crate::sys) fn item_mode_name(mode: SysItemMode) -> &'static str {
    match mode {
        SysItemMode::Init => "init",
        SysItemMode::Managed => "managed",
    }
}

pub(in crate::sys) fn driver_name(driver: SysDriverKind) -> &'static str {
    match driver {
        SysDriverKind::Script => "script",
        SysDriverKind::SplitDns => "split-dns",
        SysDriverKind::ManagedFile => "managed-file",
    }
}

pub(in crate::sys) async fn print_dry_run(
    os_id: &str,
    loaded: &LoadedSysPreset,
    selection: &ResolvedSelection,
    sys_shell: &str,
    proxy_env: &[(&'static str, String)],
    previews: &[String],
) -> Result<()> {
    println!("{}", colors::dim("[dry-run] System init preview"));
    println!("  OS: {os_id}");
    println!("  Shell: {sys_shell}");
    println!(
        "  Selection: {}",
        escape_controls(&selection.source.describe())
    );
    if !proxy_env.is_empty() {
        println!("  Proxy env:");
        for (key, value) in proxy_env {
            println!(
                "    {}",
                colors::dim(&format!("{key}={}", escape_controls(value)))
            );
        }
    }
    println!(
        "  Items: {}",
        if selection.item_ids.is_empty() {
            "(none)".to_string()
        } else {
            selection.item_ids.join(", ")
        }
    );
    for line in preview_pacman_preparation(loaded, selection) {
        println!("  {} {line}", colors::yellow("!"));
    }
    println!("  Commands:");
    for preview in previews {
        println!("    {}", escape_controls(preview));
    }
    let integrations = selection
        .item_ids
        .iter()
        .filter_map(|item_id| {
            loaded
                .manifest
                .items
                .iter()
                .find(|item| item.id == *item_id)
        })
        .flat_map(|item| {
            item.shell
                .iter()
                .map(move |integration| (item, integration))
        })
        .collect::<Vec<_>>();
    if !integrations.is_empty() {
        println!("  Shell integrations:");
        for (item, integration) in integrations {
            let action = if let Some(fragment) = &integration.fragment {
                format!("fragment {fragment}")
            } else if integration.path.is_some() {
                "PATH".to_string()
            } else if !integration.env.is_empty() {
                "environment".to_string()
            } else if !integration.eval_argv.is_empty() {
                "guarded eval".to_string()
            } else if integration.source.is_some() {
                "guarded source".to_string()
            } else {
                "aliases".to_string()
            };
            println!(
                "    sys/{}: {} · {:?} · {}",
                item.id,
                integration.phase.as_str(),
                integration.shells,
                action
            );
        }
    }
    if !selection.item_ids.is_empty() {
        println!("    shine internal sys profile pre/post update");
    }
    println!();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacman_preview_guidance_is_once_and_scoped_to_selected_native_items() {
        let loaded = LoadedSysPreset {
            root: "arch".into(),
            manifest: shine_core::runtime::parse_sys_manifest(
                r#"version = 2
[[items]]
id = 'native'
label = 'Native'
detect = { kind = 'command', command = 'fzf' }
install = { kind = 'package', provider = 'pacman', package = 'fzf' }
[[items]]
id = 'script'
label = 'Script'
detect = { kind = 'command', command = 'tool' }
install = { kind = 'script', path = 'tool.sh' }
"#,
            )
            .unwrap(),
        };
        for items in [
            vec![],
            vec!["script"],
            vec!["native"],
            vec!["native", "script"],
        ] {
            let selection = ResolvedSelection {
                item_ids: items.iter().map(|s| s.to_string()).collect(),
                source: shine_core::runtime::SelectionSource::Items,
            };
            let lines = preview_pacman_preparation(&loaded, &selection);
            assert_eq!(lines.len(), if items.contains(&"native") { 3 } else { 0 });
            if !lines.is_empty() {
                assert!(lines.join(" ").contains("pacman -Syu"));
                assert!(!lines.join(" ").contains("database is outdated"));
                // A selection may include a script that performs its own system maintenance.
                assert!(lines.join(" ").contains(
                    "Native pacman installation does not refresh databases or upgrade the system"
                ));
                assert!(!lines.join(" ").contains("Shine does not refresh"));
            }
        }
    }
}
