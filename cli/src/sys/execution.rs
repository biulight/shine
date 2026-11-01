use std::collections::BTreeMap;

use crate::colors;
use crate::output::escape_controls;

use super::{ResolvedSelection, SysItemOutcome, SysItemStatus, selection::format_item_ids};

pub(super) fn sys_item_label_width(
    selection: &ResolvedSelection,
    labels: &BTreeMap<&str, String>,
) -> usize {
    selection
        .item_ids
        .iter()
        .filter_map(|id| labels.get(id.as_str()))
        .map(|label| console::measure_text_width(&escape_controls(label)))
        .chain(std::iter::once("profile".len()))
        .max()
        .unwrap_or(14)
        .max(14)
}

pub(super) fn print_run_header(os_id: &str, sys_shell: &str, selection: &ResolvedSelection) {
    println!("{}", colors::bold("System Init"));
    println!("  OS: {os_id}");
    println!("  Shell: {sys_shell}");
    println!(
        "  Selection: {}",
        escape_controls(&selection.source.describe())
    );
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

pub(super) fn item_install_start_line(item_id: &str, label: &str, requires_admin: bool) -> String {
    let admin = if requires_admin {
        " (administrator access required)"
    } else {
        ""
    };
    format!(
        "{} [shine] {}",
        colors::bold_cyan("==>"),
        colors::bold(&format!(
            "sys/{item_id} ({}) installing{admin}",
            escape_controls(label)
        ))
    )
}

pub(crate) fn installer_color_enabled() -> bool {
    use std::io::IsTerminal;
    std::io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
        && console::colors_enabled()
}

pub(crate) fn installer_progress_settings(item_ids: &[String]) -> (bool, Option<u16>) {
    use std::io::IsTerminal;
    if !std::io::stdout().is_terminal() || std::env::var("TERM").as_deref() == Ok("dumb") {
        return (false, None);
    }
    let columns = console::Term::stdout().size().1;
    let label = item_ids
        .iter()
        .map(|id| console::measure_text_width(id))
        .max()
        .unwrap_or(0)
        + 7;
    let width = columns
        .saturating_sub(label.min(u16::MAX as usize) as u16)
        .saturating_sub(1)
        .max(1);
    (
        std::env::var_os("CARGO_TERM_PROGRESS_WHEN").is_none(),
        std::env::var_os("CARGO_TERM_PROGRESS_WIDTH")
            .is_none()
            .then_some(width),
    )
}

pub(super) fn installer_log_line(item_id: &str, text: &str) -> String {
    // Output channels do not imply severity: Cargo writes normal progress to stderr.
    let source = format!("[{}]", escape_controls(item_id));
    // Child terminal controls must not erase the source label or imitate Shine status rows.
    format!("    {} {}", colors::dim(&source), escape_controls(text))
}

pub(super) fn bootstrap_outcome_lines(outcome: &SysItemOutcome, label_width: usize) -> Vec<String> {
    let mut lines = item_outcome_lines(outcome, label_width);
    // The bounded diagnostic tail remains in the report; it has already been streamed.
    lines.truncate(1);
    if outcome.item_id == "profile" {
        // Profile diagnostics are produced after installers and have no live pipe events.
        lines.extend(
            outcome
                .logs
                .iter()
                .map(|text| installer_log_line("profile", text)),
        );
    }
    let symbol = colors::symbol(status_symbol(outcome.status));
    lines[0] = lines[0].replacen(&format!("  {symbol} "), &format!("  {symbol} [shine] "), 1);
    lines
}

pub(super) fn print_item_outcome(outcome: &SysItemOutcome, label_width: usize) {
    for line in item_outcome_lines(outcome, label_width) {
        println!("{line}");
    }
}

pub(super) fn item_outcome_lines(outcome: &SysItemOutcome, label_width: usize) -> Vec<String> {
    item_outcome_lines_with_status(
        outcome,
        label_width,
        status_symbol(outcome.status),
        status_text(outcome.status),
    )
}

pub(super) fn item_outcome_lines_with_status(
    outcome: &SysItemOutcome,
    label_width: usize,
    symbol: &str,
    status: &str,
) -> Vec<String> {
    let label = escape_controls(&outcome.label);
    let padding = label_width.saturating_sub(console::measure_text_width(&label));
    let label = format!("{label}{}", " ".repeat(padding));
    let status = format!("{status:<17}");
    let detail = if outcome.detail.is_empty() {
        String::new()
    } else {
        colors::dim(&outcome.detail)
    };
    let mut lines = vec![format!(
        "  {} {} {} {}",
        colors::symbol(symbol),
        colors::bold(&label),
        colors::status_label(&status, symbol),
        detail
    )];
    for line in &outcome.logs {
        lines.push(format!("    {}", colors::dim(line)));
    }
    lines
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
    println!("{}", sys_summary_text(outcomes));
}

pub(super) fn sys_summary_text(outcomes: &[SysItemOutcome]) -> String {
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
    colors::dim(&format!("Summary: {}", parts.join(", ")))
}

pub(super) fn presentation_bold(value: &str) -> String {
    colors::bold(value)
}

pub(super) fn presentation_dim(value: &str) -> String {
    colors::dim(value)
}

pub(super) fn presentation_symbol(value: &str) -> String {
    colors::symbol(value)
}

pub(super) fn presentation_symbol_stderr(value: &str) -> String {
    colors::symbol_stderr(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_logs_are_tagged_safe_and_not_replayed_in_outcomes() {
        assert_eq!(
            installer_log_line("rust", "download"),
            "    [rust] download"
        );
        assert_eq!(installer_log_line("rust", "warning"), "    [rust] warning");
        assert_eq!(
            installer_log_line("rust", "\x1b[2K\rfake\nrow"),
            "    [rust] \\u{1b}[2K\\rfake\\nrow"
        );
        for status in [SysItemStatus::Installed, SysItemStatus::Failed] {
            let outcome = SysItemOutcome {
                item_id: "rust".into(),
                label: "Rust".into(),
                status,
                detail: "result".into(),
                logs: vec!["already streamed".into()],
            };
            let lines = bootstrap_outcome_lines(&outcome, 14);
            assert_eq!(lines.len(), 1);
            for row in [
                item_install_start_line("rust", "Rust", false),
                installer_log_line("rust", "download"),
                installer_log_line("rust", "warning"),
                lines[0].clone(),
            ] {
                let plain = console::strip_ansi_codes(&row);
                let (prefix, _) = plain.split_once('[').unwrap();
                assert_eq!(console::measure_text_width(prefix), 4, "{plain}");
            }
            assert!(lines[0].contains(status_text(status)));
            assert!(!lines[0].contains("already streamed"));
        }
    }

    #[test]
    fn bootstrap_profile_diagnostics_are_not_lost() {
        let outcome = SysItemOutcome {
            item_id: "profile".into(),
            label: "profile".into(),
            status: SysItemStatus::Updated,
            detail: String::new(),
            logs: vec!["profile note".into()],
        };
        let lines = bootstrap_outcome_lines(&outcome, 14);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1], "    [profile] profile note");
    }

    #[test]
    fn item_install_start_names_item_and_admin_requirement() {
        assert_eq!(
            item_install_start_line("zerotier", "ZeroTier", true),
            "==> [shine] sys/zerotier (ZeroTier) installing (administrator access required)"
        );
        assert_eq!(
            item_install_start_line("zerotier", "ZeroTier", false),
            "==> [shine] sys/zerotier (ZeroTier) installing"
        );
    }

    #[test]
    fn sys_outcome_columns_use_terminal_width() {
        let labels = BTreeMap::from([
            ("ascii", "ASCII".to_string()),
            ("wide", "中文标签中文标签".to_string()),
            ("combining", "e\u{301}".to_string()),
        ]);
        let selection = ResolvedSelection {
            item_ids: labels.keys().map(|id| (*id).to_string()).collect(),
            source: shine_core::runtime::SelectionSource::Items,
        };
        let width = sys_item_label_width(&selection, &labels);
        assert_eq!(width, 16);
        for label in labels.values() {
            let outcome = SysItemOutcome {
                item_id: "item".into(),
                label: label.clone(),
                status: SysItemStatus::Updated,
                detail: "detail".into(),
                logs: vec![],
            };
            let line = &item_outcome_lines(&outcome, width)[0];
            assert_eq!(
                console::measure_text_width(line.split_once("updated").unwrap().0),
                21
            );
            assert_eq!(
                console::measure_text_width(line.split_once("detail").unwrap().0),
                39
            );
        }
    }

    #[test]
    fn managed_item_outcome_lines_preserve_detail_and_log_order() {
        let outcome = SysItemOutcome {
            item_id: "split-dns".to_string(),
            label: "Split DNS".to_string(),
            status: SysItemStatus::Updated,
            detail: "receipt refreshed".to_string(),
            logs: vec!["first".to_string(), "second".to_string()],
        };

        assert_eq!(
            item_outcome_lines(&outcome, 14),
            vec![
                "  ✓ Split DNS      updated           receipt refreshed",
                "    first",
                "    second",
            ]
        );
    }
}
