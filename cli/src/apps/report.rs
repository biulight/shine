use std::collections::BTreeSet;
use std::path::Path;

use crate::colors;
use crate::config::Config;
use crate::path_display;

pub(super) fn symbol(value: &str) -> String {
    colors::symbol(value)
}

pub(super) fn dim(value: &str) -> String {
    colors::dim(value)
}

pub(super) fn transform_label(transforms: &[String]) -> String {
    if transforms.is_empty() {
        String::new()
    } else {
        format!("  {}", colors::dim(&format!("[{}]", transforms.join(", "))))
    }
}

pub(super) fn app_configs_summary_text(total_available: usize) -> String {
    crate::output::summary_line_text(
        "App Configs",
        &[colors::dim(&format!("{total_available} files available"))],
    )
}

pub(super) fn dry_run_header_text() -> String {
    colors::dim("[dry-run] No files will be modified.")
}

pub(super) fn generator_unavailable_text(display_name: &str, error: &anyhow::Error) -> String {
    format!(
        "  {} {display_name}: generator unavailable; installed copy kept ({error:#})",
        colors::symbol("!")
    )
}

pub(super) fn restart_hint_text(hint: &str) -> String {
    format!("  {} {}", colors::symbol("!"), colors::yellow(hint))
}

pub(super) fn done_summary_text(parts: &[String]) -> String {
    crate::output::summary_line_text("Done", parts)
}

pub(super) fn install_summary_parts(
    installed: usize,
    backed_up: usize,
    skipped: usize,
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
    parts
}

pub(super) fn no_installed_files_text(category: &str) -> String {
    colors::dim(&format!(
        "No installed files found for category '{category}'."
    ))
}

pub(super) fn purge_category_text(category: &str) -> String {
    format!(
        "  {}  {}",
        colors::symbol("✓"),
        colors::dim(&format!("app/{category} presets directory purged")),
    )
}

pub(super) fn purge_all_text() -> String {
    format!(
        "  {}  {}",
        colors::symbol("✓"),
        colors::dim("app presets directory and manifest purged"),
    )
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
        parts.push(colors::yellow(&format!(
            "{user_modified} user-modified (kept)"
        )));
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
    format!("  {} {source}: up to date", colors::symbol("✓"))
}

pub(super) fn category_updated_text(category: &str, count: usize) -> String {
    let noun = if count == 1 { "file" } else { "files" };
    format!(
        "  {} {category}  {}",
        colors::symbol("✓"),
        colors::dim(&format!("{count} {noun} updated"))
    )
}

pub(super) fn artifact_apply_hint_text(category: &str) -> String {
    format!(
        "  {} {category}: managed files changed; run `shine app artifact apply {category}`",
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
    format!("  {} {source}: {}", colors::symbol("!"), detail.as_ref())
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
        label,
        transform_label,
        colors::dim("→"),
        colors::dim(&path_display::format_home(destination, &config.home_dir)),
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
        label,
        transform_label,
        colors::dim("→"),
        colors::dim(&path_display::format_home(destination, &config.home_dir)),
        colors::dim(&format!(
            "(backup: {})",
            path_display::format_home(backup, &config.home_dir)
        )),
    )
}

pub(super) fn already_managed_text(label: &str) -> String {
    format!(
        "  {}  {}  {}",
        colors::dim("-"),
        label,
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
        label,
        transform_label,
        colors::dim("→"),
        colors::dim(&path_display::format_home(destination, &config.home_dir)),
    )
}

pub(super) fn print_install_error(label: &str, err: &anyhow::Error) {
    eprintln!("{}", install_error_text(label, err));
}

pub(super) fn install_error_text(label: &str, err: &anyhow::Error) -> String {
    format!("  {} {label}: {err:#}", colors::symbol_stderr("✗"))
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
        colors::dim(&path_display::format_home(destination, &config.home_dir)),
        colors::dim(note.as_ref()),
    )
}

// --- Uninstall outcome reporting ----------------------------------------------------------

pub(super) fn removed_text(config: &Config, destination: &Path) -> String {
    format!(
        "  {}  {}",
        colors::symbol("✓"),
        colors::dim(&path_display::format_home(destination, &config.home_dir)),
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
        colors::dim(&path_display::format_home(destination, &config.home_dir)),
        colors::dim(&format!(
            "(restored {})",
            path_display::format_home(backup, &config.home_dir)
        )),
    )
}

pub(super) fn force_removed_text(destination: &Path) -> String {
    format!(
        "  {}  {}  {}",
        colors::symbol("✓"),
        colors::dim(&destination.display().to_string()),
        colors::dim("force removed"),
    )
}

pub(super) fn force_removed_with_restore_text(destination: &Path, backup: &Path) -> String {
    format!(
        "  {}  {}  {}",
        colors::symbol("✓"),
        colors::dim(&destination.display().to_string()),
        colors::dim(&format!("force removed, restored {}", backup.display())),
    )
}

pub(super) fn uninstall_not_found_text(config: &Config, destination: &Path) -> String {
    format!(
        "  {}  {}  {}",
        colors::dim("-"),
        colors::dim(&path_display::format_home(destination, &config.home_dir)),
        colors::dim("not found, skipped"),
    )
}

pub(super) fn user_modified_kept_text(config: &Config, destination: &Path) -> String {
    format!(
        "  {}  {}  {}",
        colors::symbol("!"),
        path_display::format_home(destination, &config.home_dir),
        colors::yellow("modified after install, left in place"),
    )
}

pub(super) fn uninstall_dry_run_text(config: &Config, destination: &Path) -> String {
    format!(
        "  {}  {}",
        colors::dim("[dry-run]"),
        colors::dim(&path_display::format_home(destination, &config.home_dir)),
    )
}

pub(super) fn uninstall_error_text(
    config: &Config,
    destination: &Path,
    err: &anyhow::Error,
) -> String {
    format!(
        "  {} {}: {err}",
        colors::symbol("✗"),
        path_display::format_home(destination, &config.home_dir)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
            user_modified_kept_text(&config, &destination),
            "  !  ~/.config/sample/config.toml  modified after install, left in place"
        );
    }
}
