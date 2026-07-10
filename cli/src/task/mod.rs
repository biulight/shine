//! `shine task` — a lightweight personal shortcut-command registry.
//!
//! Tasks are user runtime state (stored in `<shine_dir>/tasks.toml`, see
//! [`manifest`]), not embedded presets. Commands are saved as an argv array and
//! executed directly with no shell (`std::process::Command`), inheriting the
//! caller's stdio and environment. Users who need shell syntax (pipes,
//! redirects, globbing) save an explicit `sh -c '...'` invocation.
//!
//! `shine run <name>` is a top-level alias for `shine task run <name>`; it has
//! no independent semantics or storage.
//!
//! Platform note: direct execution runs any real executable on every platform,
//! but the `sh -c '...'` escape hatch is Unix-only (Windows has no `sh`).

pub mod manifest;

use anyhow::{Result, bail};

use crate::colors;
use crate::config::Config;
use manifest::TaskManifest;

const NOT_FOUND_HINT: &str = "Run `shine task list` to see saved tasks.";

pub async fn handle_save(
    config: &Config,
    name: &str,
    force: bool,
    command: Vec<String>,
) -> Result<()> {
    validate_task_name(name)?;
    if command.is_empty() {
        bail!("No command provided.\n\nUsage:\n  shine task save <name> -- <command...>");
    }

    let mut manifest = TaskManifest::load(config.shine_dir()).await?;
    if !force && manifest.get(name).is_some() {
        bail!("Task already exists: {name}\n\nUse `--force` to replace it.");
    }

    let rendered = render_command(&command);
    manifest.upsert(name, command);
    manifest.save(config.shine_dir()).await?;

    println!("{}", colors::green(&format!("Saved task {name}")));
    println!("{rendered}");
    Ok(())
}

pub async fn handle_run(config: &Config, name: &str, extra: &[String]) -> Result<()> {
    let manifest = TaskManifest::load(config.shine_dir()).await?;
    let Some(entry) = manifest.get(name) else {
        bail!("Task not found: {name}\n\n{NOT_FOUND_HINT}");
    };

    let mut argv = entry.command.clone();
    argv.extend_from_slice(extra);

    // Announce on stderr so a task's own stdout stays clean for piping.
    eprintln!(
        "{}: {}",
        colors::bold(&format!("Running {name}")),
        render_command(&argv)
    );

    run_task_command(name, &argv)
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
fn run_task_command(name: &str, argv: &[String]) -> Result<()> {
    let Some((program, args)) = argv.split_first() else {
        bail!("Task {name} has no command to run.");
    };

    let status = match std::process::Command::new(program).args(args).status() {
        Ok(status) => status,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
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

/// Render an argv array back into a copy-paste-safe shell command line by
/// single-quoting any argument that contains characters a shell would interpret.
fn render_command(argv: &[String]) -> String {
    argv.iter()
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }
    if arg.chars().all(is_shell_safe) {
        return arg.to_string();
    }
    // Single-quote the whole argument; the only character a single-quoted string
    // cannot contain is `'`, which is closed, escaped, and reopened as `'\''`.
    format!("'{}'", arg.replace('\'', "'\\''"))
}

/// Characters that never need quoting: alphanumerics plus punctuation that is
/// inert to POSIX shells and common in paths, URLs, and rsync targets
/// (e.g. `host:/var/www/`).
fn is_shell_safe(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(c, '_' | '-' | '.' | '/' | ':' | ',' | '=' | '@' | '%' | '+')
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
        Config::new_for_test(dir)
    }

    async fn temp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("shine-task-test-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        dir
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
    async fn save_rejects_duplicate_without_force_and_overwrites_with_force() {
        let dir = temp_dir().await;
        let config = config_in(&dir);

        handle_save(
            &config,
            "t",
            false,
            vec!["echo".to_string(), "one".to_string()],
        )
        .await
        .unwrap();

        let err = handle_save(
            &config,
            "t",
            false,
            vec!["echo".to_string(), "two".to_string()],
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("Task already exists"));

        handle_save(
            &config,
            "t",
            true,
            vec!["echo".to_string(), "two".to_string()],
        )
        .await
        .unwrap();
        let manifest = TaskManifest::load(config.shine_dir()).await.unwrap();
        assert_eq!(manifest.get("t").unwrap().command, ["echo", "two"]);

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn save_rejects_empty_command_and_invalid_name() {
        let dir = temp_dir().await;
        let config = config_in(&dir);

        let err = handle_save(&config, "t", false, vec![]).await.unwrap_err();
        assert!(err.to_string().contains("No command provided"));

        let err = handle_save(&config, "bad name", false, vec!["echo".to_string()])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Invalid task name"));

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn delete_removes_task_and_errors_when_missing() {
        let dir = temp_dir().await;
        let config = config_in(&dir);

        handle_save(&config, "t", false, vec!["echo".to_string()])
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
        handle_save(&config, "ok", false, vec!["true".to_string()])
            .await
            .unwrap();
        handle_run(&config, "ok", &["ignored".to_string()])
            .await
            .unwrap();

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
            vec!["shine-no-such-binary-xyz".to_string()],
        )
        .await
        .unwrap();
        let err = handle_run(&config, "missing-bin", &[]).await.unwrap_err();
        assert!(err.to_string().contains("command not found"));

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
