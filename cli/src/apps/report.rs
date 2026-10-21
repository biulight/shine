use std::collections::BTreeSet;
use std::path::Path;

use crate::colors;
use crate::config::Config;
use crate::output::escape_controls;
use crate::path_display;

pub(super) fn symbol(value: &str) -> String {
    colors::symbol(value)
}

pub(super) fn transform_label(transforms: &[String]) -> String {
    if transforms.is_empty() {
        String::new()
    } else {
        format!("  {}", colors::dim(&format!("[{}]", transforms.join(", "))))
    }
}

pub(super) fn app_configs_summary_text(total_available: usize) -> String {
    let noun = if total_available == 1 {
        "file"
    } else {
        "files"
    };
    crate::output::summary_line_text(
        "App Configs",
        &[colors::dim(&format!("{total_available} {noun} available"))],
    )
}

pub(super) fn dry_run_header_text() -> String {
    colors::dim("[dry-run] No files will be modified.")
}

pub(super) fn generator_unavailable_text(display_name: &str, error: &anyhow::Error) -> String {
    let display_name = escape_controls(display_name);
    let error = escape_controls(&format!("{error:#}"));
    format!(
        "  {} {display_name}: generator unavailable; installed copy kept ({error})",
        colors::symbol_stderr("!")
    )
}

pub(super) fn restart_hint_text(hint: &str) -> String {
    format!(
        "  {} {}",
        colors::symbol("!"),
        colors::yellow(&escape_controls(hint))
    )
}

pub(super) fn install_summary_parts(
    installed: usize,
    backed_up: usize,
    skipped: usize,
    previewed: usize,
    failed: usize,
) -> Vec<String> {
    let mut parts = Vec::new();
    if installed > 0 {
        let backup_note = if backed_up > 0 {
            format!(", {backed_up} backed up")
        } else {
            String::new()
        };
        parts.push(colors::green(&format!(
            "{installed} installed{backup_note}"
        )));
    }
    if skipped > 0 {
        parts.push(colors::dim(&format!("{skipped} skipped")));
    }
    crate::output::push_count(&mut parts, previewed, colors::cyan, "would install");
    crate::output::push_count(&mut parts, failed, colors::red, "failed");
    parts
}

pub(super) fn no_installed_files_text(category: &str) -> String {
    let category = escape_controls(category);
    colors::dim(&format!(
        "No installed files found for category '{category}'."
    ))
}

pub(super) fn cache_cleanup_lines(
    outcome: &shine_core::lifecycle::LifecycleOutcomeV1,
) -> Vec<String> {
    use shine_core::lifecycle::LifecycleEffect;
    if outcome.resource.as_deref() != Some("purge") {
        return Vec::new();
    }
    let mut lines = Vec::new();
    for (effect, text) in [
        (
            LifecycleEffect::CacheRemovePreviewed,
            format!("would remove {} preset cache", outcome.target),
        ),
        (
            LifecycleEffect::CachePurged,
            format!("{} preset cache removed", outcome.target),
        ),
        (
            LifecycleEffect::ReceiptRemovePreviewed,
            "would remove empty App manifest".to_string(),
        ),
        (
            LifecycleEffect::ReceiptRemoved,
            "empty App manifest removed".to_string(),
        ),
    ] {
        if outcome.effects.contains(&effect) {
            let preview = matches!(
                effect,
                LifecycleEffect::CacheRemovePreviewed | LifecycleEffect::ReceiptRemovePreviewed
            );
            lines.push(format!(
                "  {}  {}",
                colors::symbol(if preview { "·" } else { "✓" }),
                colors::dim(&text)
            ));
        }
    }
    lines
}

pub(super) fn uninstall_summary_parts(
    removed: usize,
    restored: usize,
    user_modified: usize,
    skipped: usize,
) -> Vec<String> {
    let mut parts = Vec::new();
    if removed > 0 {
        let restore_note = if restored > 0 {
            format!(", {restored} backups restored")
        } else {
            String::new()
        };
        parts.push(colors::green(&format!("{removed} removed{restore_note}")));
    }
    if user_modified > 0 {
        parts.push(colors::yellow(&format!("{user_modified} protected")));
    }
    if skipped > 0 {
        parts.push(colors::dim(&format!("{skipped} skipped")));
    }
    parts
}

pub(super) fn upgrade_header_text(verbose: bool, installed_count: usize) -> String {
    if verbose {
        crate::output::summary_line_text(
            "App Configs",
            &[colors::dim(&format!("{installed_count} installed file(s)"))],
        )
    } else {
        colors::bold("App Configs")
    }
}

pub(super) fn up_to_date_text(source: &str) -> String {
    let source = escape_controls(source);
    format!("  {} {source}: up to date", colors::symbol("✓"))
}

pub(super) fn category_updated_text(category: &str, count: usize) -> String {
    let category = escape_controls(category);
    let noun = if count == 1 { "file" } else { "files" };
    format!(
        "  {} {category}  {}",
        colors::symbol("✓"),
        colors::dim(&format!("{count} {noun} updated"))
    )
}

pub(super) fn artifact_apply_hint_text(category: &str) -> String {
    if category.chars().any(char::is_control) {
        return format!(
            "  {} {}: managed files changed; run `shine app artifact apply` with the original category argument (copyable command omitted)",
            colors::symbol("!"),
            escape_controls(category)
        );
    }
    let category_arg = crate::shell_quote::command_positionals(&[category]);
    format!(
        "  {} {category}: managed files changed; run `shine app artifact apply {category_arg}`",
        colors::symbol("!")
    )
}

pub(super) fn artifact_apply_categories(
    artifact_categories: &BTreeSet<String>,
    changed_categories: BTreeSet<String>,
) -> BTreeSet<String> {
    artifact_categories
        .intersection(&changed_categories)
        .cloned()
        .collect()
}

pub(super) fn warning_text(source: &str, detail: impl AsRef<str>) -> String {
    let source = escape_controls(source);
    format!(
        "  {} {source}: {}",
        colors::symbol_stderr("!"),
        escape_controls(detail.as_ref())
    )
}

// --- Install/upgrade outcome reporting --------------------------------------------------

pub(super) fn print_install_success(
    label: &str,
    transform_label: &str,
    destination: &Path,
    config: &Config,
) {
    println!(
        "{}",
        install_success_text(label, transform_label, destination, config)
    );
}

pub(super) fn install_success_text(
    label: &str,
    transform_label: &str,
    destination: &Path,
    config: &Config,
) -> String {
    format!(
        "  {} {}{}  {}  {}",
        colors::symbol("✓"),
        escape_controls(label),
        transform_label,
        colors::dim("→"),
        colors::dim(&escape_controls(&path_display::format_home(
            destination,
            &config.home_dir
        ))),
    )
}

pub(super) fn install_success_with_backup_text(
    label: &str,
    transform_label: &str,
    destination: &Path,
    backup: &Path,
    config: &Config,
) -> String {
    format!(
        "  {}  {}{}  {}  {}  {}",
        colors::symbol("✓"),
        escape_controls(label),
        transform_label,
        colors::dim("→"),
        colors::dim(&escape_controls(&path_display::format_home(
            destination,
            &config.home_dir
        ))),
        colors::dim(&format!(
            "(backup: {})",
            escape_controls(&path_display::format_home(backup, &config.home_dir))
        )),
    )
}

pub(super) fn already_managed_text(label: &str) -> String {
    format!(
        "  {}  {}  {}",
        colors::dim("-"),
        escape_controls(label),
        colors::dim("already up to date"),
    )
}

pub(super) fn dry_run_install_text(
    label: &str,
    transform_label: &str,
    destination: &Path,
    config: &Config,
) -> String {
    format!(
        "  {}  {}{}  {}  {}",
        colors::dim("[dry-run]"),
        escape_controls(label),
        transform_label,
        colors::dim("→"),
        colors::dim(&escape_controls(&path_display::format_home(
            destination,
            &config.home_dir
        ))),
    )
}

pub(super) fn print_install_error(label: &str, err: &anyhow::Error) {
    eprintln!("{}", install_error_text(label, err));
}

pub(super) fn install_error_text(label: &str, err: &anyhow::Error) -> String {
    format!(
        "  {} {}: {}",
        colors::symbol_stderr("✗"),
        escape_controls(label),
        escape_controls(&format!("{err:#}"))
    )
}

// --- Stale-entry cleanup reporting -------------------------------------------------------

pub(super) fn stale_removed_text(
    config: &Config,
    destination: &Path,
    note: impl AsRef<str>,
) -> String {
    format!(
        "  {} {}  {}",
        colors::symbol("✓"),
        colors::dim(&escape_controls(&path_display::format_home(
            destination,
            &config.home_dir
        ))),
        colors::dim(&escape_controls(note.as_ref())),
    )
}

// --- Uninstall outcome reporting ----------------------------------------------------------

pub(super) fn removed_text(config: &Config, destination: &Path) -> String {
    format!(
        "  {}  {}",
        colors::symbol("✓"),
        colors::dim(&escape_controls(&path_display::format_home(
            destination,
            &config.home_dir
        ))),
    )
}

pub(super) fn removed_with_restore_text(
    config: &Config,
    destination: &Path,
    backup: &Path,
) -> String {
    format!(
        "  {}  {}  {}",
        colors::symbol("✓"),
        colors::dim(&escape_controls(&path_display::format_home(
            destination,
            &config.home_dir
        ))),
        colors::dim(&format!(
            "(restored {})",
            escape_controls(&path_display::format_home(backup, &config.home_dir))
        )),
    )
}

pub(super) fn force_removed_text(destination: &Path) -> String {
    format!(
        "  {}  {}  {}",
        colors::symbol("✓"),
        colors::dim(&escape_controls(&destination.display().to_string())),
        colors::dim("force removed"),
    )
}

pub(super) fn force_removed_with_restore_text(destination: &Path, backup: &Path) -> String {
    format!(
        "  {}  {}  {}",
        colors::symbol("✓"),
        colors::dim(&escape_controls(&destination.display().to_string())),
        colors::dim(&format!(
            "force removed, restored {}",
            escape_controls(&backup.display().to_string())
        )),
    )
}

pub(super) fn uninstall_not_found_text(config: &Config, destination: &Path) -> String {
    format!(
        "  {}  {}  {}",
        colors::dim("-"),
        colors::dim(&escape_controls(&path_display::format_home(
            destination,
            &config.home_dir
        ))),
        colors::dim("not found, installation receipt removed; recorded backup left in place"),
    )
}

pub(super) fn user_modified_kept_text(
    config: &Config,
    destination: &Path,
    stream: owo_colors::Stream,
) -> String {
    format!(
        "  {}  {}  {}",
        colors::symbol_for_stream("!", stream),
        escape_controls(&path_display::format_home(destination, &config.home_dir)),
        colors::yellow_for_stream("differs from installation receipt", stream),
    )
}

pub(super) fn uninstall_dry_run_text(config: &Config, destination: &Path) -> String {
    format!(
        "  {}  {}  would remove managed content if present and its installation receipt",
        colors::dim("[dry-run]"),
        colors::dim(&escape_controls(&path_display::format_home(
            destination,
            &config.home_dir
        ))),
    )
}

pub(super) fn uninstall_error_text(
    config: &Config,
    destination: &Path,
    err: &anyhow::Error,
) -> String {
    format!(
        "  {} {}: {}",
        colors::symbol_stderr("✗"),
        escape_controls(&path_display::format_home(destination, &config.home_dir)),
        escape_controls(&err.to_string())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_outcome_data_controls_preserve_single_rows_and_error_depth() {
        let home = std::path::PathBuf::from("/tmp/shine-report-home");
        let config = crate::test_support::test_config(&home);
        let value = "name\nFORGED\r\t\u{1b}\\literal中文";
        let destination = home.join(value);
        let backup = destination.with_extension("bak");
        let error = anyhow::anyhow!("cause\nFORGED\r\t\u{1b}").context(value.to_string());
        let rows = [
            generator_unavailable_text(value, &error),
            restart_hint_text(value),
            no_installed_files_text(value),
            up_to_date_text(value),
            category_updated_text(value, 2),
            warning_text(value, value),
            install_success_text(value, "", &destination, &config),
            install_success_with_backup_text(value, "", &destination, &backup, &config),
            already_managed_text(value),
            dry_run_install_text(value, "", &destination, &config),
            install_error_text(value, &error),
            stale_removed_text(&config, &destination, value),
            removed_text(&config, &destination),
            removed_with_restore_text(&config, &destination, &backup),
            force_removed_text(&destination),
            force_removed_with_restore_text(&destination, &backup),
            uninstall_not_found_text(&config, &destination),
            user_modified_kept_text(&config, &destination, owo_colors::Stream::Stdout),
            user_modified_kept_text(&config, &destination, owo_colors::Stream::Stderr),
            uninstall_dry_run_text(&config, &destination),
            uninstall_error_text(&config, &destination, &error),
        ];
        for row in rows {
            assert!(!row.chars().any(char::is_control), "{row:?}");
            assert!(
                row.contains(r"name\nFORGED\r\t\u{1b}") && row.contains("中文"),
                "{row:?}"
            );
        }
        assert!(install_error_text(value, &error).contains(r"cause\nFORGED"));
        assert!(!uninstall_error_text(&config, &destination, &error).contains("cause"));
        assert_eq!(
            destination,
            home.join(value),
            "display must not rewrite the path"
        );
    }

    #[test]
    fn artifact_hints_omit_control_bearing_operands_and_keep_normal_quoting() {
        let text = artifact_apply_hint_text("item\nFORGED\r\t\u{1b}");
        assert!(!text.chars().any(char::is_control));
        assert!(text.contains("original category argument (copyable command omitted)"));
        assert!(text.contains(r"item\nFORGED\r\t\u{1b}: managed files changed"));
        for category in ["plain", "two words", "it's", "-leading"] {
            assert_eq!(
                artifact_apply_hint_text(category),
                format!(
                    "  ! {category}: managed files changed; run `shine app artifact apply {}`",
                    crate::shell_quote::command_positionals(&[category])
                )
            );
        }
    }

    #[test]
    fn artifact_apply_categories_only_returns_changed_artifact_categories() {
        let artifacts = BTreeSet::from(["artifact".to_string(), "unchanged".to_string()]);
        let changed = BTreeSet::from(["artifact".to_string(), "ordinary".to_string()]);

        assert_eq!(
            artifact_apply_categories(&artifacts, changed),
            BTreeSet::from(["artifact".to_string()])
        );
    }

    #[test]
    fn app_lifecycle_lines_preserve_spacing_and_home_relative_paths() {
        let home = std::path::PathBuf::from("/tmp/shine-report-home");
        let config = crate::test_support::test_config(&home);
        let destination = home.join(".config/sample/config.toml");
        let backup = home.join(".config/sample/config.toml.shine.bak");

        assert_eq!(
            install_success_text("config.toml", "", &destination, &config),
            "  ✓ config.toml  →  ~/.config/sample/config.toml"
        );
        assert_eq!(
            removed_with_restore_text(&config, &destination, &backup),
            "  ✓  ~/.config/sample/config.toml  (restored ~/.config/sample/config.toml.shine.bak)"
        );
        assert_eq!(
            user_modified_kept_text(&config, &destination, owo_colors::Stream::Stdout),
            "  !  ~/.config/sample/config.toml  differs from installation receipt"
        );
    }
}
