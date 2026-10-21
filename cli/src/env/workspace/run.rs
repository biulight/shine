//! Run for workspace environments.
use super::{
    BTreeMap, Config, Context, OsString, Path, Result, SensitiveValues, Zeroize, bail, cache_path,
    capture_sources, command_builder, compile_captured_sources, find_workspace_optional,
    finish_command_status, involves_hybrid, local_cache, merge_broker_values, parse_workspace,
    read_valid_cache, resolve_seal_encryption, resolve_sources, secret, snapshot_for_broker,
    snapshot_input_hash, validate_mode, write_cache,
};

#[allow(clippy::too_many_arguments)] // Command handler keeps independent Clap options explicit.
pub async fn handle_run(
    config: &Config,
    workspace_arg: Option<&Path>,
    mode_arg: Option<&str>,
    no_workspace: bool,
    with: &[String],
    secret_broker: bool,
    broker_secrets: &[String],
    command: &[OsString],
) -> Result<()> {
    let explicit = resolve_explicit_values(config, with).await?;
    if !secret_broker && !broker_secrets.is_empty() {
        bail!("--secret requires --secret-broker");
    }
    if secret_broker {
        let argv = broker_command_argv(command)?;
        if no_workspace {
            if broker_secrets.is_empty() {
                bail!("--no-workspace --secret-broker requires at least one --secret");
            }
            let values = crate::ssh::request_direct_secrets(broker_secrets, &argv).await?;
            return run_command(
                command,
                SensitiveValues::default(),
                false,
                merge_explicit(explicit, values)?,
            )
            .await;
        }
        if !broker_secrets.is_empty() {
            bail!(
                "workspace broker requests derive release keys from policy; do not pass --secret"
            );
        }
        let mode = mode_arg.context("workspace --secret-broker requires --mode")?;
        let snapshot = snapshot_for_broker(workspace_arg, mode).await?;
        let secrets = crate::ssh::request_workspace_secrets(snapshot.clone(), &argv).await?;
        let values = merge_broker_values(&snapshot, secrets)?;
        return run_command(command, values, snapshot.override_process_env, explicit).await;
    }
    // `--no-workspace` disables discovery entirely: only explicit `--with` values
    // and the inherited process environment reach the command. Generated Bun
    // launchers rely on this so a nearby shine.workspace.toml can never hijack them.
    let workspace_path = if no_workspace {
        None
    } else {
        find_workspace_optional(workspace_arg).await?
    };
    let (values, override_process_env) = if let Some(workspace_path) = workspace_path {
        load_run_values(config, &workspace_path, mode_arg, false).await?
    } else {
        if !no_workspace && explicit.is_empty() {
            bail!("shine.workspace.toml was not found; pass --workspace or --no-workspace");
        }
        if mode_arg.is_some() {
            bail!("--mode requires a shine.workspace.toml");
        }
        (SensitiveValues::default(), false)
    };

    run_command(command, values, override_process_env, explicit).await
}

pub(super) async fn load_run_values(
    config: &Config,
    workspace_path: &Path,
    mode_arg: Option<&str>,
    allow_file_only: bool,
) -> Result<(SensitiveValues, bool)> {
    let local_config = local_cache::load_config(config, workspace_path).await?;
    let config = &local_config;
    load_run_values_effective(config, workspace_path, mode_arg, allow_file_only).await
}

pub(super) async fn load_run_values_effective(
    config: &Config,
    workspace_path: &Path,
    mode_arg: Option<&str>,
    allow_file_only: bool,
) -> Result<(SensitiveValues, bool)> {
    let workspace_bytes = tokio::fs::read_to_string(workspace_path).await?;
    let workspace = parse_workspace(workspace_path, &workspace_bytes)?;
    let mode = mode_arg
        .or(workspace.env.default_mode.as_deref())
        .context("environment mode is required; pass --mode or set env.default_mode")?;
    validate_mode(mode)?;
    if workspace.env.files.is_empty() {
        if allow_file_only {
            return Ok((
                SensitiveValues::default(),
                workspace.env.override_process_env,
            ));
        }
        bail!("file-only workspace requires --file-group");
    }
    let sources = resolve_sources(workspace_path, &workspace.env.files, mode)?;
    let captured = capture_sources(&sources).await?;
    let input_hash = snapshot_input_hash(workspace_path, &workspace_bytes, mode, &captured);
    let hybrid = involves_hybrid(&workspace.env.encryption, &captured)?;
    let values = if hybrid {
        local_cache::compile(
            config,
            workspace_path,
            mode,
            &input_hash,
            &workspace.env.encryption,
            &captured,
        )
        .await?
    } else {
        let encryption =
            resolve_seal_encryption(None, &[], Some(&workspace.env.encryption), config)?;
        let cache_path = cache_path(workspace_path, mode)?;
        match read_valid_cache(&cache_path, mode, &input_hash, config).await {
            Ok(Some(values)) => values,
            Ok(None) => {
                let values = compile_captured_sources(&captured, config).await?;
                if let Some(encryption) = &encryption
                    && let Err(error) = write_cache(
                        &cache_path,
                        workspace_path,
                        mode,
                        &input_hash,
                        &values,
                        encryption,
                    )
                    .await
                {
                    eprintln!(
                        "Warning: could not update environment cache: {}",
                        crate::output::escape_controls(&format!("{error:#}"))
                    );
                }
                values
            }
            Err(error) => {
                eprintln!(
                    "Warning: ignoring unreadable environment cache: {}",
                    crate::output::escape_controls(&format!("{error:#}"))
                );
                compile_captured_sources(&captured, config).await?
            }
        }
    };
    Ok((values, workspace.env.override_process_env))
}

pub(super) fn broker_command_argv(command: &[OsString]) -> Result<Vec<String>> {
    command
        .iter()
        .map(|arg| {
            arg.to_str()
                .map(str::to_string)
                .context("secret broker command arguments must be valid UTF-8")
        })
        .collect()
}

pub(super) fn merge_explicit(
    mut explicit: SensitiveValues,
    broker: BTreeMap<String, String>,
) -> Result<SensitiveValues> {
    for (key, mut value) in SensitiveValues::from(broker) {
        if explicit.contains_key(&key) {
            value.zeroize();
            bail!("broker target {key} conflicts with an explicit --with target");
        }
        explicit.insert(key, value);
    }
    Ok(explicit)
}

pub(in crate::env) async fn resolve_explicit_values(
    config: &Config,
    specs: &[String],
) -> Result<SensitiveValues> {
    let parsed = crate::env::parse_env_specs(specs)?;

    let env = crate::env::EnvConfig::from_config(config);
    let mut values = SensitiveValues::default();
    for spec in parsed {
        let value = match crate::env::resolve_stored_value(&env, &spec.source)? {
            crate::env::StoredValue::Secret {
                key: secret_key,
                value: ciphertext,
            } => secret::decrypt_with_config(ciphertext, config)
                .await
                .with_context(|| format!("decrypting {secret_key}"))?,
            crate::env::StoredValue::Plaintext(value) => value.to_string(),
        };
        values.insert(spec.target, value);
    }
    Ok(values)
}

pub(super) async fn run_command(
    command: &[OsString],
    values: SensitiveValues,
    override_process_env: bool,
    explicit: SensitiveValues,
) -> Result<()> {
    let status = command_status(command, &values, override_process_env, &explicit).await;
    drop(values);
    drop(explicit);
    finish_command_status(status?)
}

pub(super) async fn command_status(
    command: &[OsString],
    values: &BTreeMap<String, String>,
    override_process_env: bool,
    explicit: &BTreeMap<String, String>,
) -> Result<std::process::ExitStatus> {
    command_builder(command, values, override_process_env, explicit)?
        .status()
        .await
        .with_context(|| format!("running {}", command[0].to_string_lossy()))
}
