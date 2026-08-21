use std::collections::BTreeMap;

use crate::colors;

use super::{
    ResolvedSelection, SysItemOutcome, SysItemStatus, SysManifest, selection::format_item_ids,
};

pub(super) fn manifest_item_labels(manifest: &SysManifest) -> BTreeMap<&str, String> {
    manifest
        .items
        .iter()
        .map(|item| (item.id.as_str(), item.label.clone()))
        .collect()
}

pub(super) fn sys_item_label_width(
    selection: &ResolvedSelection,
    labels: &BTreeMap<&str, String>,
) -> usize {
    selection
        .item_ids
        .iter()
        .filter_map(|id| labels.get(id.as_str()))
        .map(String::len)
        .chain(std::iter::once("profile".len()))
        .max()
        .unwrap_or(14)
        .max(14)
}

pub(super) fn print_run_header(os_id: &str, sys_shell: &str, selection: &ResolvedSelection) {
    println!("{}", colors::bold("System Init"));
    println!("  OS: {os_id}");
    println!("  Shell: {sys_shell}");
    println!("  Selection: {}", selection.source.describe());
    println!("  Items: {} selected", selection.item_ids.len());
    println!("  {}", colors::dim(&format_item_ids(&selection.item_ids)));
    println!();
}

pub(super) fn proxy_env_vars(config: &crate::config::Config) -> Vec<(&'static str, String)> {
    let env = crate::env::EnvConfig::from_config(config);
    let url = format!(
        "http://{}:{}",
        env.get("PROXY_HOST").unwrap_or("127.0.0.1"),
        env.get("HTTP_PROXY_PORT").unwrap_or("6152")
    );
    let no_proxy = env
        .get("PROXY_NO_PROXY")
        .unwrap_or("localhost,127.0.0.1,::1");
    vec![
        ("http_proxy", url.clone()),
        ("HTTP_PROXY", url.clone()),
        ("https_proxy", url.clone()),
        ("HTTPS_PROXY", url.clone()),
        ("all_proxy", url.clone()),
        ("ALL_PROXY", url.clone()),
        ("no_proxy", no_proxy.to_string()),
        ("NO_PROXY", no_proxy.to_string()),
        ("SHINE_SYS_PROXY", url),
    ]
}

pub(super) fn print_item_outcome(outcome: &SysItemOutcome, label_width: usize) {
    let symbol = status_symbol(outcome.status);
    let label = format!("{:<label_width$}", outcome.label);
    let status = format!("{:<17}", status_text(outcome.status));
    let detail = if outcome.detail.is_empty() {
        String::new()
    } else {
        colors::dim(&outcome.detail)
    };
    println!(
        "  {} {} {} {}",
        colors::symbol(symbol),
        colors::bold(&label),
        colors::status_label(&status, symbol),
        detail
    );
    for line in &outcome.logs {
        println!("    {}", colors::dim(line));
    }
}

pub(super) fn status_symbol(status: SysItemStatus) -> &'static str {
    match status {
        SysItemStatus::Skipped | SysItemStatus::NeedsAction => "~",
        SysItemStatus::Failed => "✗",
        _ => "✓",
    }
}

pub(super) fn status_text(status: SysItemStatus) -> &'static str {
    match status {
        SysItemStatus::Installed => "installed",
        SysItemStatus::AlreadyInstalled => "already installed",
        SysItemStatus::Skipped => "skipped",
        SysItemStatus::Updated => "updated",
        SysItemStatus::NeedsAction => "needs action",
        SysItemStatus::Completed => "completed",
        SysItemStatus::Failed => "failed",
    }
}

pub(super) fn print_sys_summary(outcomes: &[SysItemOutcome]) {
    let mut counts = BTreeMap::<SysItemStatus, usize>::new();
    for outcome in outcomes {
        *counts.entry(outcome.status).or_default() += 1;
    }
    let parts = [
        SysItemStatus::Installed,
        SysItemStatus::AlreadyInstalled,
        SysItemStatus::Skipped,
        SysItemStatus::Updated,
        SysItemStatus::NeedsAction,
        SysItemStatus::Completed,
        SysItemStatus::Failed,
    ]
    .into_iter()
    .filter_map(|status| {
        counts
            .get(&status)
            .map(|count| format!("{count} {}", status_text(status)))
    })
    .collect::<Vec<_>>();
    println!("{}", colors::dim(&format!("Summary: {}", parts.join(", "))));
}
