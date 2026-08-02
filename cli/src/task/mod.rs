//! `shine task` — a lightweight personal shortcut-command registry.
//!
//! Tasks are user runtime state (stored in `<shine_dir>/tasks.toml`, see
//! [`manifest`]), not embedded presets. Commands are saved as an argv array and
//! executed directly with no shell (`std::process::Command`), inheriting the
//! caller's stdio and environment. A task may store an optional fixed working
//! directory; otherwise it runs in the caller's current directory. Users who
//! need shell syntax (pipes, redirects, globbing) save an explicit
//! `sh -c '...'` invocation.
//!
//! `shine run <name>` is a top-level alias for `shine task run <name>`; it has
//! no independent semantics or storage.
//!
//! Platform note: direct execution runs any real executable on every platform,
//! but the `sh -c '...'` escape hatch is Unix-only (Windows has no `sh`).

pub mod manifest;

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::{colors, path_display};
use manifest::TaskManifest;

const NOT_FOUND_HINT: &str = "Run `shine task list` to see saved tasks.";

pub async fn handle_save(
    config: &Config,
    name: &str,
    force: bool,
    cwd: Option<&Path>,
    command: Vec<String>,
) -> Result<()> {
    validate_task_name(name)?;
    if command.is_empty() {
        bail!(
            "No command provided.\n\nUsage:\n  shine task save <name> [--cwd <dir>] -- <command...>"
        );
    }

    let mut manifest = TaskManifest::load(config.shine_dir()).await?;
    if !force && manifest.get(name).is_some() {
        bail!("Task already exists: {name}\n\nUse `--force` to replace it.");
    }

    let cwd = resolve_task_cwd(config, cwd)?;
    let rendered = render_command(&command);
    manifest.upsert(name, command, cwd.clone());
    manifest.save(config.shine_dir()).await?;

    println!("{}", colors::green(&format!("Saved task {name}")));
    println!("{rendered}");
    if let Some(cwd) = cwd {
        println!(
            "Working dir: {}",
            path_display::format_home(&cwd, &config.home_dir)
        );
    }
    Ok(())
}

pub async fn handle_run(config: &Config, name: &str, extra: &[String]) -> Result<()> {
    let manifest = TaskManifest::load(config.shine_dir()).await?;
    let Some(entry) = manifest.get(name) else {
        bail!("Task not found: {name}\n\n{NOT_FOUND_HINT}");
    };

    let mut argv = entry.command.clone();
    argv.extend_from_slice(extra);
    if let Some(cwd) = entry.cwd.as_deref() {
        validate_run_cwd(name, cwd)?;
    }

    // Announce on stderr so a task's own stdout stays clean for piping.
    let cwd_note = entry.cwd.as_deref().map_or_else(String::new, |cwd| {
        format!(
            " (cwd: {})",
            path_display::format_home(cwd, &config.home_dir)
        )
    });
    eprintln!(
        "{}{cwd_note}: {}",
        colors::bold(&format!("Running {name}")),
        render_command(&argv)
    );

    run_task_command(name, &argv, entry.cwd.as_deref())
}

pub async fn handle_list(config: &Config) -> Result<()> {
    let manifest = TaskManifest::load(config.shine_dir()).await?;
    if manifest.tasks.is_empty() {
        println!("No saved tasks yet. Run `shine task save <name> -- <command...>`.");
        return Ok(());
    }

    println!("{}", colors::bold("Saved Tasks"));
    let width = manifest.tasks.keys().map(|k| k.len()).max().unwrap_or(0);
    for (name, entry) in &manifest.tasks {
        println!(
            "{name:<width$}  {}",
            render_command(&entry.command),
            width = width
        );
    }
    Ok(())
}

pub async fn handle_info(config: &Config, name: &str) -> Result<()> {
    let manifest = TaskManifest::load(config.shine_dir()).await?;
    let Some(entry) = manifest.get(name) else {
        bail!("Task not found: {name}\n\n{NOT_FOUND_HINT}");
    };

    println!("{:<10} {}", "Task", name);
    println!("{:<10} {}", "Command", render_command(&entry.command));
    if let Some(cwd) = entry.cwd.as_deref() {
        println!(
            "{:<10} {}",
            "Working dir",
            path_display::format_home(cwd, &config.home_dir)
        );
    }
    Ok(())
}

pub async fn handle_delete(config: &Config, name: &str) -> Result<()> {
    let mut manifest = TaskManifest::load(config.shine_dir()).await?;
    if !manifest.remove(name) {
        bail!("Task not found: {name}\n\n{NOT_FOUND_HINT}");
    }
    manifest.save(config.shine_dir()).await?;
    println!("{}", colors::green(&format!("Deleted task {name}")));
    Ok(())
}

/// Execute a saved task's argv directly, inheriting stdio and environment, and
/// propagate the child's exit code verbatim (never wrapping it in an anyhow
/// error) so the task's own exit semantics survive `shine` in between.
fn run_task_command(name: &str, argv: &[String], cwd: Option<&Path>) -> Result<()> {
    let Some((program, args)) = argv.split_first() else {
        bail!("Task {name} has no command to run.");
    };

    let mut command = std::process::Command::new(program);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }

    let status = match command.status() {
        Ok(status) => status,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(cwd) = cwd
                && !cwd.is_dir()
            {
                bail!(
                    "Failed to run task {name}: working directory is unavailable: {}",
                    path_display::format(cwd)
                );
            }
            bail!("Failed to run task {name}: command not found: {program}");
        }
        Err(e) => bail!("Failed to run task {name}: {program}: {e}"),
    };

    if status.success() {
        return Ok(());
    }
    if let Some(code) = status.code() {
        std::process::exit(code);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        std::process::exit(128 + status.signal().unwrap_or(1));
    }
    #[cfg(not(unix))]
    std::process::exit(1);
}

fn resolve_task_cwd(config: &Config, cwd: Option<&Path>) -> Result<Option<PathBuf>> {
    let Some(cwd) = cwd else {
        return Ok(None);
    };

    let raw = cwd.to_string_lossy();
    let home = config.home_dir.to_string_lossy().into_owned();
    let expanded = shellexpand::tilde_with_context(&raw, || Some(home)).into_owned();
    let expanded = PathBuf::from(expanded);
    let absolute = if expanded.is_absolute() {
        expanded
    } else {
        std::env::current_dir()
            .context("resolving current directory for task cwd")?
            .join(expanded)
    };
    let canonical = match std::fs::canonicalize(&absolute) {
        Ok(canonical) => canonical,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => bail!(
            "Failed to save task: working directory does not exist: {}",
            path_display::format(&absolute)
        ),
        Err(error) => bail!(
            "Failed to save task: cannot resolve working directory: {}: {error}",
            path_display::format(&absolute)
        ),
    };
    if !canonical.is_dir() {
        bail!(
            "Failed to save task: working directory is not a directory: {}",
            path_display::format(&canonical)
        );
    }
    Ok(Some(canonical))
}

fn validate_run_cwd(name: &str, cwd: &Path) -> Result<()> {
    match std::fs::metadata(cwd) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => bail!(
            "Failed to run task {name}: working directory is not a directory: {}",
            path_display::format(cwd)
        ),
        Err(error) => bail!(
            "Failed to run task {name}: working directory is unavailable: {}: {error}",
            path_display::format(cwd)
        ),
    }
}

/// Render an argv array back into a copy-paste-safe shell command line by
/// single-quoting any argument that contains characters a shell would interpret.
fn render_command(argv: &[String]) -> String {
    argv.iter()
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(arg: &str) -> String {
    crate::shell_quote::quote_if_needed(arg)
}

fn validate_task_name(name: &str) -> Result<()> {
    let invalid = || {
        anyhow::anyhow!(
            "Invalid task name: {name}\n\nTask names may contain letters, numbers, dots, dashes, and underscores, and must start with a letter or number."
        )
    };

    let Some(first) = name.chars().next() else {
        return Err(invalid());
    };
    if !first.is_ascii_alphanumeric() {
        return Err(invalid());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_in(dir: &std::path::Path) -> Config {
        crate::test_support::test_config(dir)
    }

    async fn temp_dir() -> std::path::PathBuf {
        crate::test_support::make_temp_dir("shine-task-test").await
    }

    #[test]
    fn validate_task_name_accepts_allowed_charset() {
        assert!(validate_task_name("deploy-keystone").is_ok());
        assert!(validate_task_name("a.b-c_1").is_ok());
        assert!(validate_task_name("3000").is_ok());
    }

    #[test]
    fn validate_task_name_rejects_spaces_and_bad_start() {
        assert!(validate_task_name("deploy docs").is_err());
        assert!(validate_task_name(".hidden").is_err());
        assert!(validate_task_name("-lead").is_err());
        assert!(validate_task_name("").is_err());
    }

    #[test]
    fn render_command_leaves_plain_argv_unquoted() {
        let argv = [
            "rsync".to_string(),
            "-avz".to_string(),
            "dist/".to_string(),
            "marqueeio.develop:/var/www/keystone/alex/".to_string(),
        ];
        assert_eq!(
            render_command(&argv),
            "rsync -avz dist/ marqueeio.develop:/var/www/keystone/alex/"
        );
    }

    #[test]
    fn render_command_quotes_shell_syntax_arguments() {
        let argv = [
            "sh".to_string(),
            "-c".to_string(),
            "lsof -ti :3000 | xargs kill".to_string(),
        ];
        assert_eq!(render_command(&argv), "sh -c 'lsof -ti :3000 | xargs kill'");
    }

    #[test]
    fn shell_quote_escapes_embedded_single_quotes() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[tokio::test]
    async fn save_then_info_and_list_show_command() {
        let dir = temp_dir().await;
        let config = config_in(&dir);

        handle_save(
            &config,
            "port-3000",
            false,
            None,
            vec!["lsof".to_string(), "-i".to_string(), ":3000".to_string()],
        )
        .await
        .unwrap();

        let manifest = TaskManifest::load(config.shine_dir()).await.unwrap();
        assert_eq!(
            manifest.get("port-3000").unwrap().command,
            ["lsof", "-i", ":3000"]
        );

        // list/info should not error with a saved task present.
        handle_list(&config).await.unwrap();
        handle_info(&config, "port-3000").await.unwrap();

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_resolves_and_persists_fixed_cwd() {
        let dir = temp_dir().await;
        let config = config_in(&dir);
        let project = config.home_dir.join("project");
        tokio::fs::create_dir_all(&project).await.unwrap();

        handle_save(
            &config,
            "build",
            false,
            Some(Path::new("~/project")),
            vec!["cargo".to_string(), "build".to_string()],
        )
        .await
        .unwrap();

        let manifest = TaskManifest::load(config.shine_dir()).await.unwrap();
        assert_eq!(
            manifest.get("build").unwrap().cwd.as_ref(),
            Some(&std::fs::canonicalize(&project).unwrap())
        );
        handle_info(&config, "build").await.unwrap();
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_resolves_relative_cwd_from_current_directory() {
        let dir = temp_dir().await;
        let config = config_in(&dir);
        let expected = std::fs::canonicalize(".").unwrap();

        handle_save(
            &config,
            "here",
            false,
            Some(Path::new(".")),
            vec!["echo".to_string()],
        )
        .await
        .unwrap();

        let manifest = TaskManifest::load(config.shine_dir()).await.unwrap();
        assert_eq!(manifest.get("here").unwrap().cwd.as_ref(), Some(&expected));
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_rejects_missing_or_non_directory_cwd() {
        let dir = temp_dir().await;
        let config = config_in(&dir);
        let file = dir.join("not-a-directory");
        tokio::fs::write(&file, "x").await.unwrap();

        let missing = handle_save(
            &config,
            "missing-cwd",
            false,
            Some(&dir.join("missing")),
            vec!["echo".to_string()],
        )
        .await
        .unwrap_err();
        assert!(missing.to_string().contains("does not exist"));

        let not_dir = handle_save(
            &config,
            "file-cwd",
            false,
            Some(&file),
            vec!["echo".to_string()],
        )
        .await
        .unwrap_err();
        assert!(not_dir.to_string().contains("not a directory"));
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_rejects_duplicate_without_force_and_overwrites_with_force() {
        let dir = temp_dir().await;
        let config = config_in(&dir);

        handle_save(
            &config,
            "t",
            false,
            Some(&dir),
            vec!["echo".to_string(), "one".to_string()],
        )
        .await
        .unwrap();

        let err = handle_save(
            &config,
            "t",
            false,
            None,
            vec!["echo".to_string(), "two".to_string()],
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("Task already exists"));

        handle_save(
            &config,
            "t",
            true,
            None,
            vec!["echo".to_string(), "two".to_string()],
        )
        .await
        .unwrap();
        let manifest = TaskManifest::load(config.shine_dir()).await.unwrap();
        assert_eq!(manifest.get("t").unwrap().command, ["echo", "two"]);
        assert_eq!(manifest.get("t").unwrap().cwd, None);

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_rejects_empty_command_and_invalid_name() {
        let dir = temp_dir().await;
        let config = config_in(&dir);

        let err = handle_save(&config, "t", false, None, vec![])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("No command provided"));

        let err = handle_save(&config, "bad name", false, None, vec!["echo".to_string()])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Invalid task name"));

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn delete_removes_task_and_errors_when_missing() {
        let dir = temp_dir().await;
        let config = config_in(&dir);

        handle_save(&config, "t", false, None, vec!["echo".to_string()])
            .await
            .unwrap();
        handle_delete(&config, "t").await.unwrap();

        let err = handle_delete(&config, "t").await.unwrap_err();
        assert!(err.to_string().contains("Task not found"));

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn run_missing_task_reports_clear_error() {
        let dir = temp_dir().await;
        let config = config_in(&dir);

        let err = handle_run(&config, "nope", &[]).await.unwrap_err();
        assert!(err.to_string().contains("Task not found: nope"));
        assert!(err.to_string().contains("shine task list"));

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_executes_saved_command_and_appends_extra() {
        let dir = temp_dir().await;
        let config = config_in(&dir);

        // `true` exits 0 regardless of extra args, so run_task_command returns Ok
        // without calling process::exit.
        handle_save(&config, "ok", false, None, vec!["true".to_string()])
            .await
            .unwrap();
        handle_run(&config, "ok", &["ignored".to_string()])
            .await
            .unwrap();

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_executes_in_saved_working_directory() {
        let dir = temp_dir().await;
        let config = config_in(&dir);
        let project = dir.join("project");
        tokio::fs::create_dir_all(&project).await.unwrap();

        handle_save(
            &config,
            "mark",
            false,
            Some(&project),
            vec!["touch".to_string(), "ran-here".to_string()],
        )
        .await
        .unwrap();
        handle_run(&config, "mark", &[]).await.unwrap();

        assert!(project.join("ran-here").exists());
        assert!(!dir.join("ran-here").exists());
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn run_reports_saved_working_directory_that_disappeared() {
        let dir = temp_dir().await;
        let config = config_in(&dir);
        let project = dir.join("project");
        tokio::fs::create_dir_all(&project).await.unwrap();

        handle_save(
            &config,
            "gone",
            false,
            Some(&project),
            vec!["echo".to_string()],
        )
        .await
        .unwrap();
        tokio::fs::remove_dir(&project).await.unwrap();

        let error = handle_run(&config, "gone", &[]).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("working directory is unavailable")
        );
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_reports_command_not_found() {
        let dir = temp_dir().await;
        let config = config_in(&dir);

        handle_save(
            &config,
            "missing-bin",
            false,
            None,
            vec!["shine-no-such-binary-xyz".to_string()],
        )
        .await
        .unwrap();
        let err = handle_run(&config, "missing-bin", &[]).await.unwrap_err();
        assert!(err.to_string().contains("command not found"));

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
