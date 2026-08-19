//! App-file generator runner.
//!
//! A generator is an implicit lifecycle command: its stdout becomes the
//! effective source bytes used by install, status/update, and upgrade. External
//! generator code is therefore gated by `allow_app_hooks`, just like lifecycle
//! hooks. Only explicitly declared config env values are injected.

use super::metadata::{AppCategory, AppFile, ArtifactRuntime};
use crate::config::Config;
use anyhow::{Context, Result, bail};
use directories::BaseDirs;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;
use tokio::fs;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

const GENERATOR_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_STDOUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_STDERR_BYTES: usize = 64 * 1024;

pub(super) async fn generate(
    config: &Config,
    category: &AppCategory,
    file: &AppFile,
    env: &BTreeMap<String, String>,
) -> Result<Option<Vec<u8>>> {
    let Some(generator) = &file.generator else {
        return Ok(None);
    };
    if !env.contains_key(&generator.when_env) {
        return Ok(None);
    }

    let resolved = resolve_script(config, &category.name, &generator.script).await?;
    let mut command = match generator.runtime {
        ArtifactRuntime::Bun => {
            crate::proc::ensure_command("bun").with_context(|| {
                format!(
                    "app '{}' generator requires Bun (https://bun.sh)",
                    category.name
                )
            })?;
            let mut command = Command::new("bun");
            command.arg(&resolved.path);
            command
        }
        ArtifactRuntime::Native => Command::new(&resolved.path),
    };

    for spec in &generator.env {
        command.env_remove(&spec.target);
        let value = env.get(&spec.source).ok_or_else(|| {
            anyhow::anyhow!(
                "app '{}' generator requires config env '{}'",
                category.name,
                spec.source
            )
        })?;
        command.env(&spec.target, value);
    }

    let source_dir = config.presets_dir().join("app").join(&category.name);
    let overlay_dir = config
        .active_presets_overlay_dir()
        .map(|dir| dir.join("app").join(&category.name))
        .filter(|dir| dir.exists());
    let cache_dir = BaseDirs::new()
        .context("resolving system cache directory")?
        .cache_dir()
        .join("shine")
        .join("app")
        .join(&category.name);
    let state_dir = config
        .shine_dir()
        .join("state")
        .join("app")
        .join(&category.name);
    command
        .current_dir(
            resolved
                .path
                .parent()
                .context("generator script has no parent directory")?,
        )
        .env("SHINE_APP_ID", &category.name)
        .env(
            "SHINE_APP_DIR",
            resolved.path.parent().unwrap_or(Path::new(".")),
        )
        .env("SHINE_APP_SOURCE_DIR", &source_dir)
        .env(
            "SHINE_APP_HTTP_DIR",
            config
                .shine_dir()
                .join("http")
                .join("app")
                .join(&category.name),
        )
        .env("SHINE_CONFIG_DIR", config.shine_dir())
        .env("SHINE_CACHE_DIR", cache_dir)
        .env("SHINE_STATE_DIR", state_dir)
        .kill_on_drop(true);
    if let Some(overlay_dir) = overlay_dir {
        command.env("SHINE_APP_OVERLAY_DIR", overlay_dir);
    }

    let output = run_generator_command(
        &mut command,
        &category.name,
        GENERATOR_TIMEOUT,
        MAX_STDOUT_BYTES,
        MAX_STDERR_BYTES,
    )
    .await;
    resolved.cleanup().await;
    let output = output?;

    if !output.status.success() {
        bail!(
            "app '{}' generator exited with {} (details redacted)",
            category.name,
            output.status
        );
    }
    let content = String::from_utf8(output.stdout)
        .with_context(|| format!("app '{}' generator output is not UTF-8", category.name))?;

    if !output.stderr.is_empty() {
        let note = String::from_utf8_lossy(&output.stderr);
        let note = note.trim();
        if !note.is_empty() {
            eprintln!(
                "  {} {}: {}",
                crate::colors::symbol("!"),
                category.name,
                note
            );
        }
    }

    Ok(Some(content.into_bytes()))
}

#[derive(Debug)]
struct GeneratorOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

enum LimitedReadError {
    LimitExceeded,
    Io(std::io::Error),
}

async fn read_limited(
    mut stream: impl AsyncRead + Unpin,
    limit: usize,
) -> std::result::Result<Vec<u8>, LimitedReadError> {
    let mut output = Vec::with_capacity(limit.min(64 * 1024));
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(LimitedReadError::Io)?;
        if read == 0 {
            return Ok(output);
        }
        let remaining = limit.saturating_sub(output.len());
        if read > remaining {
            return Err(LimitedReadError::LimitExceeded);
        }
        output.extend_from_slice(&chunk[..read]);
    }
}

async fn terminate_child(child: &mut tokio::process::Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

async fn run_generator_command(
    command: &mut Command,
    app_id: &str,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> Result<GeneratorOutput> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .with_context(|| format!("running app '{app_id}' generator"))?;
    let stdout = child
        .stdout
        .take()
        .context("generator stdout pipe is unavailable")?;
    let stderr = child
        .stderr
        .take()
        .context("generator stderr pipe is unavailable")?;
    let stdout_reader = read_limited(stdout, stdout_limit);
    let stderr_reader = read_limited(stderr, stderr_limit);
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(stdout_reader, stderr_reader, deadline);

    let mut status = None;
    let mut stdout = None;
    let mut stderr = None;
    loop {
        tokio::select! {
            _ = &mut deadline => {
                terminate_child(&mut child).await;
                bail!("app '{app_id}' generator timed out");
            }
            result = &mut stdout_reader, if stdout.is_none() => {
                match result {
                    Ok(bytes) => stdout = Some(bytes),
                    Err(LimitedReadError::LimitExceeded) => {
                        terminate_child(&mut child).await;
                        bail!(
                            "app '{app_id}' generator output exceeds the {} MiB limit",
                            stdout_limit / 1024 / 1024
                        );
                    }
                    Err(LimitedReadError::Io(error)) => {
                        terminate_child(&mut child).await;
                        return Err(error).context("reading generator stdout");
                    }
                }
            }
            result = &mut stderr_reader, if stderr.is_none() => {
                match result {
                    Ok(bytes) => stderr = Some(bytes),
                    Err(LimitedReadError::LimitExceeded) => {
                        terminate_child(&mut child).await;
                        bail!(
                            "app '{app_id}' generator stderr exceeds the {} KiB limit",
                            stderr_limit / 1024
                        );
                    }
                    Err(LimitedReadError::Io(error)) => {
                        terminate_child(&mut child).await;
                        return Err(error).context("reading generator stderr");
                    }
                }
            }
            result = child.wait(), if status.is_none() => {
                status = Some(
                    result.with_context(|| format!("waiting for app '{app_id}' generator"))?
                );
            }
        }

        match (status.take(), stdout.take(), stderr.take()) {
            (Some(status), Some(stdout), Some(stderr)) => {
                return Ok(GeneratorOutput {
                    status,
                    stdout,
                    stderr,
                });
            }
            (pending_status, pending_stdout, pending_stderr) => {
                status = pending_status;
                stdout = pending_stdout;
                stderr = pending_stderr;
            }
        }
    }
}

struct ResolvedScript {
    path: PathBuf,
    temp_dir: Option<PathBuf>,
}

impl ResolvedScript {
    async fn cleanup(&self) {
        if let Some(dir) = &self.temp_dir {
            let _ = fs::remove_dir_all(dir).await;
        }
    }
}

impl Drop for ResolvedScript {
    fn drop(&mut self) {
        if let Some(dir) = &self.temp_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

async fn resolve_script(config: &Config, app_id: &str, script: &Path) -> Result<ResolvedScript> {
    let category_rel = Path::new("app").join(app_id);
    let overlay_script = config
        .active_presets_overlay_dir()
        .map(|dir| dir.join(&category_rel).join(script))
        .filter(|path| path.exists());

    if config.is_external_presets || overlay_script.is_some() {
        if !config.allow_app_hooks {
            bail!(
                "app '{app_id}' generator skipped: set allow_app_hooks = true to allow external app generators"
            );
        }
        let path = overlay_script.unwrap_or_else(|| config.preset_path(category_rel.join(script)));
        if !path.exists() {
            bail!(
                "app '{app_id}' generator script not found: {}",
                script.display()
            );
        }
        return Ok(ResolvedScript {
            path,
            temp_dir: None,
        });
    }

    let asset_key = format!("app/{app_id}/{}", script.display());
    let bytes = crate::presets::read_asset_bytes(&asset_key)
        .with_context(|| format!("embedded generator script not found: {}", script.display()))?;
    let temp_dir = std::env::temp_dir().join(format!("shine-generator-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&temp_dir)
        .await
        .context("creating generator temporary directory")?;
    let path = temp_dir.join(
        script
            .file_name()
            .context("generator script must have a file name")?,
    );
    fs::write(&path, bytes)
        .await
        .context("writing embedded generator script")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .await
            .context("marking embedded generator script executable")?;
    }
    Ok(ResolvedScript {
        path,
        temp_dir: Some(temp_dir),
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::apps::metadata::{AppCategory, AppFile, AppGenerator, AppListMode};
    use crate::env::EnvVarSpec;
    use crate::install_core::AppInstallStrategy;
    use std::os::unix::fs::PermissionsExt;

    async fn fixture() -> (
        PathBuf,
        Config,
        AppCategory,
        AppFile,
        BTreeMap<String, String>,
    ) {
        let root = crate::test_support::make_temp_dir("shine-generator").await;
        let mut config = Config::new_for_test(&root);
        config.is_external_presets = true;
        config.allow_app_hooks = true;
        let category_dir = config.presets_dir().join("app/sample");
        fs::create_dir_all(&category_dir).await.unwrap();
        let script = category_dir.join("generate.sh");
        fs::write(&script, b"#!/bin/sh\nprintf 'generated\\n'\n")
            .await
            .unwrap();
        fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
            .await
            .unwrap();

        let file = AppFile {
            source_rel: PathBuf::from("fallback.txt"),
            target_rel: PathBuf::from("generated.txt"),
            destination_root: None,
            description: None,
            display_name: None,
            legacy_dest_annotation: None,
            transforms: Vec::new(),
            install_strategy: AppInstallStrategy::Copy,
            requires_admin: false,
            restart_hint: None,
            generator: Some(AppGenerator {
                script: PathBuf::from("generate.sh"),
                runtime: ArtifactRuntime::Native,
                env: vec![EnvVarSpec {
                    source: "SOURCE_URL".to_string(),
                    target: "SOURCE_URL".to_string(),
                }],
                when_env: "SOURCE_URL".to_string(),
                auto: true,
            }),
        };
        let category = AppCategory {
            name: "sample".to_string(),
            description: None,
            destination_root: Some(root.join("dest").display().to_string()),
            files: vec![file.clone()],
            list_mode: AppListMode::Files,
            post_upgrade: Vec::new(),
            post_install: Vec::new(),
            uses_metadata: true,
            has_explicit_files: true,
            artifact: None,
        };
        let env = BTreeMap::from([(
            "SOURCE_URL".to_string(),
            "https://example.test/subscription".to_string(),
        )]);
        (root, config, category, file, env)
    }

    #[tokio::test]
    async fn external_generator_runs_when_explicitly_allowed() {
        let (root, config, category, file, env) = fixture().await;
        let output = generate(&config, &category, &file, &env)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(output, b"generated\n");
        fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn external_generator_requires_app_hook_opt_in() {
        let (root, mut config, category, file, env) = fixture().await;
        config.allow_app_hooks = false;
        let error = generate(&config, &category, &file, &env).await.unwrap_err();
        assert!(error.to_string().contains("allow_app_hooks"));
        fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn generator_uses_static_fallback_when_condition_is_absent() {
        let (root, config, category, file, _) = fixture().await;
        assert!(
            generate(&config, &category, &file, &BTreeMap::new())
                .await
                .unwrap()
                .is_none()
        );
        fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn generator_is_terminated_when_stdout_exceeds_limit() {
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("i=0; while [ \"$i\" -lt 2048 ]; do printf x; i=$((i + 1)); done; sleep 10");

        let error = tokio::time::timeout(
            Duration::from_secs(2),
            run_generator_command(&mut command, "sample", Duration::from_secs(30), 1024, 1024),
        )
        .await
        .expect("generator was not terminated promptly")
        .unwrap_err();

        assert!(error.to_string().contains("output exceeds"));
    }

    #[tokio::test]
    async fn generator_is_terminated_when_stderr_exceeds_limit() {
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("i=0; while [ \"$i\" -lt 2048 ]; do printf x >&2; i=$((i + 1)); done; sleep 10");

        let error = tokio::time::timeout(
            Duration::from_secs(2),
            run_generator_command(&mut command, "sample", Duration::from_secs(30), 1024, 1024),
        )
        .await
        .expect("generator was not terminated promptly")
        .unwrap_err();

        assert!(error.to_string().contains("stderr exceeds"));
    }
}
