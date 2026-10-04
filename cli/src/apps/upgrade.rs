use anyhow::{Result, anyhow};
use std::collections::{BTreeMap, BTreeSet};

use crate::config::Config;
use crate::env::EnvConfig;
use crate::presentation::{
    LifecycleReporter, PresentationEvent, TerminalInteraction, TerminalRenderer,
};
use shine_core::lifecycle::LifecycleOperation;
use shine_core::lifecycle::{LifecycleResultV1, LifecycleStatus};
use shine_core::runtime::{
    AppFileAction, AppPlanRequest, PlanningInputVersions, RuntimeEvent, RuntimeObserver,
};

use super::report;

#[derive(Debug, Default)]
pub struct AppUpgradeReport {
    /// Physical files changed, retained for diagnostics and tests.
    pub updated: usize,
    /// User-facing app targets changed. Default summaries count this value.
    pub updated_categories: usize,
    pub skipped: usize,
    pub failed: usize,
    pub user_modified: usize,
    pub restart_hints: BTreeSet<String>,
}

pub async fn handle_upgrade_installed(
    config: &Config,
    prune_stale: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<AppUpgradeReport> {
    handle_upgrade_installed_with_output(config, prune_stale, false, sep).await
}

pub(crate) async fn handle_upgrade_installed_with_output(
    config: &Config,
    prune_stale: bool,
    verbose: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<AppUpgradeReport> {
    handle_upgrade_installed_with_output_with_result_approved(
        config,
        prune_stale,
        verbose,
        true,
        sep,
    )
    .await
    .map(|(report, _)| report)
}

pub(crate) async fn handle_upgrade_installed_with_output_with_result_approved(
    config: &Config,
    prune_stale: bool,
    verbose: bool,
    yes: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<(AppUpgradeReport, LifecycleResultV1)> {
    handle_upgrade_installed_target_with_result_approved(
        config,
        None,
        prune_stale,
        verbose,
        yes,
        sep,
    )
    .await
}

pub(crate) async fn handle_upgrade_installed_with_output_with_result_prepared(
    config: &Config,
    prune_stale: bool,
    verbose: bool,
    prepared: crate::lifecycle_plan::PreparedLifecyclePlan,
    sep: &mut crate::output::SectionSeparator,
) -> Result<(AppUpgradeReport, LifecycleResultV1)> {
    let mut renderer = TerminalRenderer::stdio_with_separator(sep);
    handle_upgrade_installed_target_with_prepared_reporter(
        config,
        None,
        prune_stale,
        verbose,
        prepared,
        &mut renderer,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn handle_upgrade_installed_target_with_result(
    config: &Config,
    category_filter: Option<&str>,
    prune_stale: bool,
    verbose: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<(AppUpgradeReport, LifecycleResultV1)> {
    handle_upgrade_installed_target_with_result_approved(
        config,
        category_filter,
        prune_stale,
        verbose,
        true,
        sep,
    )
    .await
}

pub(crate) async fn handle_upgrade_installed_target_with_result_approved(
    config: &Config,
    category_filter: Option<&str>,
    prune_stale: bool,
    verbose: bool,
    yes: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<(AppUpgradeReport, LifecycleResultV1)> {
    let mut renderer = TerminalRenderer::stdio_with_separator(sep);
    handle_upgrade_installed_target_with_reporter(
        config,
        category_filter,
        prune_stale,
        verbose,
        yes,
        &mut renderer,
    )
    .await
}

async fn handle_upgrade_installed_target_with_reporter(
    config: &Config,
    category_filter: Option<&str>,
    prune_stale: bool,
    verbose: bool,
    yes: bool,
    reporter: &mut dyn LifecycleReporter,
) -> Result<(AppUpgradeReport, LifecycleResultV1)> {
    let reviewed = crate::lifecycle_plan::review_upgrade_plans(
        config,
        [crate::lifecycle_plan::LifecyclePlanRequest::app(
            AppPlanRequest {
                operation: LifecycleOperation::Upgrade,
                target: category_filter.map(str::to_string),
                force: false,
                purge: false,
                prune_stale,
                input_versions: PlanningInputVersions::default(),
            },
            config,
        )],
        yes,
        verbose,
    )
    .await?
    .into_iter()
    .next()
    .expect("one reviewed App Plan");
    let runtime = crate::lifecycle_plan::prepare_runtime(config, &reviewed).await?;
    handle_upgrade_installed_target_with_prepared_reporter(
        config,
        category_filter,
        prune_stale,
        verbose,
        crate::lifecycle_plan::PreparedLifecyclePlan { reviewed, runtime },
        reporter,
    )
    .await
}

async fn handle_upgrade_installed_target_with_prepared_reporter(
    config: &Config,
    _category_filter: Option<&str>,
    _prune_stale: bool,
    verbose: bool,
    prepared: crate::lifecycle_plan::PreparedLifecyclePlan,
    reporter: &mut dyn LifecycleReporter,
) -> Result<(AppUpgradeReport, LifecycleResultV1)> {
    let crate::lifecycle_plan::PreparedLifecyclePlan {
        reviewed,
        mut runtime,
    } = prepared;
    let env = EnvConfig::load_or_init(config).await?;
    runtime.context_mut_for_cli().env = env.as_map().clone();
    let mut observer = UpgradeObserver::default();
    let mut interaction = TerminalInteraction;
    let artifact_categories = runtime
        .app_categories(None)?
        .into_iter()
        .filter(|category| category.artifact.is_some())
        .map(|category| category.name)
        .collect::<BTreeSet<_>>();
    let core = match crate::lifecycle_plan::execute_reviewed(
        config,
        runtime,
        reviewed,
        shine_core::frontend::ExecutionOptions {
            show_hook_success: verbose,
        },
        &mut observer,
        &mut interaction,
    )
    .await?
    {
        shine_core::frontend::OperationDetails::AppUpgrade(report) => *report,
        _ => unreachable!("reviewed operation result type"),
    };

    let show_unchanged = verbose && crate::presentation::full_upgrade_plan();
    let mut started = false;
    let begin = |reporter: &mut dyn LifecycleReporter, started: &mut bool| {
        if !*started {
            reporter.emit(PresentationEvent::SectionStart);
            reporter.emit(PresentationEvent::stdout(report::upgrade_header_text(
                show_unchanged,
                core.files.len(),
            )));
            *started = true;
        }
    };
    if show_unchanged && !core.files.is_empty() {
        begin(reporter, &mut started);
    }

    let mut updated_files = BTreeMap::<String, usize>::new();
    for file in &core.files {
        let source = format!("app/{}/{}", file.category, file.source.display());
        match file.action {
            AppFileAction::Installed | AppFileAction::BackedUp => {
                *updated_files.entry(file.category.clone()).or_default() += 1;
                if verbose {
                    begin(reporter, &mut started);
                    reporter.emit(PresentationEvent::stdout(report::install_success_text(
                        &source,
                        "",
                        &file.destination,
                        config,
                    )));
                }
            }
            AppFileAction::Removed | AppFileAction::Restored | AppFileAction::Missing => {
                *updated_files.entry(file.category.clone()).or_default() += 1;
                begin(reporter, &mut started);
                reporter.emit(PresentationEvent::stdout(report::stale_removed_text(
                    config,
                    &file.destination,
                    if file.action == AppFileAction::Missing {
                        "(stale managed file already missing)"
                    } else {
                        "(removed stale managed file)"
                    },
                )));
            }
            AppFileAction::Unchanged if show_unchanged => {
                begin(reporter, &mut started);
                reporter.emit(PresentationEvent::stdout(report::up_to_date_text(&source)));
            }
            AppFileAction::UserModified => {
                begin(reporter, &mut started);
                reporter.emit(PresentationEvent::stderr(report::warning_text(
                    &source,
                    "user-modified, skipped",
                )));
            }
            AppFileAction::GeneratorPreserved | AppFileAction::Failed => {
                begin(reporter, &mut started);
                let detail = file
                    .generator_error
                    .as_ref()
                    .or(file.error.as_ref())
                    .cloned()
                    .unwrap_or_else(|| "upgrade failed".to_string());
                reporter.emit(PresentationEvent::stderr(report::install_error_text(
                    &source,
                    &anyhow!(detail),
                )));
            }
            _ => {}
        }
    }
    if !verbose && !updated_files.is_empty() {
        begin(reporter, &mut started);
        for (category, count) in &updated_files {
            reporter.emit(PresentationEvent::stdout(report::category_updated_text(
                category, *count,
            )));
        }
    }
    let changed_categories = core
        .files
        .iter()
        .filter(|file| {
            matches!(
                file.action,
                AppFileAction::Installed
                    | AppFileAction::BackedUp
                    | AppFileAction::Removed
                    | AppFileAction::Restored
            )
        })
        .map(|file| file.category.clone())
        .collect::<BTreeSet<_>>();
    for category in report::artifact_apply_categories(&artifact_categories, changed_categories) {
        begin(reporter, &mut started);
        reporter.emit(PresentationEvent::stdout(report::artifact_apply_hint_text(
            &category,
        )));
    }
    for event in observer.events {
        if matches!(
            &event,
            RuntimeEvent::Warning { .. }
                | RuntimeEvent::Progress {
                    code: "app_hook_completed",
                    ..
                }
                | RuntimeEvent::ProcessOutput { .. }
        ) {
            begin(reporter, &mut started);
            render_runtime_event(reporter, event);
        }
    }

    let updated = core
        .files
        .iter()
        .filter(|file| file.status == LifecycleStatus::Changed)
        .count();
    let result = AppUpgradeReport {
        updated,
        updated_categories: core.updated_categories.len(),
        skipped: core.skipped,
        failed: core.failed,
        user_modified: core.user_modified,
        restart_hints: core.restart_hints,
    };
    Ok((result, core.lifecycle))
}

#[derive(Default)]
struct UpgradeObserver {
    events: Vec<RuntimeEvent>,
}

impl RuntimeObserver for UpgradeObserver {
    fn emit(&mut self, event: RuntimeEvent) {
        self.events.push(event);
    }
}

fn render_runtime_event(reporter: &mut dyn LifecycleReporter, event: RuntimeEvent) {
    match event {
        RuntimeEvent::Warning { target, detail, .. } => reporter.emit(PresentationEvent::stderr(
            report::warning_text(target.as_deref().unwrap_or("app"), detail),
        )),
        RuntimeEvent::Progress {
            code: "app_hook_completed",
            target,
        } => {
            reporter.emit(PresentationEvent::stdout(format!(
                "  {} {}: post-upgrade hook completed",
                report::symbol("✓"),
                target.trim_start_matches("app/")
            )));
        }
        RuntimeEvent::ProcessOutput { text, .. } => {
            for line in text.lines() {
                reporter.emit(PresentationEvent::stdout(format!(
                    "     {}",
                    report::dim(line)
                )));
            }
        }
        _ => {}
    }
}

#[cfg(all(test, unix))]
mod tests {
    // HOME must remain stable across the asynchronous lifecycle calls.
    #![allow(clippy::await_holding_lock)]

    use super::*;
    use crate::test_support::env_lock;
    use tokio::fs;

    #[derive(Default)]
    struct RecordingReporter(Vec<PresentationEvent>);

    impl LifecycleReporter for RecordingReporter {
        fn emit(&mut self, event: PresentationEvent) {
            self.0.push(event);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn verbose_upgrade_reports_changed_app_files_but_not_current_files() {
        let _guard = env_lock();
        let dir = crate::test_support::make_temp_dir("shine-app-upgrade-report").await;
        let previous_home = std::env::var_os("HOME");
        // SAFETY: `_guard` serializes HOME changes across tests in this process.
        unsafe { std::env::set_var("HOME", &dir) };
        let category = dir.join("presets/app/sample");
        fs::create_dir_all(&category).await.unwrap();
        fs::write(
            category.join("shine.toml"),
            "description = 'Sample app'\ndest = '~/.config/sample'\n\n[permissions]\nschema_version = 1\n\n[[files]]\nsource = 'config.txt'\ntarget = 'config.txt'\n",
        )
        .await
        .unwrap();
        fs::write(category.join("config.txt"), b"old\n")
            .await
            .unwrap();
        let mut config = Config::new_for_test(&dir);
        config.is_external_presets = true;
        fs::create_dir_all(config.shine_dir()).await.unwrap();
        super::super::install::handle_install(&config, Some("sample"), false, false)
            .await
            .unwrap();

        let mut current = RecordingReporter::default();
        handle_upgrade_installed_target_with_reporter(
            &config,
            Some("sample"),
            false,
            true,
            true,
            &mut current,
        )
        .await
        .unwrap();
        assert!(current.0.is_empty());

        crate::presentation::with_full_upgrade_plan(true, async {
            let mut full = RecordingReporter::default();
            handle_upgrade_installed_target_with_reporter(
                &config,
                Some("sample"),
                false,
                true,
                true,
                &mut full,
            )
            .await
            .unwrap();
            assert!(full.0.iter().any(|event| matches!(
                event,
                PresentationEvent::Line { text, .. } if text.contains("up to date")
            )));
        })
        .await;

        fs::write(category.join("config.txt"), b"new\n")
            .await
            .unwrap();
        let mut changed = RecordingReporter::default();
        handle_upgrade_installed_target_with_reporter(
            &config,
            Some("sample"),
            false,
            true,
            true,
            &mut changed,
        )
        .await
        .unwrap();
        assert!(changed.0.iter().any(|event| matches!(
            event,
            PresentationEvent::Line { text, .. } if text.contains("config.txt")
        )));

        match previous_home {
            Some(home) => unsafe { std::env::set_var("HOME", home) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        fs::remove_dir_all(dir).await.unwrap();
    }
}
