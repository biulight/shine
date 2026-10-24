//! Terminal presentation; diff reads remain at their original output boundaries.
use super::*;
use query::{InventoryInputs, StatusInputs, UpdateInputs};

fn print_app_analysis(files: &[shine_core::runtime::AppFileInspection], detailed: bool) {
    for file in files {
        if let Some(scope) = &file.code_analysis {
            for line in crate::code_analysis::lines(scope, detailed) {
                println!("{line}");
            }
        }
    }
}

fn print_generator_notice(attention: &GeneratorAttention<'_>, run_generators: bool) {
    let GeneratorAttention {
        not_evaluated,
        failures,
    } = attention;
    if !run_generators && *not_evaluated {
        println!();
        println!(
            "{}",
            colors::yellow("! Generated configuration was not evaluated.")
        );
        println!(
            "{}",
            colors::dim(
                "  Update status may be incomplete. Re-run with `--run-generators` to evaluate generator output."
            )
        );
    }
    for row in failures {
        println!();
        println!(
            "{}",
            colors::yellow(&format!(
                "! {}: {}",
                output::escape_controls(&row.label),
                row.status_text
            ))
        );
    }
}

fn print_missing_app_sources(items: &[(String, String)]) {
    for (category, resource) in items {
        println!(
            "{}",
            colors::yellow(&format!(
                "! app/{category} · {resource}: Preset source missing; installed content and receipt retained."
            ))
        );
        let category_arg = crate::shell_quote::command_positionals(&[category]);
        println!("  Preview uninstall: shine app uninstall --dry-run {category_arg}");
    }
}

fn print_shell_attention(rows: &[&ShellRow]) {
    let lines = shell_attention_lines(rows);
    if lines.is_empty() {
        return;
    }
    println!("{}", colors::bold("Shell attention required"));
    for line in lines {
        println!("{line}");
    }
    println!();
}

fn print_action_hints(upgrade_targets: &[String], refresh_commands: &[String]) {
    if upgrade_targets.is_empty() && refresh_commands.is_empty() {
        return;
    }
    println!();
    if !upgrade_targets.is_empty() {
        println!("{}", colors::dim(&update_hint_text(upgrade_targets)));
    }
    for command in refresh_commands {
        println!(
            "{}",
            colors::dim(&format!(
                "Run {} to refresh generated configuration.",
                refresh_command_hint(command)
            ))
        );
    }
}

fn print_app_update_detail(row: &AppRow) {
    let destination = row
        .dest
        .as_deref()
        .map(|dest| {
            format!(
                "  {}  {}",
                colors::dim("→"),
                colors::dim(&output::escape_controls(dest))
            )
        })
        .unwrap_or_default();
    println!(
        "     {}  {}{}  {}",
        colors::symbol("↑"),
        output::escape_controls(&row.label),
        destination,
        colors::status_label(row.status_text, "↑"),
    );
}

fn print_name_section(separator: &mut output::SectionSeparator, title: &str, names: &[String]) {
    if names.is_empty() {
        return;
    }

    separator.begin();
    println!("{} {}", colors::cyan("==>"), colors::bold(title));
    output::print_columns(names);
}

pub(super) fn app_status_summary_parts(
    up_to_date: usize,
    update_available: usize,
    refresh_available: usize,
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
    output::push_count(
        &mut parts,
        refresh_available,
        colors::cyan,
        "refresh available",
    );
    output::push_count(&mut parts, user_modified, colors::yellow, "user-modified");
    output::push_count(&mut parts, missing, colors::yellow, "destination missing");
    parts
}

fn refresh_command_hint(command: &str) -> String {
    if command.chars().any(char::is_control) {
        "shine app refresh with the original category and source arguments (control characters; copyable command omitted)".to_string()
    } else {
        format!("`{command}`")
    }
}

fn app_status_run_hint(row: &AppLifecycleStatus) -> String {
    let mut commands = Vec::new();
    if row.upgrade_available {
        commands.push(format!(
            "run `shine upgrade {}`",
            crate::shell_quote::command_arg(&format!("app/{}", row.category))
        ));
    }
    commands.extend(row.refresh_sources.iter().map(|source| {
        let command = format!(
            "shine app refresh {}",
            crate::shell_quote::command_positionals(&[&row.category, source])
        );
        format!("run {}", refresh_command_hint(&command))
    }));
    if commands.is_empty() {
        String::new()
    } else {
        format!("  {}", colors::dim(&commands.join("; ")))
    }
}

pub(super) async fn update(
    config: &Config,
    inputs: UpdateInputs,
    diff: bool,
    run_generators: bool,
) -> Result<bool> {
    let UpdateInputs {
        shell_rows,
        shell_lifecycle,
        app_rows,
        app_lifecycle,
        app_inspections,
        update_sys,
        missing_sources,
    } = inputs;
    let UpdateView {
        mut update_shell,
        update_app,
        actionable_app,
        refresh_commands,
        shell_attention,
        any_update,
    } = UpdateView::new(
        &shell_rows,
        &shell_lifecycle,
        &app_rows,
        &app_lifecycle,
        !update_sys.is_empty(),
    );
    let attention = GeneratorAttention::from_rows(&app_rows);
    if !any_update
        && !attention.has_attention()
        && shell_attention.is_empty()
        && missing_sources.is_empty()
    {
        print_app_analysis(&app_inspections, diff);
        return Ok(false);
    }

    crate::config::print_presets_note(config);
    print_shell_attention(&shell_attention);
    print_missing_app_sources(&missing_sources);
    print_app_analysis(&app_inspections, diff);

    if !diff {
        let shell_names = shell_categories(&update_shell);
        let app_names = actionable_app
            .keys()
            .map(|category| (*category).to_string())
            .collect::<Vec<_>>();
        let sys_names = sorted_names(update_sys.iter().map(|row| row.item_id.clone()).collect());

        let mut separator = output::SectionSeparator::new();
        print_name_section(&mut separator, "Shell Presets", &shell_names);
        print_name_section(&mut separator, "App Configs", &app_names);
        print_name_section(&mut separator, "System Configs", &sys_names);
        print_generator_notice(&attention, run_generators);
        let upgrade_app_names = update_app
            .keys()
            .map(|category| (*category).to_string())
            .collect::<Vec<_>>();
        print_action_hints(
            &update_targets(&shell_names, &upgrade_app_names, &sys_names),
            &refresh_commands,
        );
        if attention.has_failures() {
            anyhow::bail!("one or more App generators could not be evaluated");
        }
        return Ok(true);
    }

    let update_diffs = query::collect_diffs(config, app_inspections).await?;
    let mut shell_diff_state = crate::info::ShellDiffState::default();

    if !update_shell.is_empty() {
        println!("{}", colors::bold("Shell Presets"));

        let label_width = update_shell
            .iter()
            .map(|r| console::measure_text_width(&output::escape_controls(&r.label)))
            .max()
            .unwrap_or(0);

        update_shell.sort_by(|left, right| left.category.cmp(&right.category));
        for rows in update_shell.chunk_by(|left, right| left.category == right.category) {
            for row in rows {
                let pad = " ".repeat(label_width.saturating_sub(console::measure_text_width(
                    &output::escape_controls(&row.label),
                )));
                println!(
                    "  {}  {}{}  {}",
                    row.symbol,
                    output::escape_controls(&row.label),
                    pad,
                    colors::status_label(row.status_text, row.status_sym),
                );
                update_diffs
                    .print_shell_for_row(config, &row.selector(), &mut shell_diff_state)
                    .await?;
            }
            shell_diff_state.print_snapshot(&rows[0].category);
        }
    }

    if !actionable_app.is_empty() {
        if !update_shell.is_empty() {
            println!();
        }
        println!("{}", colors::bold("App Configs"));

        let label_width = actionable_app
            .keys()
            .map(|category| console::measure_text_width(category))
            .max()
            .unwrap_or(0);

        for (category, rows) in &actionable_app {
            let pad = " ".repeat(label_width.saturating_sub(console::measure_text_width(category)));
            let (status_text, status_sym) = app_category_action_status(rows);
            println!(
                "  {}  {}{}  {}",
                colors::symbol("↑"),
                category,
                pad,
                colors::status_label(status_text, status_sym),
            );
            for row in rows {
                print_app_update_detail(row);
                update_diffs.print_app_for_row(config, &row.label).await?;
            }
        }
    }

    if !update_sys.is_empty() {
        if !update_shell.is_empty() || !actionable_app.is_empty() {
            println!();
        }
        println!("{}", colors::bold("System Configs"));
        for row in &update_sys {
            println!(
                "  {}  {}  {}  {}",
                colors::symbol("↑"),
                output::escape_controls(&row.label),
                colors::dim(&format!("({})", output::escape_controls(&row.item_id))),
                colors::status_label("update available", "↑"),
            );
            for detail in &row.details {
                println!("     {}", colors::dim(&output::escape_controls(detail)));
            }
        }
    }

    print_generator_notice(&attention, run_generators);
    if any_update {
        let shell_names = shell_categories(&update_shell);
        let app_names = update_app
            .keys()
            .map(|category| (*category).to_string())
            .collect::<Vec<_>>();
        let sys_names = sorted_names(update_sys.iter().map(|row| row.item_id.clone()).collect());
        print_action_hints(
            &update_targets(&shell_names, &app_names, &sys_names),
            &refresh_commands,
        );
    }
    if attention.has_failures() {
        anyhow::bail!("one or more App generators could not be evaluated");
    }

    Ok(true)
}
pub(super) fn status_shell_attention(shell_rows: &[ShellRow]) {
    let installed_shell: Vec<&ShellRow> = shell_rows.iter().filter(|r| r.is_installed).collect();
    print_shell_attention(
        &installed_shell
            .iter()
            .copied()
            .filter(|row| row.link_conflict || row.preset_missing)
            .collect::<Vec<_>>(),
    );
}
pub(super) async fn status(
    config: &Config,
    inputs: StatusInputs,
    diff: bool,
    run_generators: bool,
) -> Result<()> {
    let StatusInputs {
        shell_rows,
        app_rows,
        app_inspections,
        sys,
        missing_sources,
    } = inputs;
    let update_sys = &sys.updates;
    let installed_shell: Vec<&ShellRow> = shell_rows.iter().filter(|r| r.is_installed).collect();
    let all_shell: Vec<&ShellRow> = shell_rows.iter().collect();
    let installed_app: Vec<&AppRow> = app_rows
        .iter()
        .filter(|r| r.file_status != FileStatus::NotInstalled)
        .collect();
    let all_app: Vec<&AppRow> = app_rows.iter().collect();
    let any = !installed_shell.is_empty()
        || !installed_app.is_empty()
        || !sys.installed.is_empty()
        || !missing_sources.is_empty();

    if !any {
        println!(
            "{}",
            colors::dim("Nothing installed yet. Run `shine shell install` or `shine app install`.")
        );
        return Ok(());
    }

    print_app_analysis(&app_inspections, true);
    let update_diffs = if diff {
        Some(query::collect_diffs(config, app_inspections).await?)
    } else {
        None
    };
    let mut shell_diff_state = crate::info::ShellDiffState::default();
    let mut shell_statuses: Vec<_> = if diff {
        installed_shell
            .iter()
            .map(|row| ShellLifecycleStatus::from(*row))
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
                upgrade_available: row.upgrade_available,
                refresh_sources: row.refresh_sources.clone(),
            })
            .collect()
    } else {
        app_category_statuses(&all_app)
    };

    print_missing_app_sources(&missing_sources);

    // ── Shell Presets ────────────────────────────────────────────────────────
    if !installed_shell.is_empty() {
        println!("{}", colors::bold("Shell Presets"));

        let label_width = if diff {
            installed_shell
                .iter()
                .map(|row| console::measure_text_width(&output::escape_controls(&row.label)))
                .max()
        } else {
            shell_statuses
                .iter()
                .map(|row| console::measure_text_width(&output::escape_controls(&row.category)))
                .max()
        }
        .unwrap_or(0);

        // Inspection appends receipt-only commands after active metadata. Keep those siblings
        // with their category so its shared diff cannot split the detailed status rows.
        shell_statuses.sort_by(|left, right| left.category.cmp(&right.category));
        for rows in shell_statuses.chunk_by(|left, right| left.category == right.category) {
            for row in rows {
                let label = if diff {
                    &row.detail_label
                } else {
                    &row.category
                };
                let label = output::escape_controls(label);
                let pad =
                    " ".repeat(label_width.saturating_sub(console::measure_text_width(&label)));
                let run_hint = if row.status == ShellRowStatus::UpdateAvail {
                    format!("  {}", colors::dim("run `shine upgrade`"))
                } else {
                    String::new()
                };
                println!(
                    "  {}  {}{}  {}{}",
                    colors::symbol(row.status.symbol()),
                    label,
                    pad,
                    colors::status_label(row.status_text, row.status.symbol()),
                    run_hint,
                );
                if diff
                    && row.status == ShellRowStatus::UpdateAvail
                    && let Some(diffs) = &update_diffs
                {
                    diffs
                        .print_shell_for_row(config, &row.selector, &mut shell_diff_state)
                        .await?;
                }
            }
            shell_diff_state.print_snapshot(&rows[0].category);
        }
    }

    // ── App Configs ──────────────────────────────────────────────────────────
    if !installed_app.is_empty() {
        if !installed_shell.is_empty() {
            println!();
        }
        println!("{}", colors::bold("App Configs"));

        let label_width = if diff {
            installed_app
                .iter()
                .map(|row| console::measure_text_width(&output::escape_controls(&row.label)))
                .max()
        } else {
            app_statuses
                .iter()
                .map(|row| console::measure_text_width(&output::escape_controls(&row.category)))
                .max()
        }
        .unwrap_or(0);

        let mut up_to_date = 0usize;
        let mut update_available = 0usize;
        let mut refresh_available = 0usize;
        let mut user_modified = 0usize;
        let mut missing = 0usize;

        for row in &app_statuses {
            let label = if diff {
                &row.detail_label
            } else {
                &row.category
            };
            let label = output::escape_controls(label);
            let pad = " ".repeat(label_width.saturating_sub(console::measure_text_width(&label)));
            let dest_part = if diff {
                row.dest
                    .as_deref()
                    .map(|d| {
                        format!(
                            "  {}  {}",
                            colors::dim("→"),
                            colors::dim(&output::escape_controls(d))
                        )
                    })
                    .unwrap_or_default()
            } else {
                String::new()
            };

            let run_hint = app_status_run_hint(row);

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
                FileStatus::UpdateAvail => {
                    if row.upgrade_available {
                        update_available += 1;
                    }
                    if !row.refresh_sources.is_empty() {
                        refresh_available += 1;
                    }
                    if !row.upgrade_available && row.refresh_sources.is_empty() {
                        update_available += 1;
                    }
                }
                FileStatus::GeneratorNotEvaluated
                | FileStatus::GeneratorEvaluationFailed
                | FileStatus::GeneratorTrustRequired => {}
                FileStatus::UpToDate => up_to_date += 1,
                FileStatus::NotInstalled => {}
            }
        }

        let mut parts = app_status_summary_parts(
            up_to_date,
            update_available,
            refresh_available,
            user_modified,
            missing,
        );
        for (status, label) in [
            (FileStatus::GeneratorNotEvaluated, "generator not evaluated"),
            (
                FileStatus::GeneratorEvaluationFailed,
                "generator evaluation failed",
            ),
            (
                FileStatus::GeneratorTrustRequired,
                "generator trust required",
            ),
        ] {
            let count = app_statuses
                .iter()
                .filter(|row| row.file_status == status)
                .count();
            let unit = match (diff, count == 1) {
                (false, true) => "category",
                (false, false) => "categories",
                (true, true) => "file",
                (true, false) => "files",
            };
            output::push_count(
                &mut parts,
                count,
                colors::yellow,
                &format!("{unit}: {label}"),
            );
        }
        if !parts.is_empty() {
            output::footer("Summary", &parts);
        }
    }

    if !sys.installed.is_empty() {
        if !installed_shell.is_empty() || !installed_app.is_empty() {
            println!();
        }
        println!("{}", colors::bold("System Configs"));
        for row in update_sys {
            println!(
                "  {}  {}  {}  {}  {}",
                colors::symbol("↑"),
                output::escape_controls(&row.label),
                colors::dim(&format!("({})", output::escape_controls(&row.item_id))),
                colors::status_label("update available", "↑"),
                colors::dim("run `shine upgrade`"),
            );
            for detail in &row.details {
                println!("     {}", colors::dim(&output::escape_controls(detail)));
            }
        }
        for row in &sys.installed {
            if update_sys
                .iter()
                .any(|update| update.item_id == row.item_id)
            {
                continue;
            }
            let target = format!("sys/{}", row.item_id);
            let state = sys
                .report
                .items
                .iter()
                .find(|item| item.target == target)
                .map(|item| item.state);
            let (symbol, status) = match state {
                Some(shine_core::frontend::InspectionStateV1::Current) => {
                    ("✓", "up-to-date (recorded)")
                }
                Some(shine_core::frontend::InspectionStateV1::PresetMissing) => {
                    ("!", "preset missing; recorded entry preserved")
                }
                _ => ("~", "recorded"),
            };
            println!(
                "  {}  {}  {}  {}",
                colors::symbol(symbol),
                output::escape_controls(&row.label),
                colors::dim(&format!("({})", output::escape_controls(&row.item_id))),
                colors::status_label(status, symbol)
            );
        }
    }

    let attention = GeneratorAttention::from_rows(&app_rows);
    print_generator_notice(&attention, run_generators);
    if attention.has_failures() {
        anyhow::bail!("one or more App generators could not be evaluated");
    }

    Ok(())
}
pub(super) fn inventory(inputs: InventoryInputs) -> Result<()> {
    let InventoryInputs {
        installed_shell,
        installed_app,
        installed_sys,
        missing_sources,
    } = inputs;
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
    if !missing_sources.is_empty() {
        separator.begin();
        print_missing_app_sources(&missing_sources);
    }

    Ok(())
}
