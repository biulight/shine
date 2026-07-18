use anyhow::{Context, Result};
use console::{Style, style};
use dialoguer::theme::ColorfulTheme;

use crate::colors;
use crate::sys::execution::{format_command_preview, status_symbol, status_text, sys_init_command};
use crate::sys::run_manifest::SysRunEntry;
use crate::sys::{LoadedSysPreset, ResolvedSelection, SysDriverKind, SysItem, SysItemMode};

pub(super) fn sys_init_theme() -> ColorfulTheme {
    ColorfulTheme {
        prompt_prefix: style(">".to_string()).for_stderr().cyan().bold(),
        prompt_suffix: style("".to_string()).for_stderr(),
        success_prefix: style("✓".to_string()).for_stderr().green(),
        success_suffix: style("".to_string()).for_stderr(),
        active_item_prefix: style("›".to_string()).for_stderr().cyan().bold(),
        inactive_item_prefix: style(" ".to_string()).for_stderr(),
        checked_item_prefix: style("[x]".to_string()).for_stderr().green(),
        unchecked_item_prefix: style("[ ]".to_string()).for_stderr().black().bright(),
        prompt_style: Style::new().for_stderr().bold(),
        active_item_style: Style::new().for_stderr().cyan(),
        inactive_item_style: Style::new().for_stderr(),
        values_style: Style::new().for_stderr().cyan(),
        hint_style: Style::new().for_stderr().black().bright(),
        ..ColorfulTheme::default()
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
        colors::bold(&item.label),
        status
    );
    if !item.description.is_empty() {
        println!("      {}", colors::dim(&item.description));
    }
    if item.mode == SysItemMode::Managed {
        println!(
            "      {}",
            colors::dim(&format!("Run: shine sys apply {}", item.id))
        );
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
) -> Result<()> {
    println!("{}", colors::dim("[dry-run] System init preview"));
    println!("  OS: {os_id}");
    println!("  Shell: {sys_shell}");
    println!("  Selection: {}", selection.source.describe());
    if !proxy_env.is_empty() {
        println!("  Proxy env:");
        for (key, value) in proxy_env {
            println!("    {}", colors::dim(&format!("{key}={value}")));
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
    println!("  Script: {}", loaded.script_path.display());
    let command = sys_init_command(os_id);
    println!("  Commands:");
    for item_id in &selection.item_ids {
        println!(
            "    {}",
            format_command_preview(&command, &loaded.script_path, std::slice::from_ref(item_id))
        );
    }
    if !selection.item_ids.is_empty() {
        println!("    shine internal sys profile pre/post update");
    }
    println!();
    let content = tokio::fs::read_to_string(&loaded.script_path)
        .await
        .with_context(|| format!("reading {}", loaded.script_path.display()))?;
    println!("{}", colors::dim("--- script content ---"));
    print!("{content}");
    Ok(())
}
