use crate::apps::{load_embedded_categories, load_installed_categories};
use crate::check::{AppRow, FileStatus, ShellRow, build_app_rows, build_shell_rows};
use crate::colors;
use crate::config::Config;
use anyhow::Result;

pub(crate) async fn handle_status_list(config: &Config) -> Result<()> {
    crate::config::print_presets_note(config);
    let shell_rows = build_shell_rows(config).await?;
    let installed_shell: Vec<&ShellRow> = shell_rows.iter().filter(|r| r.is_installed).collect();

    let cats_result = if config.is_external_presets {
        load_installed_categories(config, None).await
    } else {
        load_embedded_categories(None)
    };
    let app_rows = match cats_result {
        Ok(cats) => build_app_rows(config, &cats).await?,
        Err(_) => Vec::new(),
    };
    let installed_app: Vec<&AppRow> = app_rows
        .iter()
        .filter(|r| r.file_status != FileStatus::NotInstalled)
        .collect();

    let any = !installed_shell.is_empty() || !installed_app.is_empty();

    if !any {
        println!(
            "{}",
            colors::dim("Nothing installed yet. Run `shine shell install` or `shine app install`.")
        );
        return Ok(());
    }

    // ── Shell Presets ────────────────────────────────────────────────────────
    if !installed_shell.is_empty() {
        println!("{}", colors::bold("Shell Presets"));

        let label_width = installed_shell
            .iter()
            .map(|r| r.label.len())
            .max()
            .unwrap_or(0);

        for row in &installed_shell {
            let pad = " ".repeat(label_width.saturating_sub(row.label.len()));
            println!(
                "  {}  {}{}  {}",
                row.symbol,
                row.label,
                pad,
                colors::status_label(row.status_text, row.status_sym),
            );
        }
    }

    // ── App Configs ──────────────────────────────────────────────────────────
    if !installed_app.is_empty() {
        if !installed_shell.is_empty() {
            println!();
        }
        println!("{}", colors::bold("App Configs"));

        let label_width = installed_app
            .iter()
            .map(|r| r.label.len())
            .max()
            .unwrap_or(0);

        let mut up_to_date = 0usize;
        let mut update_available = 0usize;
        let mut user_modified = 0usize;
        let mut missing = 0usize;

        for row in &installed_app {
            let pad = " ".repeat(label_width.saturating_sub(row.label.len()));
            let dest_part = row
                .dest
                .as_deref()
                .map(|d| format!("  {}  {}", colors::dim("→"), colors::dim(d)))
                .unwrap_or_default();

            let run_hint = if row.sym == "↑" {
                format!("  {}", colors::dim("run `shine upgrade`"))
            } else {
                String::new()
            };

            println!(
                "  {}  {}{}{}  {}{}",
                colors::symbol(row.sym),
                row.label,
                pad,
                dest_part,
                colors::status_label(row.status_text, row.sym),
                run_hint,
            );

            match row.file_status {
                FileStatus::Missing => missing += 1,
                FileStatus::UserModified | FileStatus::Partial => user_modified += 1,
                FileStatus::UpdateAvail => update_available += 1,
                FileStatus::UpToDate => up_to_date += 1,
                FileStatus::NotInstalled => {}
            }
        }

        // Summary footer
        let mut parts: Vec<String> = Vec::new();
        if up_to_date > 0 {
            parts.push(colors::green(&format!("{up_to_date} up-to-date")));
        }
        if update_available > 0 {
            parts.push(colors::cyan(&format!(
                "{update_available} update available"
            )));
        }
        if user_modified > 0 {
            parts.push(colors::yellow(&format!("{user_modified} user-modified")));
        }
        if missing > 0 {
            parts.push(colors::yellow(&format!("{missing} destination missing")));
        }

        if !parts.is_empty() {
            let sep = colors::dim(" · ");
            println!("\n{}  {}", colors::bold("Summary"), parts.join(&sep));
        }
    }

    Ok(())
}

pub(crate) async fn handle_list(config: &Config) -> Result<()> {
    crate::config::print_presets_note(config);
    let shell_rows = build_shell_rows(config).await?;
    let installed_shell: Vec<&ShellRow> = shell_rows.iter().filter(|r| r.is_installed).collect();

    let cats_result = if config.is_external_presets {
        load_installed_categories(config, None).await
    } else {
        load_embedded_categories(None)
    };
    let app_rows = match cats_result {
        Ok(cats) => build_app_rows(config, &cats).await?,
        Err(_) => Vec::new(),
    };
    let installed_app: Vec<&AppRow> = app_rows
        .iter()
        .filter(|r| r.file_status != FileStatus::NotInstalled)
        .collect();

    let any = !installed_shell.is_empty() || !installed_app.is_empty();

    if !any {
        println!(
            "{}",
            colors::dim("Nothing installed yet. Run `shine shell install` or `shine app install`.")
        );
        return Ok(());
    }

    if !installed_shell.is_empty() {
        println!("{}", colors::bold("Shell Presets"));
        for row in &installed_shell {
            println!("  {}", row.label);
        }
    }

    if !installed_app.is_empty() {
        if !installed_shell.is_empty() {
            println!();
        }
        println!("{}", colors::bold("App Configs"));
        for row in &installed_app {
            match row.dest.as_deref() {
                Some(dest) => println!(
                    "  {}  {}  {}",
                    row.label,
                    colors::dim("→"),
                    colors::dim(dest)
                ),
                None => println!("  {}", row.label),
            }
        }
    }

    Ok(())
}
