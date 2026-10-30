//! CLI preparation, confirmation and rendering for broker policy commands.
use super::model::{BrokerAllow, BrokerPolicy, RemoteEnrollmentPlan, WorkspaceSnapshot};
use super::policy::{
    allow_from_snapshot, resolve_release, sha256, validate_name, validate_wire_string,
};
use super::store::{apply_remote_enrollment, load_store, save_store};
use crate::output::{colorize_unified_diff, escape_controls, unified_text_diff};
use crate::{config::Config, env::workspace};
use anyhow::{Context, Result, bail};
use shine_core::runtime::{FileSystemHost, RealHost};
use std::path::Path;

#[allow(clippy::too_many_arguments)] // Mirrors one policy's explicit identity fields.
pub async fn policy_from_workspace(
    name: &str,
    ssh_target: &str,
    project: &str,
    workspace_path: &Path,
    remote_workspace: Option<&str>,
    mode: &str,
    release: &[String],
    release_all_declared: bool,
    argv: &[String],
) -> Result<BrokerPolicy> {
    validate_name(name, "policy name")?;
    if let Some(remote_workspace) = remote_workspace {
        validate_wire_string(remote_workspace, "remote workspace")?;
        if !Path::new(remote_workspace).is_absolute() {
            bail!("remote workspace must be an absolute path");
        }
    }
    let snapshot = workspace::snapshot_for_broker(Some(workspace_path), mode).await?;
    let release = resolve_release(&snapshot, release, release_all_declared)?;
    let allow = allow_from_snapshot(&snapshot, &release, argv)?;
    Ok(BrokerPolicy {
        name: name.to_string(),
        ssh_target: ssh_target.to_string(),
        project: project.to_string(),
        workspace_sha256: sha256(snapshot.workspace_contents.as_bytes()),
        remote_workspace: remote_workspace.map(str::to_string),
        allow: vec![allow],
    })
}

#[allow(clippy::too_many_arguments)] // Mirrors the explicit policy identity fields at the CLI boundary.
pub async fn handle_policy_add(
    config: &Config,
    name: &str,
    ssh_target: &str,
    project: &str,
    workspace_path: &Path,
    remote_workspace: Option<&str>,
    mode: &str,
    release: &[String],
    release_all_declared: bool,
    argv: &[String],
) -> Result<()> {
    let policy = policy_from_workspace(
        name,
        ssh_target,
        project,
        workspace_path,
        remote_workspace,
        mode,
        release,
        release_all_declared,
        argv,
    )
    .await?;
    let _guard = RealHost
        .acquire_operation_lock(&config.shine_dir().join("ssh-secret-broker.lock"))
        .await?;
    let mut store = load_store(config).await?;
    if store.policies.iter().any(|item| item.name == name) {
        bail!(
            "policy {name} already exists; to replace it, use `shine env broker policy update --name {name}` with the policy parameters supplied again: --ssh-target, --workspace, --mode, --release or --release-all-declared, and the command after `--`. Also repeat any --project and --remote-workspace constraints. Review and confirmation are still required for changes. See `shine env broker policy update --help`."
        );
    }
    store.policies.push(policy);
    save_store(config, &store).await?;
    println!("added SSH secret broker policy {name}");
    Ok(())
}

#[allow(clippy::too_many_arguments)] // Mirrors the explicit policy identity fields at the CLI boundary.
pub async fn handle_policy_update(
    config: &Config,
    name: &str,
    ssh_target: &str,
    project: &str,
    workspace_path: &Path,
    remote_workspace: Option<&str>,
    mode: &str,
    release: &[String],
    release_all_declared: bool,
    argv: &[String],
) -> Result<()> {
    let policy = policy_from_workspace(
        name,
        ssh_target,
        project,
        workspace_path,
        remote_workspace,
        mode,
        release,
        release_all_declared,
        argv,
    )
    .await?;
    let store = load_store(config).await?;
    let existing = store
        .policies
        .iter()
        .find(|item| item.name == name)
        .with_context(|| format!("policy {name} does not exist"))?;
    let old = toml::to_string_pretty(existing)?;
    let new = toml::to_string_pretty(&policy)?;
    if old == new {
        println!("policy {name} is current");
        return Ok(());
    }
    println!(
        "{}",
        colorize_unified_diff(&unified_text_diff(&old, &new, None))
    );
    let confirmed = dialoguer::Confirm::new()
        .with_prompt(format!("Replace SSH secret broker policy {name}?"))
        .default(false)
        .interact()
        .context("reading policy update confirmation")?;
    if !confirmed {
        bail!("policy update cancelled");
    }
    // Re-read under the shared mutation lock after human review. A changed or
    // revoked target invalidates this review; unrelated policy edits survive.
    apply_remote_enrollment(
        config,
        &RemoteEnrollmentPlan {
            name: name.to_owned(),
            candidate: policy,
            previous: Some(existing.clone()),
        },
    )
    .await?;
    println!("updated SSH secret broker policy {name}");
    Ok(())
}

pub async fn handle_policy_list(config: &Config) -> Result<()> {
    let store = load_store(config).await?;
    if store.policies.is_empty() && store.file_policies.is_empty() {
        println!("No SSH secret broker policies configured.");
    }
    for policy in &store.file_policies {
        println!(
            "{}: {} (files)",
            policy.name,
            escape_controls(&policy.ssh_target)
        );
    }
    for policy in store.policies {
        println!(
            "{}: {} ({})",
            policy.name,
            crate::output::escape_controls(&policy.ssh_target),
            crate::output::escape_controls(&policy.project)
        );
    }
    Ok(())
}

pub async fn handle_policy_info(config: &Config, name: &str) -> Result<()> {
    let store = load_store(config).await?;
    if let Some(policy) = store.file_policies.iter().find(|p| p.name == name) {
        println!("{}", toml::to_string_pretty(policy)?);
        return Ok(());
    }
    let policy = store
        .policies
        .iter()
        .find(|item| item.name == name)
        .with_context(|| format!("policy {} does not exist", escape_controls(name)))?;
    print!("{}", toml::to_string_pretty(policy)?);
    Ok(())
}

pub async fn handle_policy_remove(config: &Config, name: &str) -> Result<()> {
    let _guard = RealHost
        .acquire_operation_lock(&config.shine_dir().join("ssh-secret-broker.lock"))
        .await?;
    let mut store = load_store(config).await?;
    let before = store.policies.len() + store.file_policies.len();
    store.policies.retain(|item| item.name != name);
    store.file_policies.retain(|item| item.name != name);
    if store.policies.len() + store.file_policies.len() == before {
        bail!("policy {} does not exist", escape_controls(name));
    }
    save_store(config, &store).await?;
    println!("removed SSH secret broker policy {name}");
    Ok(())
}

pub async fn handle_policy_diff(
    config: &Config,
    name: &str,
    workspace_path: &Path,
    mode: &str,
    release: &[String],
    release_all_declared: bool,
    argv: &[String],
) -> Result<()> {
    let store = load_store(config).await?;
    let existing = store
        .policies
        .iter()
        .find(|item| item.name == name)
        .with_context(|| format!("policy {} does not exist", escape_controls(name)))?;
    let candidate = policy_from_workspace(
        name,
        &existing.ssh_target,
        &existing.project,
        workspace_path,
        existing.remote_workspace.as_deref(),
        mode,
        release,
        release_all_declared,
        argv,
    )
    .await?;
    let old = toml::to_string_pretty(existing)?;
    let new = toml::to_string_pretty(&candidate)?;
    if old == new {
        println!("policy {name} is current");
    } else {
        println!(
            "{}",
            colorize_unified_diff(&unified_text_diff(&old, &new, None))
        );
    }
    Ok(())
}

pub async fn handle_describe(
    workspace_path: Option<&Path>,
    mode: &str,
    release: &[String],
    release_all_declared: bool,
    argv: &[String],
) -> Result<()> {
    let snapshot = workspace::snapshot_for_broker(workspace_path, mode).await?;
    let release = resolve_release(&snapshot, release, release_all_declared)?;
    let allow = allow_from_snapshot(&snapshot, &release, argv)?;
    if crate::ssh::broker_session_available() {
        let summary = crate::ssh::describe_broker_workspace(snapshot, &release, argv).await?;
        println!("{summary}");
        return Ok(());
    }
    print_description(&snapshot, &allow);
    Ok(())
}

fn print_description(snapshot: &WorkspaceSnapshot, allow: &BrokerAllow) {
    println!(
        "workspace_sha256 = \"{}\"",
        sha256(snapshot.workspace_contents.as_bytes())
    );
    println!("mode = {:?}", allow.mode);
    println!("argv = {:?}", allow.argv);
    println!("release = {:?}", allow.release);
    for source in &allow.sources {
        println!(
            "source {} {} {:?}",
            escape_controls(&source.path),
            source.sha256,
            source.declared_secrets
        );
    }
}
