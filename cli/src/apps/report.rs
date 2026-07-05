use std::path::Path;

use crate::colors;
use crate::config::Config;
use crate::path_display;

// --- Install/upgrade outcome reporting --------------------------------------------------

pub(super) fn print_install_success(
    label: &str,
    transform_label: &str,
    destination: &Path,
    config: &Config,
) {
    println!(
        "  {}  {}{}  {}  {}",
        colors::symbol("✓"),
        label,
        transform_label,
        colors::dim("→"),
        colors::dim(&path_display::format_home(destination, &config.home_dir)),
    );
}

pub(super) fn print_install_success_with_backup(
    label: &str,
    transform_label: &str,
    destination: &Path,
    backup: &Path,
    config: &Config,
) {
    println!(
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
    );
}

pub(super) fn print_already_managed(label: &str) {
    println!(
        "  {}  {}  {}",
        colors::dim("-"),
        label,
        colors::dim("already up to date"),
    );
}

pub(super) fn print_dry_run_install(
    label: &str,
    transform_label: &str,
    destination: &Path,
    config: &Config,
) {
    println!(
        "  {}  {}{}  {}  {}",
        colors::dim("[dry-run]"),
        label,
        transform_label,
        colors::dim("→"),
        colors::dim(&path_display::format_home(destination, &config.home_dir)),
    );
}

pub(super) fn print_install_error(label: &str, err: &anyhow::Error) {
    eprintln!("  {} {label}: {err:#}", colors::symbol("✗"));
}

// --- Stale-entry cleanup reporting -------------------------------------------------------

pub(super) fn print_stale_removed(config: &Config, destination: &Path, note: impl AsRef<str>) {
    println!(
        "  {}  {}  {}",
        colors::symbol("✓"),
        colors::dim(&path_display::format_home(destination, &config.home_dir)),
        colors::dim(note.as_ref()),
    );
}

pub(super) fn print_stale_not_found(config: &Config, destination: &Path) {
    println!(
        "  {}  {}  {}",
        colors::dim("-"),
        colors::dim(&path_display::format_home(destination, &config.home_dir)),
        colors::dim("stale destination missing, manifest cleaned"),
    );
}

// --- Uninstall outcome reporting ----------------------------------------------------------

pub(super) fn print_removed(config: &Config, destination: &Path) {
    println!(
        "  {}  {}",
        colors::symbol("✓"),
        colors::dim(&path_display::format_home(destination, &config.home_dir)),
    );
}

pub(super) fn print_removed_with_restore(config: &Config, destination: &Path, backup: &Path) {
    println!(
        "  {}  {}  {}",
        colors::symbol("✓"),
        colors::dim(&path_display::format_home(destination, &config.home_dir)),
        colors::dim(&format!(
            "(restored {})",
            path_display::format_home(backup, &config.home_dir)
        )),
    );
}

pub(super) fn print_force_removed(destination: &Path) {
    println!(
        "  {}  {}  {}",
        colors::symbol("✓"),
        colors::dim(&destination.display().to_string()),
        colors::dim("force removed"),
    );
}

pub(super) fn print_force_removed_with_restore(destination: &Path, backup: &Path) {
    println!(
        "  {}  {}  {}",
        colors::symbol("✓"),
        colors::dim(&destination.display().to_string()),
        colors::dim(&format!("force removed, restored {}", backup.display())),
    );
}

pub(super) fn print_uninstall_not_found(config: &Config, destination: &Path) {
    println!(
        "  {}  {}  {}",
        colors::dim("-"),
        colors::dim(&path_display::format_home(destination, &config.home_dir)),
        colors::dim("not found, skipped"),
    );
}

pub(super) fn print_user_modified_kept(config: &Config, destination: &Path) {
    println!(
        "  {}  {}  {}",
        colors::symbol("!"),
        path_display::format_home(destination, &config.home_dir),
        colors::yellow("modified after install, left in place"),
    );
}

pub(super) fn print_uninstall_dry_run(config: &Config, destination: &Path) {
    println!(
        "  {}  {}",
        colors::dim("[dry-run]"),
        colors::dim(&path_display::format_home(destination, &config.home_dir)),
    );
}

pub(super) fn print_uninstall_error(config: &Config, destination: &Path, err: &anyhow::Error) {
    eprintln!(
        "  {} {}: {err}",
        colors::symbol("✗"),
        path_display::format_home(destination, &config.home_dir)
    );
}
