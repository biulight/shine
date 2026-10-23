//! Pure policy validation, exact request matching and enrollment review.
use super::model::{
    BrokerAllow, BrokerPolicy, BrokerSource, MatchedPolicy, POLICY_VERSION, PolicyStore,
    RemoteEnrollmentPlan, WorkspaceSnapshot,
};
use crate::env::{RESERVED_REMOTE_ENV, workspace};
use anyhow::{Context, Result, bail};
pub use shine_core::digest::sha256_hex as sha256;
use std::{collections::BTreeSet, path::Path};

pub(super) fn validate_store(store: &PolicyStore) -> Result<()> {
    if store.version != POLICY_VERSION {
        bail!(
            "unsupported SSH secret broker policy version {}",
            store.version
        );
    }
    let mut names = BTreeSet::new();
    let mut selectors = BTreeSet::new();
    for policy in &store.policies {
        validate_name(&policy.name, "policy name")?;
        if !names.insert(policy.name.clone()) {
            bail!("duplicate SSH secret broker policy name: {}", policy.name);
        }
        if policy.ssh_target.trim().is_empty() {
            bail!("policy {} has an empty ssh_target", policy.name);
        }
        validate_wire_string(&policy.ssh_target, "ssh target")?;
        if let Some(remote_workspace) = &policy.remote_workspace {
            validate_wire_string(remote_workspace, "remote workspace")?;
            if !Path::new(remote_workspace).is_absolute() {
                bail!(
                    "policy {} remote_workspace must be an absolute path",
                    policy.name
                );
            }
        }
        validate_digest(&policy.workspace_sha256)?;
        if policy.allow.is_empty() {
            bail!(
                "policy {} must contain at least one allow entry",
                policy.name
            );
        }
        for allow in &policy.allow {
            workspace::validate_broker_mode(&allow.mode)?;
            if allow.argv.is_empty() {
                bail!("policy {} contains an empty argv", policy.name);
            }
            validate_wire_strings(&allow.argv, "argv")?;
            validate_release(&allow.release)?;
            if allow.sources.is_empty() {
                bail!(
                    "policy {} contains an allow entry with no sources",
                    policy.name
                );
            }
            let mut source_paths = BTreeSet::new();
            for source in &allow.sources {
                if source.path.is_empty() || !source_paths.insert(source.path.clone()) {
                    bail!(
                        "policy {} contains an empty or duplicate source path",
                        policy.name
                    );
                }
                validate_wire_string(&source.path, "source path")?;
                validate_digest(&source.sha256)?;
                validate_release(&source.declared_secrets)?;
            }
            if !allow.release.iter().all(|key| {
                allow
                    .sources
                    .iter()
                    .any(|item| item.declared_secrets.contains(key))
            }) {
                bail!(
                    "policy {} releases a key not declared by its sources",
                    policy.name
                );
            }
            let selector = format!(
                "{}\0{}\0{}\0{:?}\0{:?}",
                policy.ssh_target, policy.workspace_sha256, allow.mode, allow.argv, allow.sources
            );
            if !selectors.insert(selector) {
                bail!("multiple policy entries have the same exact request selector");
            }
        }
    }
    Ok(())
}

pub(super) fn validate_name(value: &str, what: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        bail!("{what} must contain only letters, digits, dots, hyphens, and underscores");
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("SHA-256 digest must be 64 hexadecimal characters");
    }
    Ok(())
}

fn validate_release(values: &[String]) -> Result<()> {
    let mut unique = BTreeSet::new();
    for value in values {
        crate::env::validate_env_key(value)?;
        if RESERVED_REMOTE_ENV.contains(&value.as_str()) {
            bail!("cannot release into shine-managed SSH variable {value}");
        }
        if !unique.insert(value) {
            bail!("duplicate secret key: {value}");
        }
    }
    Ok(())
}

pub fn validate_wire_strings(values: &[String], what: &str) -> Result<()> {
    if values.len() > 128 {
        bail!("{what} contains too many values");
    }
    for value in values {
        validate_wire_string(value, what)?;
    }
    Ok(())
}

pub fn validate_wire_string(value: &str, what: &str) -> Result<()> {
    if value.len() > 4096 {
        bail!("{what} exceeds the 4096-byte limit");
    }
    if value
        .chars()
        .any(|ch| ch == '\0' || (ch.is_control() && !matches!(ch, '\t')))
    {
        bail!("{what} contains a disallowed control character");
    }
    Ok(())
}

pub fn allow_from_snapshot(
    snapshot: &WorkspaceSnapshot,
    release: &[String],
    argv: &[String],
) -> Result<BrokerAllow> {
    validate_release(release)?;
    validate_wire_strings(argv, "argv")?;
    let mut sources = Vec::with_capacity(snapshot.sources.len());
    let mut all_declared = BTreeSet::new();
    for source in &snapshot.sources {
        let declared_secrets =
            workspace::declared_secrets_from_source(&source.path, &source.contents)?;
        all_declared.extend(declared_secrets.iter().cloned());
        sources.push(BrokerSource {
            path: source.path.clone(),
            sha256: sha256(source.contents.as_bytes()),
            declared_secrets,
        });
    }
    for key in release {
        if !all_declared.contains(key) {
            bail!("release key {key} is not declared by the selected workspace sources");
        }
    }
    Ok(BrokerAllow {
        mode: snapshot.mode.clone(),
        argv: argv.to_vec(),
        release: release.to_vec(),
        sources,
    })
}

pub fn resolve_release(
    snapshot: &WorkspaceSnapshot,
    requested: &[String],
    release_all_declared: bool,
) -> Result<Vec<String>> {
    if release_all_declared {
        if !requested.is_empty() {
            bail!("--release and --release-all-declared are mutually exclusive");
        }
        let mut declared = BTreeSet::new();
        for source in &snapshot.sources {
            declared.extend(workspace::declared_secrets_from_source(
                &source.path,
                &source.contents,
            )?);
        }
        if declared.is_empty() {
            bail!("--release-all-declared found no declared workspace secrets");
        }
        return Ok(declared.into_iter().collect());
    }
    if requested.is_empty() {
        bail!("pass at least one --release KEY or --release-all-declared");
    }
    validate_release(requested)?;
    Ok(requested.to_vec())
}

pub fn match_workspace_request(
    store: &PolicyStore,
    ssh_target: &str,
    snapshot: &WorkspaceSnapshot,
    argv: &[String],
) -> Result<MatchedPolicy> {
    validate_snapshot(snapshot)?;
    validate_wire_strings(argv, "argv")?;
    let workspace_digest = sha256(snapshot.workspace_contents.as_bytes());
    let actual_sources = snapshot
        .sources
        .iter()
        .map(|source| {
            Ok(BrokerSource {
                path: source.path.clone(),
                sha256: sha256(source.contents.as_bytes()),
                declared_secrets: workspace::declared_secrets_from_source(
                    &source.path,
                    &source.contents,
                )?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let mut matches = Vec::new();
    for policy in &store.policies {
        if policy.ssh_target != ssh_target || policy.workspace_sha256 != workspace_digest {
            continue;
        }
        if let Some(expected) = &policy.remote_workspace
            && expected != &snapshot.workspace_path
        {
            continue;
        }
        for allow in &policy.allow {
            if allow.mode == snapshot.mode && allow.argv == argv && allow.sources == actual_sources
            {
                matches.push(MatchedPolicy {
                    policy_name: policy.name.clone(),
                    project: policy.project.clone(),
                    release: allow.release.clone(),
                });
            }
        }
    }
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => bail!("no SSH secret broker policy matches this workspace request"),
        _ => bail!("multiple SSH secret broker policies match this workspace request"),
    }
}

pub fn validate_snapshot(snapshot: &WorkspaceSnapshot) -> Result<()> {
    validate_wire_string(&snapshot.workspace_path, "workspace path")?;
    validate_wire_string(&snapshot.mode, "mode")?;
    if snapshot.workspace_contents.len() > 256 * 1024 {
        bail!("workspace contents exceed the 256 KiB limit");
    }
    if snapshot.sources.len() > 64 {
        bail!("workspace request contains too many sources");
    }
    let total = snapshot.sources.iter().try_fold(0usize, |sum, source| {
        validate_wire_string(&source.path, "source path")?;
        sum.checked_add(source.contents.len())
            .context("source size overflow")
    })?;
    if total > 768 * 1024 {
        bail!("workspace source contents exceed the 768 KiB limit");
    }
    Ok(())
}

impl RemoteEnrollmentPlan {
    pub fn action_label(&self) -> String {
        if self.previous.is_some() {
            format!("update local policy {}", self.name)
        } else {
            format!("create local policy {}", self.name)
        }
    }

    pub fn diff(&self) -> Result<Option<String>> {
        let Some(previous) = &self.previous else {
            return Ok(None);
        };
        let old = toml::to_string_pretty(previous)?;
        let new = toml::to_string_pretty(&self.candidate)?;
        Ok(Some(crate::output::unified_text_diff(&old, &new, None)))
    }
}

pub fn plan_remote_enrollment(
    store: &PolicyStore,
    ssh_target: &str,
    snapshot: &WorkspaceSnapshot,
    release: &[String],
    argv: &[String],
    update_policy: Option<&str>,
) -> Result<RemoteEnrollmentPlan> {
    let allow = allow_from_snapshot(snapshot, release, argv)?;
    if let Some(name) = update_policy {
        validate_name(name, "policy name")?;
        let existing = store
            .policies
            .iter()
            .find(|policy| policy.name == name)
            .with_context(|| format!("policy {name} does not exist"))?;
        if existing.ssh_target != ssh_target {
            bail!(
                "policy {name} targets {}, not the current SSH target {ssh_target}",
                existing.ssh_target
            );
        }
        if let Some(expected) = &existing.remote_workspace
            && expected != &snapshot.workspace_path
        {
            bail!(
                "policy {name} requires remote workspace {expected}, but the request came from {}",
                snapshot.workspace_path
            );
        }
        let matching = existing
            .allow
            .iter()
            .enumerate()
            .filter(|(_, item)| item.mode == snapshot.mode && item.argv == argv)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let [index] = matching.as_slice() else {
            bail!(
                "policy {name} must contain exactly one allow entry with mode {} and the requested argv before it can be updated from remote metadata",
                snapshot.mode
            );
        };
        let mut candidate = existing.clone();
        candidate.workspace_sha256 = sha256(snapshot.workspace_contents.as_bytes());
        candidate.allow[*index] = allow;
        let mut proposed = store.clone();
        let target = proposed
            .policies
            .iter_mut()
            .find(|policy| policy.name == name)
            .expect("existing policy was cloned");
        *target = candidate.clone();
        validate_store(&proposed)?;
        return Ok(RemoteEnrollmentPlan {
            name: name.to_string(),
            candidate,
            previous: Some(existing.clone()),
        });
    }

    let project = Path::new(&snapshot.workspace_path)
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .unwrap_or("project");
    let program = argv.first().map(String::as_str).unwrap_or("command");
    let raw_name = format!("{ssh_target}-{project}-{}-{program}", snapshot.mode);
    let name = raw_name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    validate_name(&name, "generated policy name")?;
    let policy = BrokerPolicy {
        name: name.clone(),
        ssh_target: ssh_target.to_string(),
        project: project.to_string(),
        workspace_sha256: sha256(snapshot.workspace_contents.as_bytes()),
        remote_workspace: None,
        allow: vec![allow],
    };
    if store.policies.iter().any(|item| item.name == name) {
        bail!("policy {name} already exists; inspect or update it explicitly");
    }
    let mut proposed = store.clone();
    proposed.policies.push(policy.clone());
    validate_store(&proposed)?;
    Ok(RemoteEnrollmentPlan {
        name,
        candidate: policy,
        previous: None,
    })
}
