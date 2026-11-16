use anyhow::Result;

use crate::{colors, config::Config};

use super::detect::detect_os_id;

pub async fn handle_profile_enable_approved(
    config: &Config,
    item_id: &str,
    dry_run: bool,
    yes: bool,
) -> Result<()> {
    handle_profile_state(config, item_id, true, dry_run, yes).await
}

pub async fn handle_profile_disable_approved(
    config: &Config,
    item_id: &str,
    dry_run: bool,
    yes: bool,
) -> Result<()> {
    handle_profile_state(config, item_id, false, dry_run, yes).await
}

async fn handle_profile_state(
    config: &Config,
    item_id: &str,
    enabled: bool,
    dry_run: bool,
    yes: bool,
) -> Result<()> {
    crate::config::print_presets_note(config);
    let os_id = detect_os_id().await?;
    let sys_shell: &'static str = config.shell_type.into();
    let plan_request = shine_core::runtime::SysProfilePlanRequest {
        os_id: os_id.clone(),
        item_id: item_id.to_string(),
        enabled,
    };
    let (runtime, reviewed) = if dry_run {
        (crate::core_runtime::from_config(config).await?, None)
    } else {
        let reviewed = crate::lifecycle_plan::review_plans(
            config,
            [crate::lifecycle_plan::LifecyclePlanRequest::sys_profile(
                plan_request.clone(),
            )],
            yes,
        )
        .await?
        .into_iter()
        .next()
        .expect("one reviewed Sys profile Plan");
        let runtime = crate::lifecycle_plan::prepare_runtime(config, &reviewed).await?;
        (runtime, Some(reviewed))
    };
    let report = if let Some(reviewed) = reviewed {
        match crate::lifecycle_plan::execute_reviewed(
            config,
            runtime,
            reviewed,
            shine_core::frontend::ExecutionOptions::default(),
            &mut shine_core::runtime::NullObserver,
            &mut crate::presentation::TerminalInteraction,
        )
        .await?
        {
            shine_core::frontend::OperationDetails::SysProfile(report) => *report,
            _ => unreachable!("reviewed operation result type"),
        }
    } else {
        runtime
            .preview_sys_profile(shine_core::runtime::SysProfileStateRequest {
                os_id,
                item_id: item_id.to_string(),
                enabled,
                dry_run,
            })
            .await?
    };
    let action = if enabled { "enable" } else { "disable" };
    if dry_run {
        println!(
            "{} sys/{item_id}: would {action} shell integration for {sys_shell}",
            colors::dim("[dry-run]")
        );
        return Ok(());
    }
    let outcome = report
        .outcome
        .expect("Core profile mutation returns an outcome");
    println!("{}", profile_outcome_text(item_id, enabled, &outcome));
    Ok(())
}

fn profile_outcome_text(item_id: &str, enabled: bool, outcome: &super::SysItemOutcome) -> String {
    use super::SysItemStatus;
    let state = match outcome.status {
        SysItemStatus::NeedsAction => "needs action",
        SysItemStatus::Failed => "failed",
        SysItemStatus::Skipped | SysItemStatus::AlreadyInstalled => {
            if enabled {
                "already enabled"
            } else {
                "already disabled"
            }
        }
        SysItemStatus::Installed | SysItemStatus::Updated | SysItemStatus::Completed => {
            if enabled {
                "enabled"
            } else {
                "disabled"
            }
        }
    };
    let symbol = super::execution::status_symbol(outcome.status);
    format!(
        "{} sys/{item_id}: shell integration {} ({})",
        colors::symbol(symbol),
        colors::status_label(state, symbol),
        outcome.detail
    )
}

/// Independent profile refresh: never enters bootstrap selection or detection.
pub async fn handle_profile_init(
    config: &Config,
    dry_run: bool,
    force_profile: bool,
    yes: bool,
) -> Result<()> {
    crate::config::print_presets_note(config);
    let os_id = detect_os_id().await?;
    let shell: &'static str = config.shell_type.into();
    let request = shine_core::runtime::SysInitPlanRequest {
        os_id: os_id.clone(),
        force_profile,
    };
    let runtime = crate::core_runtime::from_config(config).await?;
    let plan = runtime.plan_sys_init(request.clone()).await?;
    let ext = if os_id == "windows" { "ps1" } else { "sh" };
    println!("Sys initialization target: {os_id}/{shell}");
    for phase in ["pre", "post"] {
        let path = format!("sys/{os_id}/profile/base.{phase}.{ext}");
        println!(
            "  {path}: {}",
            if runtime.presets().get(&path).is_some() {
                "present"
            } else {
                "absent (empty base)"
            }
        );
    }
    let enabled = plan
        .steps
        .iter()
        .filter(|step| step.resource.as_deref() == Some("preserved-integration"))
        .map(|step| crate::output::escape_controls(&step.target))
        .collect::<Vec<_>>();
    println!(
        "Enabled integrations: {}",
        if enabled.is_empty() {
            "none".to_string()
        } else {
            enabled.join(", ")
        }
    );
    let has_base = ["pre", "post"].iter().any(|phase| {
        runtime
            .presets()
            .get(&format!("sys/{os_id}/profile/base.{phase}.{ext}"))
            .is_some_and(|bytes| !bytes.is_empty())
    });
    if !has_base && enabled.is_empty() {
        println!("No initialization content; coordinating empty profiles and loading blocks.");
    }
    if matches!(os_id.as_str(), "ubuntu" | "arch") {
        let other = if shell == "bash" {
            "~/.zshrc"
        } else {
            "~/.bashrc"
        };
        println!("Remove this OS's Shine Sys loading blocks from {other}, if present.");
    }
    println!(
        "An interrupted refresh may leave some files updated. Review and rerun sys init to finish; sys recover cannot undo this refresh."
    );
    if dry_run {
        crate::lifecycle_plan::print_sys_init_preview(&plan)?;
        println!("[dry-run] No initialization code executed or profiles changed.");
        return Ok(());
    }
    let reviewed = crate::lifecycle_plan::review_plans(
        config,
        [crate::lifecycle_plan::LifecyclePlanRequest::SysInit(
            request,
        )],
        yes,
    )
    .await?
    .into_iter()
    .next()
    .expect("one Sys init Plan");
    let runtime = crate::lifecycle_plan::prepare_runtime(config, &reviewed).await?;
    let result = crate::lifecycle_plan::execute_reviewed(
        config,
        runtime,
        reviewed,
        shine_core::frontend::ExecutionOptions::default(),
        &mut shine_core::runtime::NullObserver,
        &mut crate::presentation::TerminalInteraction,
    )
    .await?;
    let shine_core::frontend::OperationDetails::SysInit(outcome) = result else {
        unreachable!("Sys init result")
    };
    let state = match outcome.status {
        super::SysItemStatus::NeedsAction => "needs action (some files may already have changed)",
        super::SysItemStatus::Failed => "failed",
        super::SysItemStatus::Skipped | super::SysItemStatus::AlreadyInstalled => {
            "already up to date"
        }
        _ => "updated",
    };
    println!(
        "Sys initialization {state}: {}",
        crate::output::escape_controls(&outcome.detail)
    );
    if !matches!(
        outcome.status,
        super::SysItemStatus::NeedsAction | super::SysItemStatus::Failed
    ) {
        println!(
            "Open a new {shell} session to load initialization; the current terminal was not reloaded."
        );
    }
    Ok(())
}
