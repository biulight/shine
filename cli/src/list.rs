use crate::apps::load_active_categories;
use crate::colors;
use crate::config::Config;
use crate::info::UpdateDiffs;
use crate::output;
use crate::status::{
    AppRow, FileStatus, ShellRow, build_app_rows, build_app_rows_with_lifecycle, build_shell_rows,
};
use crate::sys;
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};

const SHELL_PRESET_PRESENT_LINK_MISSING: &str = "preset present, bin symlink missing";

pub async fn handle_update_list(config: &Config, diff: bool) -> Result<bool> {
    let shell_rows = build_shell_rows(config).await?;
    let shell_lifecycle = crate::shells::collect_update_lifecycle_result(config).await?;
    let pending_shell = shell_lifecycle
        .outcomes
        .iter()
        .filter(|outcome| outcome.status == utils::lifecycle::LifecycleStatus::Pending)
        .map(|outcome| outcome.target.as_str())
        .collect::<BTreeSet<_>>();
    let update_shell: Vec<&ShellRow> = shell_rows
        .iter()
        .filter(|r| {
            r.is_installed
                && pending_shell.contains(
                    format!(
                        "shell/{}/{}",
                        r.category,
                        r.label.split('/').next_back().unwrap_or(&r.label)
                    )
                    .as_str(),
                )
        })
        .collect();

    let cats_result = load_active_categories(config, None).await;
    let (app_rows, app_lifecycle) = match cats_result {
        Ok(cats) => build_app_rows_with_lifecycle(config, &cats).await?,
        Err(_) => (
            Vec::new(),
            utils::lifecycle::LifecycleResultV1::new(
                utils::lifecycle::LifecycleOperation::Update,
                false,
            ),
        ),
    };
    let pending_app = app_lifecycle
        .outcomes
        .iter()
        .filter(|outcome| outcome.status == utils::lifecycle::LifecycleStatus::Pending)
        .map(|outcome| outcome.target.as_str())
        .collect::<BTreeSet<_>>();
    let update_app: Vec<&AppRow> = app_rows
        .iter()
        .filter(|r| {
            r.file_status == FileStatus::UpdateAvail
                && pending_app.contains(format!("app/{}", r.category).as_str())
        })
        .collect();
    let update_app = app_update_categories(&update_app);
    let update_sys = sys::managed_updates(config).await.unwrap_or_default();

    let any = !update_shell.is_empty() || !update_app.is_empty() || !update_sys.is_empty();
    if !any {
        return Ok(false);
    }

    crate::config::print_presets_note(config);

    if !diff {
        let shell_names = shell_categories(&update_shell);
        let app_names = update_app
            .keys()
            .map(|category| (*category).to_string())
            .collect::<Vec<_>>();
        let sys_names = sorted_names(update_sys.iter().map(|row| row.item_id.clone()).collect());

        let mut separator = output::SectionSeparator::new();
        print_name_section(&mut separator, "Shell Presets", &shell_names);
        print_name_section(&mut separator, "App Configs", &app_names);
        print_name_section(&mut separator, "System Configs", &sys_names);
        print_update_hint();
        return Ok(true);
    }

    let update_diffs = UpdateDiffs::collect(config).await?;

    if !update_shell.is_empty() {
        println!("{}", colors::bold("Shell Presets"));

        let label_width = update_shell
            .iter()
            .map(|r| r.label.len())
            .max()
            .unwrap_or(0);

        for row in &update_shell {
            let pad = " ".repeat(label_width.saturating_sub(row.label.len()));
            println!(
                "  {}  {}{}  {}",
                row.symbol,
                row.label,
                pad,
                colors::status_label(row.status_text, row.status_sym),
            );
            update_diffs.print_shell_for_row(config, &row.label).await?;
        }
    }

    if !update_app.is_empty() {
        if !update_shell.is_empty() {
            println!();
        }
        println!("{}", colors::bold("App Configs"));

        let label_width = update_app
            .keys()
            .map(|category| category.len())
            .max()
            .unwrap_or(0);

        for (category, rows) in &update_app {
            let pad = " ".repeat(label_width.saturating_sub(category.len()));
            println!(
                "  {}  {}{}  {}",
                colors::symbol("↑"),
                category,
                pad,
                colors::status_label("update available", "↑"),
            );
            for row in rows {
                print_app_update_detail(row);
                update_diffs.print_app_for_row(config, &row.label).await?;
            }
        }
    }

    if !update_sys.is_empty() {
        if !update_shell.is_empty() || !update_app.is_empty() {
            println!();
        }
        println!("{}", colors::bold("System Configs"));
        for row in &update_sys {
            println!(
                "  {}  {}  {}  {}",
                colors::symbol("↑"),
                row.label,
                colors::dim(&format!("({})", row.item_id)),
                colors::status_label("update available", "↑"),
            );
            for detail in &row.details {
                println!("     {}", colors::dim(detail));
            }
        }
    }

    print_update_hint();

    Ok(true)
}

fn print_update_hint() {
    println!();
    println!("{}", colors::dim("Run `shine upgrade` to apply updates."));
}

fn app_update_categories<'a>(rows: &[&'a AppRow]) -> BTreeMap<&'a str, Vec<&'a AppRow>> {
    let mut categories = BTreeMap::new();
    for row in rows {
        categories
            .entry(row.category.as_str())
            .or_insert_with(Vec::new)
            .push(*row);
    }
    categories
}

fn print_app_update_detail(row: &AppRow) {
    let destination = row
        .dest
        .as_deref()
        .map(|dest| format!("  {}  {}", colors::dim("→"), colors::dim(dest)))
        .unwrap_or_default();
    println!(
        "     {}  {}{}  {}",
        colors::symbol("↑"),
        row.label,
        destination,
        colors::status_label("update available", "↑"),
    );
}

pub async fn handle_status_list(config: &Config, diff: bool) -> Result<()> {
    crate::config::print_presets_note(config);
    let shell_rows = build_shell_rows(config).await?;
    let installed_shell: Vec<&ShellRow> = shell_rows.iter().filter(|r| r.is_installed).collect();
    let all_shell: Vec<&ShellRow> = shell_rows.iter().collect();

    let cats_result = load_active_categories(config, None).await;
    let app_rows = match cats_result {
        Ok(cats) => build_app_rows(config, &cats).await?,
        Err(_) => Vec::new(),
    };
    let installed_app: Vec<&AppRow> = app_rows
        .iter()
        .filter(|r| r.file_status != FileStatus::NotInstalled)
        .collect();
    let all_app: Vec<&AppRow> = app_rows.iter().collect();
    let update_sys = sys::managed_updates(config).await.unwrap_or_default();

    let any = !installed_shell.is_empty() || !installed_app.is_empty() || !update_sys.is_empty();

    if !any {
        println!(
            "{}",
            colors::dim("Nothing installed yet. Run `shine shell install` or `shine app install`.")
        );
        return Ok(());
    }

    let update_diffs = if diff {
        Some(UpdateDiffs::collect(config).await?)
    } else {
        None
    };
    let shell_statuses = if diff {
        installed_shell
            .iter()
            .map(|row| ShellLifecycleStatus {
                category: row.category.clone(),
                detail_label: row.label.clone(),
                status_sym: row.status_sym,
                status_text: row.status_text,
            })
            .collect()
    } else {
        shell_category_statuses(&all_shell)
    };
    let app_statuses = if diff {
        installed_app
            .iter()
            .map(|row| AppLifecycleStatus {
                category: row.category.clone(),
                detail_label: row.label.clone(),
                sym: row.sym,
                status_text: row.status_text,
                file_status: row.file_status,
                dest: row.dest.clone(),
            })
            .collect()
    } else {
        app_category_statuses(&all_app)
    };

    // ── Shell Presets ────────────────────────────────────────────────────────
    if !installed_shell.is_empty() {
        println!("{}", colors::bold("Shell Presets"));

        let label_width = if diff {
            installed_shell.iter().map(|row| row.label.len()).max()
        } else {
            shell_statuses.iter().map(|row| row.category.len()).max()
        }
        .unwrap_or(0);

        for row in &shell_statuses {
            let label = if diff {
                &row.detail_label
            } else {
                &row.category
            };
            let pad = " ".repeat(label_width.saturating_sub(label.len()));
            let run_hint = if row.status_sym == "↑" {
                format!("  {}", colors::dim("run `shine upgrade`"))
            } else {
                String::new()
            };
            println!(
                "  {}  {}{}  {}{}",
                colors::symbol(row.status_sym),
                label,
                pad,
                colors::status_label(row.status_text, row.status_sym),
                run_hint,
            );
            if diff
                && row.status_sym == "↑"
                && let Some(diffs) = &update_diffs
            {
                diffs.print_shell_for_row(config, &row.detail_label).await?;
            }
        }
    }

    // ── App Configs ──────────────────────────────────────────────────────────
    if !installed_app.is_empty() {
        if !installed_shell.is_empty() {
            println!();
        }
        println!("{}", colors::bold("App Configs"));

        let label_width = if diff {
            installed_app.iter().map(|row| row.label.len()).max()
        } else {
            app_statuses.iter().map(|row| row.category.len()).max()
        }
        .unwrap_or(0);

        let mut up_to_date = 0usize;
        let mut update_available = 0usize;
        let mut user_modified = 0usize;
        let mut missing = 0usize;

        for row in &app_statuses {
            let label = if diff {
                &row.detail_label
            } else {
                &row.category
            };
            let pad = " ".repeat(label_width.saturating_sub(label.len()));
            let dest_part = if diff {
                row.dest
                    .as_deref()
                    .map(|d| format!("  {}  {}", colors::dim("→"), colors::dim(d)))
                    .unwrap_or_default()
            } else {
                String::new()
            };

            let run_hint = if row.sym == "↑" {
                format!("  {}", colors::dim("run `shine upgrade`"))
            } else {
                String::new()
            };

            println!(
                "  {}  {}{}{}  {}{}",
                colors::symbol(row.sym),
                label,
                pad,
                dest_part,
                colors::status_label(row.status_text, row.sym),
                run_hint,
            );

            if diff
                && row.file_status == FileStatus::UpdateAvail
                && let Some(diffs) = &update_diffs
            {
                diffs.print_app_for_row(config, &row.detail_label).await?;
            }

            match row.file_status {
                FileStatus::Missing => missing += 1,
                FileStatus::UserModified | FileStatus::Partial => user_modified += 1,
                FileStatus::UpdateAvail => update_available += 1,
                FileStatus::UpToDate => up_to_date += 1,
                FileStatus::NotInstalled => {}
            }
        }

        let parts = app_status_summary_parts(up_to_date, update_available, user_modified, missing);
        if !parts.is_empty() {
            output::footer("Summary", &parts);
        }
    }

    if !update_sys.is_empty() {
        if !installed_shell.is_empty() || !installed_app.is_empty() {
            println!();
        }
        println!("{}", colors::bold("System Configs"));
        for row in &update_sys {
            println!(
                "  {}  {}  {}  {}  {}",
                colors::symbol("↑"),
                row.label,
                colors::dim(&format!("({})", row.item_id)),
                colors::status_label("update available", "↑"),
                colors::dim("run `shine upgrade`"),
            );
            for detail in &row.details {
                println!("     {}", colors::dim(detail));
            }
        }
    }

    Ok(())
}

pub async fn handle_list(config: &Config) -> Result<()> {
    crate::config::print_presets_note(config);
    let shell_rows = build_shell_rows(config).await?;
    let installed_shell: Vec<String> = shell_rows
        .iter()
        .filter(|r| should_show_shell_in_simple_list(r))
        .map(|r| r.category.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    let cats_result = load_active_categories(config, None).await;
    let installed_app = match cats_result {
        Ok(cats) => {
            let app_rows = build_app_rows(config, &cats).await?;
            installed_app_categories(&app_rows)
        }
        Err(_) => Vec::new(),
    };
    let installed_sys = sys::installed_managed(config).await?;
    let installed_sys: Vec<String> = installed_sys
        .iter()
        .map(|row| row.item_id.clone())
        .collect();

    let installed_shell = sorted_names(installed_shell);
    let installed_app = sorted_names(installed_app);
    let installed_sys = sorted_names(installed_sys);

    let any = !installed_shell.is_empty() || !installed_app.is_empty() || !installed_sys.is_empty();

    if !any {
        println!(
            "{}",
            colors::dim(
                "Nothing installed yet. Run `shine shell install`, `shine app install`, or `shine sys list`."
            )
        );
        return Ok(());
    }

    let mut separator = output::SectionSeparator::new();
    print_name_section(&mut separator, "Shell Presets", &installed_shell);
    print_name_section(&mut separator, "App Configs", &installed_app);
    print_name_section(&mut separator, "System Configs", &installed_sys);

    Ok(())
}

fn print_name_section(separator: &mut output::SectionSeparator, title: &str, names: &[String]) {
    if names.is_empty() {
        return;
    }

    separator.begin();
    println!("{} {}", colors::cyan("==>"), colors::bold(title));
    output::print_columns(names);
}

fn installed_app_categories(rows: &[AppRow]) -> Vec<String> {
    rows.iter()
        .filter(|row| row.file_status != FileStatus::NotInstalled)
        .map(|row| row.category.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn shell_categories(rows: &[&ShellRow]) -> Vec<String> {
    rows.iter()
        .map(|row| row.category.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

struct ShellLifecycleStatus {
    category: String,
    detail_label: String,
    status_sym: &'static str,
    status_text: &'static str,
}

fn shell_category_statuses(rows: &[&ShellRow]) -> Vec<ShellLifecycleStatus> {
    let mut grouped: BTreeMap<&str, Vec<&ShellRow>> = BTreeMap::new();
    for row in rows {
        grouped.entry(&row.category).or_default().push(row);
    }
    grouped
        .into_iter()
        .filter_map(|(category, rows)| {
            if !rows.iter().any(|row| row.is_installed) {
                return None;
            }
            if rows.len() == 1 {
                let row = rows[0];
                return Some(ShellLifecycleStatus {
                    category: category.to_string(),
                    detail_label: row.label.clone(),
                    status_sym: row.status_sym,
                    status_text: row.status_text,
                });
            }
            let selected = rows
                .iter()
                .filter(|row| row.is_installed)
                .max_by_key(|row| shell_status_priority(row.status_sym))
                .expect("grouped shell category is non-empty");
            let partially_installed = rows.iter().any(|row| !row.is_installed);
            let (status_sym, status_text) = if partially_installed && selected.status_sym == "✓" {
                ("~", "partial install")
            } else {
                (selected.status_sym, selected.status_text)
            };
            Some(ShellLifecycleStatus {
                category: category.to_string(),
                detail_label: category.to_string(),
                status_sym,
                status_text,
            })
        })
        .collect()
}

fn shell_status_priority(sym: &str) -> usize {
    match sym {
        "!" => 4,
        "~" => 3,
        "↑" => 2,
        "✓" => 1,
        _ => 0,
    }
}

struct AppLifecycleStatus {
    category: String,
    detail_label: String,
    sym: &'static str,
    status_text: &'static str,
    file_status: FileStatus,
    dest: Option<String>,
}

fn app_category_statuses(rows: &[&AppRow]) -> Vec<AppLifecycleStatus> {
    let mut grouped: BTreeMap<&str, Vec<&AppRow>> = BTreeMap::new();
    for row in rows {
        grouped.entry(&row.category).or_default().push(row);
    }
    grouped
        .into_iter()
        .filter_map(|(category, rows)| {
            let has_installed = rows
                .iter()
                .any(|row| row.file_status != FileStatus::NotInstalled);
            if !has_installed {
                return None;
            }
            if rows.len() == 1 {
                let row = rows[0];
                return Some(AppLifecycleStatus {
                    category: category.to_string(),
                    detail_label: row.label.clone(),
                    sym: row.sym,
                    status_text: row.status_text,
                    file_status: row.file_status,
                    dest: row.dest.clone(),
                });
            }
            let has_not_installed = rows
                .iter()
                .any(|row| row.file_status == FileStatus::NotInstalled);
            let installed_max = rows
                .iter()
                .map(|row| row.file_status)
                .filter(|status| *status != FileStatus::NotInstalled)
                .max()
                .expect("installed app category has an installed row");
            let status = if has_not_installed && installed_max == FileStatus::UpToDate {
                FileStatus::Partial
            } else {
                installed_max
            };
            let (sym, status_text) = match status {
                FileStatus::Missing => ("!", "destination missing"),
                FileStatus::UserModified => ("~", "user modified"),
                FileStatus::Partial => ("~", "partial install"),
                FileStatus::UpdateAvail => ("↑", "update available"),
                FileStatus::UpToDate => ("✓", "up-to-date"),
                FileStatus::NotInstalled => unreachable!(),
            };
            Some(AppLifecycleStatus {
                category: category.to_string(),
                detail_label: category.to_string(),
                sym,
                status_text,
                file_status: status,
                dest: None,
            })
        })
        .collect()
}

fn sorted_names(mut names: Vec<String>) -> Vec<String> {
    names.sort_by(|left, right| {
        left.to_lowercase()
            .cmp(&right.to_lowercase())
            .then_with(|| left.cmp(right))
    });
    names
}

fn should_show_shell_in_simple_list(row: &ShellRow) -> bool {
    row.is_installed && row.status_text != SHELL_PRESET_PRESENT_LINK_MISSING
}

fn app_status_summary_parts(
    up_to_date: usize,
    update_available: usize,
    user_modified: usize,
    missing: usize,
) -> Vec<String> {
    let mut parts = Vec::new();
    output::push_count(&mut parts, up_to_date, colors::green, "up-to-date");
    output::push_count(
        &mut parts,
        update_available,
        colors::cyan,
        "update available",
    );
    output::push_count(&mut parts, user_modified, colors::yellow, "user-modified");
    output::push_count(&mut parts, missing, colors::yellow, "destination missing");
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell_row(status_text: &'static str, is_installed: bool) -> ShellRow {
        ShellRow {
            category: "proxy".to_string(),
            symbol: String::new(),
            label: "proxy/setproxy".to_string(),
            status_sym: "~",
            status_text,
            is_installed,
            link_conflict: false,
            changes: Vec::new(),
        }
    }

    fn app_row(category: &str, file_status: FileStatus) -> AppRow {
        AppRow {
            category: category.to_string(),
            sym: "✓",
            label: category.to_string(),
            simple_label: category.to_string(),
            dest: None,
            status_text: "up-to-date",
            file_status,
        }
    }

    #[test]
    fn update_rows_group_app_files_by_category() {
        let first = app_row("clash-verge", FileStatus::UpdateAvail);
        let mut second = app_row("clash-verge", FileStatus::UpdateAvail);
        second.label = "clash-verge/rules/lan.list".to_string();
        let other = app_row("surge", FileStatus::UpdateAvail);
        let grouped = app_update_categories(&[&first, &second, &other]);

        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped["clash-verge"].len(), 2);
        assert_eq!(grouped["surge"].len(), 1);
    }

    #[test]
    fn update_rows_collapse_shell_commands_to_their_category() {
        let first = shell_row("update available", true);
        let mut second = shell_row("update available", true);
        second.label = "proxy/usetproxy".to_string();

        assert_eq!(shell_categories(&[&first, &second]), vec!["proxy"]);
    }

    #[test]
    fn default_shell_status_collapses_commands_and_reports_partial_install() {
        let mut installed = shell_row("up-to-date", true);
        installed.status_sym = "✓";
        let mut missing = shell_row("not installed", false);
        missing.label = "proxy/usetproxy".to_string();
        missing.status_sym = "✗";

        let statuses = shell_category_statuses(&[&installed, &missing]);

        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].category, "proxy");
        assert_eq!(statuses[0].status_text, "partial install");
    }

    #[test]
    fn default_app_status_collapses_files_and_reports_partial_install() {
        let installed = app_row("surge", FileStatus::UpToDate);
        let missing = app_row("surge", FileStatus::NotInstalled);

        let statuses = app_category_statuses(&[&installed, &missing]);

        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].category, "surge");
        assert_eq!(statuses[0].file_status, FileStatus::Partial);
    }

    #[test]
    fn simple_list_hides_preset_present_when_bin_symlink_missing() {
        let row = shell_row(SHELL_PRESET_PRESENT_LINK_MISSING, true);

        assert!(!should_show_shell_in_simple_list(&row));
    }

    #[test]
    fn simple_list_keeps_other_installed_shell_states() {
        assert!(should_show_shell_in_simple_list(&shell_row(
            "up-to-date",
            true
        )));
        assert!(should_show_shell_in_simple_list(&shell_row(
            "bin symlink present, preset missing",
            true
        )));
        assert!(should_show_shell_in_simple_list(&shell_row(
            "update available",
            true
        )));
    }

    #[test]
    fn simple_list_hides_uninstalled_shell_rows() {
        let row = shell_row("not installed", false);

        assert!(!should_show_shell_in_simple_list(&row));
    }

    #[test]
    fn simple_list_collapses_installed_app_files_to_their_category() {
        let rows = vec![
            app_row("surge", FileStatus::UpToDate),
            app_row("surge", FileStatus::Missing),
            app_row("ghostty", FileStatus::NotInstalled),
        ];

        assert_eq!(installed_app_categories(&rows), vec!["surge"]);
    }

    #[test]
    fn simple_list_shows_partially_installed_app_categories() {
        let rows = vec![
            app_row("surge", FileStatus::NotInstalled),
            app_row("surge", FileStatus::UserModified),
        ];

        assert_eq!(installed_app_categories(&rows), vec!["surge"]);
    }

    #[test]
    fn simple_list_sorts_names_case_insensitively() {
        assert_eq!(
            sorted_names(vec![
                "surge".to_string(),
                "JetBrains".to_string(),
                "ghostty".to_string(),
            ]),
            vec!["ghostty", "JetBrains", "surge"]
        );
    }

    #[test]
    fn app_status_summary_parts_includes_only_nonzero_counts() {
        assert_eq!(
            app_status_summary_parts(3, 1, 0, 0),
            vec!["3 up-to-date".to_string(), "1 update available".to_string()]
        );
    }

    #[test]
    fn app_status_summary_parts_empty_when_all_zero() {
        assert!(app_status_summary_parts(0, 0, 0, 0).is_empty());
    }

    #[test]
    fn app_status_summary_parts_reports_all_four_counters() {
        assert_eq!(
            app_status_summary_parts(1, 2, 3, 4),
            vec![
                "1 up-to-date".to_string(),
                "2 update available".to_string(),
                "3 user-modified".to_string(),
                "4 destination missing".to_string(),
            ]
        );
    }
}
