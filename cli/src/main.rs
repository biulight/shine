use anyhow::{Context, Result, bail};
use clap::Parser;
use cli::commands::{Cli, Commands, CompletionShell};
use cli::completion;

fn main() -> std::process::ExitCode {
    match run_main() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            if !error.is::<cli::output::ReportedCommandFailure>() {
                eprintln!("Error: {error:?}");
            }
            std::process::ExitCode::FAILURE
        }
    }
}

fn run_main() -> Result<()> {
    completion::complete_from_env();

    let argv = std::env::args_os().collect::<Vec<_>>();
    let cli = Cli::parse_from(&argv);
    if let Commands::ShellLaunch { target, .. } = &cli.command
        && !argv
            .windows(2)
            .any(|pair| pair[0] == target.as_str() && pair[1] == "--")
    {
        bail!("__shell-launch requires -- immediately after the target");
    }

    if let Commands::Completions { command } = &cli.command
        && CompletionShell::from_command(command).is_some()
    {
        command.generate();
        return Ok(());
    }

    if let Some(config_dir) = &cli.config_dir {
        if config_dir.trim().is_empty() {
            bail!("--config-dir is required when using --config-dir")
        }
        // SAFETY: called before the Tokio runtime starts; no other threads exist at
        // this point, so the write cannot race concurrent env reads.
        unsafe { std::env::set_var("SHINE_CONFIG_DIR", config_dir) }
    }

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build async runtime")?
        .block_on(cli::dispatch::run(cli))
}
