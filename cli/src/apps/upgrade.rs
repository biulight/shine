use anyhow::{Result, anyhow};
use std::collections::{BTreeMap, BTreeSet};
use std::io::IsTerminal;

use crate::config::Config;
use crate::env::EnvConfig;
use crate::presentation::{
    LifecycleReporter, PresentationEvent, TerminalInteraction, TerminalRenderer,
};
use utils::lifecycle::{LifecycleResultV1, LifecycleStatus};
use utils::runtime::{AppFileAction, AppUpgradeRequest, RuntimeEvent, RuntimeObserver};

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
    handle_upgrade_installed_with_output_with_result(config, prune_stale, verbose, sep)
        .await
        .map(|(report, _)| report)
}

pub(crate) async fn handle_upgrade_installed_with_output_with_result(
    config: &Config,
    prune_stale: bool,
    verbose: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<(AppUpgradeReport, LifecycleResultV1)> {
    handle_upgrade_installed_target_with_result(config, None, prune_stale, verbose, sep).await
}

pub(crate) async fn handle_upgrade_installed_target_with_result(
    config: &Config,
    category_filter: Option<&str>,
    prune_stale: bool,
    verbose: bool,
    sep: &mut crate::output::SectionSeparator,
) -> Result<(AppUpgradeReport, LifecycleResultV1)> {
    let mut renderer = TerminalRenderer::stdio_with_separator(sep);
    handle_upgrade_installed_target_with_reporter(
        config,
        category_filter,
        prune_stale,
        verbose,
        &mut renderer,
    )
    .await
}

async fn handle_upgrade_installed_target_with_reporter(
    config: &Config,
    category_filter: Option<&str>,
    prune_stale: bool,
    verbose: bool,
    reporter: &mut dyn LifecycleReporter,
) -> Result<(AppUpgradeReport, LifecycleResultV1)> {
    let mut runtime = crate::core_runtime::from_config(config).await?;
    let env = EnvConfig::load_or_init(config).await?;
    runtime.context_mut_for_cli().env = env.as_map().clone();
    let prompt_stale = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let mut observer = UpgradeObserver::default();
    let mut interaction = TerminalInteraction;
    let core = runtime
        .upgrade_apps(
            AppUpgradeRequest {
                category: category_filter.map(str::to_string),
                prune_stale,
                prompt_stale,
                show_hook_success: verbose,
            },
            &mut observer,
            &mut interaction,
        )
        .await?;

    let mut started = false;
    let begin = |reporter: &mut dyn LifecycleReporter, started: &mut bool| {
        if !*started {
            reporter.emit(PresentationEvent::SectionStart);
            reporter.emit(PresentationEvent::stdout(report::upgrade_header_text(
                verbose,
                core.files.len(),
            )));
            *started = true;
        }
    };
    if verbose && !core.files.is_empty() {
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
            AppFileAction::Unchanged if verbose => {
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
    for event in observer.events {
        begin(reporter, &mut started);
        render_runtime_event(reporter, event);
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
