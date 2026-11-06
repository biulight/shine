//! Configuration and release checks, explicit generator evaluation and independent error policies.

use crate::config::Config;
use crate::generator_review::GeneratorCheckFailure;
use crate::update_check::UpdateStatus;
use crate::{colors, info, list, update_check, version};
use anyhow::{Result, bail};

pub async fn handle_update(
    config: &Config,
    target: Option<&str>,
    diff: bool,
    verbose: bool,
    refresh_release: bool,
    run_generators: bool,
) -> Result<()> {
    let compatibility = crate::preset_migration::active_compatibility_plan(target).await?;
    let migration_required = crate::preset_migration::compatibility_required(&compatibility);
    crate::preset_migration::print_compatibility(&compatibility);

    if let Some(target) = target {
        info::handle_update_target_with_verbosity(config, target, run_generators, verbose).await?;
        if migration_required {
            bail!(
                "{}",
                crate::preset_migration::compatibility_failure_message(&compatibility)
            );
        }
        return Ok(());
    }

    let generator_overview = verbose && run_generators && !diff;
    let config_updates = if verbose {
        match Box::pin(list::handle_status_list(config, diff, run_generators)).await {
            Ok(()) => {
                if !generator_overview {
                    println!();
                }
                Ok(true)
            }
            Err(error) => Err(error),
        }
    } else {
        Box::pin(list::handle_update_list(config, diff, run_generators)).await
    };
    let (mut printed_update, config_update_error) = match config_updates {
        Ok(printed) => (printed, None),
        Err(error)
            if error.is::<crate::output::ReportedCommandFailure>()
                || error.is::<GeneratorCheckFailure>() =>
        {
            (false, Some(error))
        }
        Err(error) => {
            eprintln!(
                "{}",
                colors::yellow_stderr(&format!(
                    "warning: configuration update check failed: {error:#}"
                ))
            );
            (false, Some(error))
        }
    };

    let current = version::semver();
    if generator_overview {
        println!("\n{}", colors::bold("Shine version"));
        println!("  Installed version: {current}");
    } else if verbose {
        println!("Checking for updates (current: {current})...");
    }

    let update_status = if refresh_release {
        update_check::check_for_update_forced(config).await
    } else {
        update_check::check_for_update(config).await
    };

    let release_check_completed = update_status.is_ok();
    match update_status {
        Ok(UpdateStatus::UpToDate) => {
            if verbose {
                println!(
                    "{}",
                    colors::green(&format!("shine {current} is up to date."))
                );
            }
        }
        Ok(UpdateStatus::UpdateAvailable { latest }) => {
            if printed_update && !verbose {
                println!();
            }
            println!(
                "{}",
                colors::yellow(&format!(
                    "A newer version of shine is available: {current} -> {latest}."
                ))
            );
            println!("Run `shine self upgrade` to install it.");
            printed_update = true;
        }
        Ok(UpdateStatus::UpdateRequired { latest }) => {
            if printed_update && !verbose {
                println!();
            }
            println!(
                "{}",
                colors::yellow(&format!(
                    "A newer patch release of shine is available: {current} -> {latest}."
                ))
            );
            println!("Run `shine self upgrade` to install it.");
            printed_update = true;
        }
        Err(e) => {
            if generator_overview {
                let reason = update_check::rate_limit_http_status(&e)
                    .map(|status| {
                        format!("GitHub API rate limit reached (HTTP {})", status.as_u16())
                    })
                    .unwrap_or_else(|| crate::output::escape_controls(&e.to_string()));
                eprintln!("  {}  Check skipped · {reason}", colors::yellow_stderr("!"));
            } else {
                eprintln!("{}", format_update_check_failure_warning(&e));
            }
        }
    }

    if !printed_update && config_update_error.is_none() && !migration_required {
        println!(
            "{}",
            colors::dim(if release_check_completed {
                "Nothing to update."
            } else {
                "No configuration updates found; Shine version check incomplete."
            })
        );
    }

    if let Some(error) = config_update_error {
        if error.is::<GeneratorCheckFailure>() {
            eprintln!("\n{}  {error}.", colors::bold_stderr("Result"));
            return Err(crate::output::ReportedCommandFailure.into());
        }
        return Err(error);
    }
    if migration_required {
        bail!(
            "{}",
            crate::preset_migration::compatibility_failure_message(&compatibility)
        );
    }

    Ok(())
}

fn format_update_check_failure_warning(err: &anyhow::Error) -> String {
    colors::yellow_stderr(&format!("warning: skipped shine version check: {err}"))
}

#[cfg(test)]
mod tests;
