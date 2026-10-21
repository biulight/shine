mod projection;
mod query;
mod render;
use projection::*;
#[cfg(test)]
use render::app_status_summary_parts;

use crate::apps::load_active_categories;
use crate::colors;
use crate::config::Config;
use crate::info::UpdateDiffs;
use crate::output;
use crate::status::{
    AppRow, FileStatus, ShellRow, ShellRowStatus, build_app_rows_with_lifecycle_options,
    build_shell_rows,
};
use crate::sys;
use anyhow::{Context, Result};
use shine_core::frontend::{
    CapabilityKindV1, FrontendServiceError, InventoryReportV1, InventoryRequest,
};
use std::collections::{BTreeMap, BTreeSet};

pub async fn handle_update_list(config: &Config, diff: bool, run_generators: bool) -> Result<bool> {
    let inputs = query::collect_update(config, run_generators).await?;
    render::update(config, inputs, diff, run_generators).await
}

pub async fn handle_status_list(config: &Config, diff: bool, run_generators: bool) -> Result<()> {
    crate::config::print_presets_note(config);
    let shell_rows = build_shell_rows(config).await?;
    render::status_shell_attention(&shell_rows);
    let inputs = query::collect_status(config, shell_rows, run_generators).await?;
    render::status(config, inputs, diff, run_generators).await
}

pub async fn handle_list(config: &Config) -> Result<()> {
    crate::config::print_presets_note(config);
    let inputs = query::collect_inventory(config).await?;
    render::inventory(inputs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell_row(status_text: &'static str, is_installed: bool) -> ShellRow {
        ShellRow {
            category: "proxy".to_string(),
            command_name: "setproxy".to_string(),
            file_status: FileStatus::Partial,
            symbol: String::new(),
            label: "proxy/setproxy".to_string(),
            status_sym: "~",
            status_text,
            is_installed,
            link_conflict: false,
            preset_missing: false,
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
            upgrade_available: file_status == FileStatus::UpdateAvail,
            refresh_sources: Vec::new(),
        }
    }

    #[test]
    fn shell_attention_reports_conflicts_and_missing_presets_without_upgrade_hint() {
        let mut conflict = shell_row("launcher ownership conflict", true);
        conflict.link_conflict = true;
        let mut missing = shell_row("preset missing; installed entry preserved", true);
        missing.preset_missing = true;
        let current = shell_row("up-to-date", true);
        let lines = shell_attention_lines(&[&conflict, &missing, &current]).join("\n");
        assert!(lines.contains("launcher ownership conflict"));
        assert!(lines.contains("preset missing"));
        assert!(lines.contains("shine shell uninstall"));
        assert!(!lines.contains("shine upgrade"));
        assert!(!lines.contains("up-to-date"));
        assert!(shell_attention_lines(&[&current]).is_empty());
    }

    #[tokio::test]
    async fn shell_rows_keep_identity_and_partial_status_when_display_fields_change() {
        use shine_core::runtime::{CoreRuntime, PresetSnapshot, PresetSourceKind};
        let runtime = crate::test_support::installed_shell_runtime().await;
        let mut rows = runtime
            .inspect_shells()
            .await
            .unwrap()
            .into_iter()
            .map(ShellRow::from)
            .collect::<Vec<_>>();
        let run = rows
            .iter_mut()
            .find(|row| row.command_name == "run")
            .unwrap();
        run.label = "friendly label/with/slashes".into();
        run.status_sym = "↑"; // Presentation must not manufacture an applicable update.
        let statuses = shell_category_statuses(&rows.iter().collect::<Vec<_>>());
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].status, ShellRowStatus::Partial);
        assert_eq!(statuses[0].status_text, "partial install");

        let mut snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded);
        for (logical, bytes) in runtime.presets().files() {
            snapshot = snapshot.file(
                logical,
                if logical == "shell/demo/run.ts" {
                    b"console.log('changed');\n".to_vec()
                } else {
                    bytes.clone()
                },
            );
        }
        let updated = CoreRuntime::new(
            runtime.host().clone(),
            runtime.context().clone(),
            snapshot.build(),
        );
        let mut rows = updated
            .inspect_shells()
            .await
            .unwrap()
            .into_iter()
            .map(ShellRow::from)
            .collect::<Vec<_>>();
        let run = rows
            .iter_mut()
            .find(|row| row.command_name == "run")
            .unwrap();
        assert_eq!(run.status(), ShellRowStatus::UpdateAvail);
        run.label = "friendly label/with/slashes".into();
        run.status_sym = "!"; // Display changes cannot hide the stable pending target either.
        let detail = ShellLifecycleStatus::from(&*run);
        assert_eq!(detail.detail_label, "friendly label/with/slashes");
        assert_eq!(detail.selector, "demo/run");
        assert_eq!(detail.status, ShellRowStatus::UpdateAvail);

        let pending = pending_shell_rows(&rows, &BTreeSet::from(["shell/demo/run"]));
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].target(), "shell/demo/run");
        assert_eq!(pending[0].selector(), "demo/run");
        assert_eq!(
            shell_category_statuses(&rows.iter().collect::<Vec<_>>())[0].status,
            ShellRowStatus::UpdateAvail
        );
    }

    #[tokio::test]
    async fn missing_preset_guidance_uses_receipt_identity_instead_of_display_label() {
        use shine_core::runtime::{CoreRuntime, PresetSnapshot, PresetSourceKind};
        let runtime = crate::test_support::installed_shell_runtime().await;
        let missing = CoreRuntime::new(
            runtime.host().clone(),
            runtime.context().clone(),
            PresetSnapshot::builder(PresetSourceKind::Embedded).build(),
        );
        let mut rows = missing
            .inspect_shells()
            .await
            .unwrap()
            .into_iter()
            .map(ShellRow::from)
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].preset_missing && rows[0].is_installed);
        rows[0].label = "friendly label".into();
        let attention = shell_attention_lines(&[&rows[0]]).join("\n");
        assert!(attention.contains("! shell/friendly label"));
        assert!(attention.contains("shine shell uninstall demo/run"));
        assert!(!attention.contains("uninstall friendly label"));
        assert!(!attention.contains("shine upgrade"));
        assert_eq!(
            shell_category_statuses(&[&rows[0]])[0].status,
            ShellRowStatus::Attention
        );
    }

    #[test]
    fn typed_shell_priority_keeps_attention_order_and_last_row_ties() {
        let mut current = shell_row("current", true);
        current.file_status = FileStatus::UpToDate;
        let mut pending = shell_row("pending", true);
        pending.file_status = FileStatus::UpdateAvail;
        let partial = shell_row("partial", true);
        let mut modified = shell_row("modified", true);
        modified.file_status = FileStatus::UserModified;
        let mut missing = shell_row("missing", true);
        missing.file_status = FileStatus::Missing;
        let mut conflict = shell_row("conflict", true);
        conflict.link_conflict = true;
        for (rows, status, text) in [
            (
                vec![&current, &pending],
                ShellRowStatus::UpdateAvail,
                "pending",
            ),
            (vec![&pending, &partial], ShellRowStatus::Partial, "partial"),
            (
                vec![&partial, &modified],
                ShellRowStatus::Partial,
                "modified",
            ),
            (
                vec![&modified, &partial],
                ShellRowStatus::Partial,
                "partial",
            ),
            (
                vec![&partial, &missing],
                ShellRowStatus::Attention,
                "missing",
            ),
            (
                vec![&missing, &conflict],
                ShellRowStatus::Attention,
                "conflict",
            ),
        ] {
            let grouped = shell_category_statuses(&rows);
            assert_eq!(grouped[0].status, status);
            assert_eq!(grouped[0].status_text, text);
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
    fn manual_generator_updates_render_refresh_commands_instead_of_upgrade_targets() {
        use clap::Parser;
        let mut row = app_row("surge", FileStatus::UpdateAvail);
        row.upgrade_available = false;
        row.refresh_sources = vec!["subscription-proxies.conf".to_string()];
        let grouped = app_update_categories(&[&row]);

        assert_eq!(
            app_refresh_commands(&grouped),
            ["shine app refresh surge subscription-proxies.conf"]
        );
        assert_eq!(
            app_category_action_status(&grouped["surge"]),
            ("refresh available", "↑")
        );
        for (category, source) in [("-surge", "-file.conf"), ("surge", "-file.conf")] {
            let mut row = app_row(category, FileStatus::UpdateAvail);
            row.refresh_sources = vec![source.into()];
            let commands = app_refresh_commands(&app_update_categories(&[&row]));
            let parsed =
                crate::commands::Cli::try_parse_from(commands[0].split_whitespace()).unwrap();
            let crate::commands::Commands::App {
                command:
                    crate::commands::AppCommands::Refresh {
                        category: actual,
                        file,
                        ..
                    },
            } = parsed.command
            else {
                panic!("expected refresh command")
            };
            assert_eq!(actual, category);
            assert_eq!(file.as_deref(), Some(source));
        }
    }

    #[test]
    fn mixed_app_updates_retain_both_upgrade_and_refresh_actions() {
        let automatic = app_row("sample", FileStatus::UpdateAvail);
        let mut manual = app_row("sample", FileStatus::UpdateAvail);
        manual.upgrade_available = false;
        manual.refresh_sources = vec!["generated.conf".to_string()];
        let grouped = app_update_categories(&[&automatic, &manual]);

        assert_eq!(
            app_category_action_status(&grouped["sample"]),
            ("update and refresh available", "↑")
        );
        assert_eq!(
            app_refresh_commands(&grouped),
            ["shine app refresh sample generated.conf"]
        );
    }

    #[test]
    fn update_hint_targets_the_only_pending_category() {
        let targets = update_targets(&[], &["clash-verge".to_string()], &[]);

        assert_eq!(targets, ["app/clash-verge"]);
        assert_eq!(
            update_hint_text(&targets),
            "Run `shine upgrade app/clash-verge` to apply updates."
        );
    }

    #[test]
    fn update_hint_keeps_global_upgrade_for_multiple_categories() {
        let targets = update_targets(
            &["proxy".to_string()],
            &["clash-verge".to_string()],
            &["split-dns".to_string()],
        );

        assert_eq!(targets, ["shell/proxy", "app/clash-verge", "sys/split-dns"]);
        assert_eq!(
            update_hint_text(&targets),
            "Run `shine upgrade` to apply updates."
        );
    }

    #[test]
    fn default_shell_status_collapses_commands_and_reports_partial_install() {
        let mut installed = shell_row("up-to-date", true);
        installed.status_sym = "✓";
        installed.file_status = FileStatus::UpToDate;
        let mut missing = shell_row("not installed", false);
        missing.label = "proxy/usetproxy".to_string();
        missing.status_sym = "✗";
        missing.file_status = FileStatus::NotInstalled;
        missing.command_name = "usetproxy".into();

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

    #[tokio::test]
    async fn simple_list_shell_visibility_uses_production_inventory() {
        use shine_core::runtime::{
            CoreRuntime, FileSystemHost, FileSystemObservationHost, HostOperation, PresetSnapshot,
            PresetSourceKind, command_path_for_name,
        };
        for (state, expected) in [
            ("current", true),
            ("update", true),
            ("foreign-launcher", true),
            ("receipt-without-launcher", true),
            ("legacy-launcher-without-receipt", true),
            ("uninstalled", false),
            ("missing-preset", false),
        ] {
            let installed = crate::test_support::installed_shell_runtime().await;
            let manifest_path = installed.context().shine_dir.join("shell-manifest.toml");
            let launcher = command_path_for_name(&installed.context().bin_dir, "run".as_ref());
            let mut snapshot = installed.presets().clone();
            match state {
                "update" => {
                    let mut builder = PresetSnapshot::builder(PresetSourceKind::Embedded);
                    for (path, bytes) in snapshot.files() {
                        builder = builder.file(
                            path,
                            if path == "shell/demo/run.ts" {
                                b"console.log('new');\n".to_vec()
                            } else {
                                bytes.clone()
                            },
                        );
                    }
                    snapshot = builder.build();
                }
                "foreign-launcher" => installed.host().put_file(&launcher, b"foreign".to_vec()),
                "receipt-without-launcher" | "uninstalled" => {
                    installed.host().remove_file(&launcher).await.unwrap();
                }
                "missing-preset" => {
                    snapshot = PresetSnapshot::builder(PresetSourceKind::Embedded).build()
                }
                _ => {}
            }
            if matches!(state, "uninstalled" | "legacy-launcher-without-receipt") {
                installed.host().put_file(
                    &manifest_path,
                    b"schema_version = 1\nentries = []\n".to_vec(),
                );
            }
            let manifest = installed.host().read(&manifest_path).await.unwrap();
            let launcher_before = installed.host().read(&launcher).await.ok();
            let start = installed.host().operations().len();
            let service = shine_core::frontend::FrontendService::new(CoreRuntime::new(
                installed.host().clone(),
                installed.context().clone(),
                snapshot,
            ));
            let report = service
                .inventory(InventoryRequest::for_kind(CapabilityKindV1::Shell))
                .await
                .unwrap();
            let run = report
                .items
                .iter()
                .find(|item| item.target == "shell/demo/run")
                .unwrap_or_else(|| {
                    panic!("{state}: missing receipt or available item: {report:?}")
                });
            assert_eq!(run.available, state != "missing-preset", "{state}: {run:?}");
            assert_eq!(run.installed, state != "uninstalled", "{state}: {run:?}");
            assert_eq!(
                installed_shell_categories_from_inventory(&report),
                if expected {
                    vec!["demo".to_string()]
                } else {
                    Vec::new()
                },
                "{state}: {report:?}"
            );
            // An available, uninstalled sibling cannot create another category entry.
            if state != "missing-preset" {
                let idle = report
                    .items
                    .iter()
                    .find(|item| item.target == "shell/demo/idle")
                    .unwrap();
                assert!(idle.available && !idle.installed, "{state}: {idle:?}");
            }
            assert!(
                installed.host().operations()[start..]
                    .iter()
                    .all(|op| matches!(op, HostOperation::Read(_)))
            );
            assert_eq!(
                installed.host().read(&manifest_path).await.unwrap(),
                manifest
            );
            assert_eq!(installed.host().read(&launcher).await.ok(), launcher_before);
        }
    }

    #[test]
    fn frontend_inventory_projection_preserves_legacy_list_visibility() {
        let report = InventoryReportV1 {
            schema_version: shine_core::frontend::INVENTORY_REPORT_SCHEMA_VERSION,
            items: vec![
                shine_core::frontend::CapabilityInventoryItemV1 {
                    target: "app/installed".to_string(),
                    kind: CapabilityKindV1::App,
                    available: true,
                    installed: true,
                },
                shine_core::frontend::CapabilityInventoryItemV1 {
                    target: "app/orphan".to_string(),
                    kind: CapabilityKindV1::App,
                    available: false,
                    installed: true,
                },
                shine_core::frontend::CapabilityInventoryItemV1 {
                    target: "shell/tools/one".to_string(),
                    kind: CapabilityKindV1::Shell,
                    available: true,
                    installed: true,
                },
                shine_core::frontend::CapabilityInventoryItemV1 {
                    target: "shell/tools/two".to_string(),
                    kind: CapabilityKindV1::Shell,
                    available: true,
                    installed: true,
                },
                shine_core::frontend::CapabilityInventoryItemV1 {
                    target: "shell/orphan/tool".to_string(),
                    kind: CapabilityKindV1::Shell,
                    available: false,
                    installed: true,
                },
                shine_core::frontend::CapabilityInventoryItemV1 {
                    target: "sys/managed".to_string(),
                    kind: CapabilityKindV1::Sys,
                    available: false,
                    installed: true,
                },
            ],
            diagnostics: Vec::new(),
        };

        assert_eq!(
            installed_app_categories_from_inventory(&report),
            ["installed", "orphan"]
        );
        assert_eq!(
            installed_shell_categories_from_inventory(&report),
            ["tools"]
        );
        assert_eq!(installed_sys_items_from_inventory(&report), ["managed"]);
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
            app_status_summary_parts(3, 1, 0, 0, 0),
            vec!["3 up-to-date".to_string(), "1 update available".to_string()]
        );
    }

    #[test]
    fn app_status_summary_parts_empty_when_all_zero() {
        assert!(app_status_summary_parts(0, 0, 0, 0, 0).is_empty());
    }

    #[test]
    fn app_status_summary_parts_reports_all_five_counters() {
        assert_eq!(
            app_status_summary_parts(1, 2, 3, 4, 5),
            vec![
                "1 up-to-date".to_string(),
                "2 update available".to_string(),
                "3 refresh available".to_string(),
                "4 user-modified".to_string(),
                "5 destination missing".to_string(),
            ]
        );
    }

    #[test]
    fn update_projection_keeps_pending_ownership_and_manual_refresh_actions_separate() {
        use shine_core::lifecycle::{
            LifecycleOperation, LifecycleOutcomeV1, LifecycleResultV1, LifecycleStatus,
        };
        let mut pending_shell = shell_row("update available", true);
        pending_shell.command_name = "pending".into();
        let mut conflict = shell_row("launcher conflict", true);
        conflict.link_conflict = true;
        let shell = vec![pending_shell, conflict];
        let mut shell_lifecycle = LifecycleResultV1::new(LifecycleOperation::Update, false);
        shell_lifecycle.push(LifecycleOutcomeV1::new(
            shell[0].target(),
            None::<String>,
            LifecycleStatus::Pending,
            [],
        ));
        let mut manual = app_row("demo", FileStatus::UpdateAvail);
        manual.upgrade_available = false;
        manual.refresh_sources = vec!["manual.txt".into()];
        let app = vec![
            manual,
            app_row("demo", FileStatus::UpdateAvail),
            app_row("excluded", FileStatus::UpdateAvail),
        ];
        let mut app_lifecycle = LifecycleResultV1::new(LifecycleOperation::Update, false);
        app_lifecycle.push(LifecycleOutcomeV1::new(
            "app/demo",
            None::<String>,
            LifecycleStatus::Pending,
            [],
        ));
        let view = UpdateView::new(&shell, &shell_lifecycle, &app, &app_lifecycle, false);
        assert_eq!(view.update_shell.len(), 1);
        assert_eq!(view.update_shell[0].command_name, "pending");
        assert_eq!(view.shell_attention.len(), 1);
        assert_eq!(view.actionable_app["demo"].len(), 2);
        assert_eq!(view.update_app["demo"].len(), 1);
        assert!(!view.actionable_app.contains_key("excluded"));
        assert_eq!(view.refresh_commands, ["shine app refresh demo manual.txt"]);
        assert!(view.any_update);
    }
}
