//! Local policy store and shared wire snapshots for SSH secret brokering.

use super::workspace;
use crate::config::Config;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const POLICY_VERSION: u32 = 1;
const POLICY_FILE: &str = "ssh-secret-broker.toml";
const RESERVED_REMOTE_ENV: &[&str] = &[
    "SHINE_SSH_SESSION",
    "SHINE_SSH_TOKEN",
    "SHINE_SSH_REMOTE_SOCK",
    "SHINE_TERMINAL_THEME",
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceSnapshot {
    pub workspace_path: String,
    pub workspace_contents: String,
    pub mode: String,
    pub override_process_env: bool,
    pub sources: Vec<SourceSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceSnapshot {
    /// Path relative to the workspace root where possible, otherwise absolute.
    pub path: String,
    pub contents: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyStore {
    #[serde(default = "policy_version")]
    pub version: u32,
    #[serde(default, rename = "policy")]
    pub policies: Vec<BrokerPolicy>,
}

impl Default for PolicyStore {
    fn default() -> Self {
        Self {
            version: POLICY_VERSION,
            policies: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrokerPolicy {
    pub name: String,
    pub ssh_target: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub project: String,
    pub workspace_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_workspace: Option<String>,
    #[serde(default)]
    pub allow: Vec<BrokerAllow>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrokerAllow {
    pub mode: String,
    pub argv: Vec<String>,
    pub release: Vec<String>,
    pub sources: Vec<BrokerSource>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BrokerSource {
    pub path: String,
    pub sha256: String,
    #[serde(default)]
    pub declared_secrets: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct MatchedPolicy {
    pub policy_name: String,
    pub project: String,
    pub release: Vec<String>,
}

fn policy_version() -> u32 {
    POLICY_VERSION
}

pub fn policy_path(config: &Config) -> PathBuf {
    config.shine_dir().join(POLICY_FILE)
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub async fn load_store(config: &Config) -> Result<PolicyStore> {
    load_store_from(&policy_path(config)).await
}

pub async fn load_stores(config: &Config, overrides: &[PathBuf]) -> Result<PolicyStore> {
    let mut merged = load_store(config).await?;
    for path in overrides {
        let mut extra = load_store_from(path).await?;
        merged.policies.append(&mut extra.policies);
    }
    validate_store(&merged)?;
    Ok(merged)
}

pub async fn load_store_from(path: &Path) -> Result<PolicyStore> {
    validate_policy_file(path, true).await?;
    let contents = match tokio::fs::read_to_string(path).await {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PolicyStore {
                version: POLICY_VERSION,
                policies: Vec::new(),
            });
        }
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let store: PolicyStore =
        toml::from_str(&contents).with_context(|| format!("parsing {}", path.display()))?;
    validate_store(&store)?;
    Ok(store)
}

async fn save_store(config: &Config, store: &PolicyStore) -> Result<()> {
    validate_store(store)?;
    let path = policy_path(config);
    validate_policy_file(&path, true).await?;
    let contents = toml::to_string_pretty(store).context("serializing SSH secret broker policy")?;
    secure_atomic_write(&path, contents.as_bytes()).await?;
    validate_policy_file(&path, false).await
}

async fn secure_atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    tokio::fs::create_dir_all(parent).await?;
    let temp = parent.join(format!(".shine-broker-write-{}", uuid::Uuid::new_v4()));
    #[cfg(unix)]
    let std_file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)
            .with_context(|| format!("creating {}", temp.display()))?
    };
    #[cfg(not(unix))]
    let std_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .with_context(|| format!("creating {}", temp.display()))?;
    let mut file = tokio::fs::File::from_std(std_file);
    if let Err(error) = async {
        file.write_all(contents).await?;
        file.sync_all().await
    }
    .await
    {
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(error).with_context(|| format!("writing {}", temp.display()));
    }
    drop(file);
    crate::persist::finalize_temp(&temp, path).await
}

async fn validate_policy_file(path: &Path, allow_missing: bool) -> Result<()> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(());
        }
        Err(error) => return Err(error).with_context(|| format!("inspecting {}", path.display())),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        bail!(
            "SSH secret broker policy must be a regular file, not a symlink: {}",
            path.display()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let expected_uid = unsafe { libc::geteuid() };
        if metadata.uid() != expected_uid {
            bail!(
                "SSH secret broker policy is not owned by the current user: {}",
                path.display()
            );
        }
        if metadata.mode() & 0o077 != 0 {
            bail!(
                "SSH secret broker policy permissions are too broad (expected 0600): {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn validate_store(store: &PolicyStore) -> Result<()> {
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

fn validate_name(value: &str, what: &str) -> Result<()> {
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
        super::validate_env_key(value)?;
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

#[allow(clippy::too_many_arguments)] // Mirrors one policy's explicit identity fields.
pub async fn policy_from_workspace(
    name: &str,
    ssh_target: &str,
    project: &str,
    workspace_path: &Path,
    remote_workspace: Option<&str>,
    mode: &str,
    release: &[String],
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
    let allow = allow_from_snapshot(&snapshot, release, argv)?;
    Ok(BrokerPolicy {
        name: name.to_string(),
        ssh_target: ssh_target.to_string(),
        project: project.to_string(),
        workspace_sha256: sha256(snapshot.workspace_contents.as_bytes()),
        remote_workspace: remote_workspace.map(str::to_string),
        allow: vec![allow],
    })
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
        argv,
    )
    .await?;
    let mut store = load_store(config).await?;
    if store.policies.iter().any(|item| item.name == name) {
        bail!("policy {name} already exists; use `shine env broker policy update {name}`");
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
        argv,
    )
    .await?;
    let mut store = load_store(config).await?;
    let existing = store
        .policies
        .iter_mut()
        .find(|item| item.name == name)
        .with_context(|| format!("policy {name} does not exist"))?;
    let old = toml::to_string_pretty(&*existing)?;
    let new = toml::to_string_pretty(&policy)?;
    if old == new {
        println!("policy {name} is current");
        return Ok(());
    }
    println!(
        "{}",
        similar::TextDiff::from_lines(&old, &new).unified_diff()
    );
    let confirmed = dialoguer::Confirm::new()
        .with_prompt(format!("Replace SSH secret broker policy {name}?"))
        .default(false)
        .interact()
        .context("reading policy update confirmation")?;
    if !confirmed {
        bail!("policy update cancelled");
    }
    *existing = policy;
    save_store(config, &store).await?;
    println!("updated SSH secret broker policy {name}");
    Ok(())
}

pub async fn handle_policy_list(config: &Config) -> Result<()> {
    let store = load_store(config).await?;
    if store.policies.is_empty() {
        println!("No SSH secret broker policies configured.");
    }
    for policy in store.policies {
        println!(
            "{}: {} ({})",
            policy.name, policy.ssh_target, policy.project
        );
    }
    Ok(())
}

pub async fn handle_policy_info(config: &Config, name: &str) -> Result<()> {
    let store = load_store(config).await?;
    let policy = store
        .policies
        .iter()
        .find(|item| item.name == name)
        .with_context(|| format!("policy {name} does not exist"))?;
    print!("{}", toml::to_string_pretty(policy)?);
    Ok(())
}

pub async fn handle_policy_remove(config: &Config, name: &str) -> Result<()> {
    let mut store = load_store(config).await?;
    let before = store.policies.len();
    store.policies.retain(|item| item.name != name);
    if store.policies.len() == before {
        bail!("policy {name} does not exist");
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
    argv: &[String],
) -> Result<()> {
    let store = load_store(config).await?;
    let existing = store
        .policies
        .iter()
        .find(|item| item.name == name)
        .with_context(|| format!("policy {name} does not exist"))?;
    let candidate = policy_from_workspace(
        name,
        &existing.ssh_target,
        &existing.project,
        workspace_path,
        existing.remote_workspace.as_deref(),
        mode,
        release,
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
            similar::TextDiff::from_lines(&old, &new).unified_diff()
        );
    }
    Ok(())
}

pub async fn handle_describe(
    workspace_path: Option<&Path>,
    mode: &str,
    release: &[String],
    argv: &[String],
) -> Result<()> {
    let snapshot = workspace::snapshot_for_broker(workspace_path, mode).await?;
    let allow = allow_from_snapshot(&snapshot, release, argv)?;
    if crate::ssh::broker_session_available() {
        let summary = crate::ssh::describe_broker_workspace(snapshot, release, argv).await?;
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
            source.path, source.sha256, source.declared_secrets
        );
    }
}

pub async fn add_policy_from_remote_snapshot(
    config: &Config,
    ssh_target: &str,
    snapshot: &WorkspaceSnapshot,
    release: &[String],
    argv: &[String],
) -> Result<String> {
    let allow = allow_from_snapshot(snapshot, release, argv)?;
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
    let mut store = load_store(config).await?;
    if store.policies.iter().any(|item| item.name == name) {
        bail!("policy {name} already exists; inspect or update it explicitly");
    }
    store.policies.push(policy);
    save_store(config, &store).await?;
    Ok(name)
}

pub fn decrypt_workspace_snapshot<'a>(
    config: &'a Config,
    snapshot: &'a WorkspaceSnapshot,
    release: &'a [String],
) -> impl std::future::Future<Output = Result<BTreeMap<String, String>>> + 'a {
    workspace::decrypt_broker_snapshot(config, snapshot, release)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            workspace_path: "/srv/api/shine.workspace.toml".into(),
            workspace_contents: "version = 1\n[env]\nfiles = [\"env/development.toml\"]\n".into(),
            mode: "development".into(),
            override_process_env: false,
            sources: vec![SourceSnapshot {
                path: "env/development.toml".into(),
                contents: "version = 1\n[plain]\nPUBLIC = \"x\"\n[secret]\nAPI_TOKEN = true\nNPM_TOKEN = true\n[payload]\ndata = \"ciphertext\"\n".into(),
            }],
        }
    }

    #[test]
    fn allow_separates_declared_keys_from_release_subset() {
        let snapshot = snapshot();
        let allow = allow_from_snapshot(
            &snapshot,
            &["API_TOKEN".into()],
            &["bun".into(), "test".into()],
        )
        .unwrap();

        assert_eq!(allow.release, ["API_TOKEN"]);
        assert_eq!(
            allow.sources[0].declared_secrets,
            ["API_TOKEN", "NPM_TOKEN"]
        );
    }

    #[test]
    fn exact_policy_match_rejects_changed_argv_or_source() {
        let snapshot = snapshot();
        let allow = allow_from_snapshot(
            &snapshot,
            &["API_TOKEN".into()],
            &["bun".into(), "test".into()],
        )
        .unwrap();
        let store = PolicyStore {
            version: POLICY_VERSION,
            policies: vec![BrokerPolicy {
                name: "dev-api".into(),
                ssh_target: "dev".into(),
                project: "api".into(),
                workspace_sha256: sha256(snapshot.workspace_contents.as_bytes()),
                remote_workspace: None,
                allow: vec![allow],
            }],
        };

        let matched =
            match_workspace_request(&store, "dev", &snapshot, &["bun".into(), "test".into()])
                .unwrap();
        assert_eq!(matched.release, ["API_TOKEN"]);

        assert!(
            match_workspace_request(
                &store,
                "dev",
                &snapshot,
                &["bun".into(), "run".into(), "test".into()],
            )
            .is_err()
        );
        let mut changed = snapshot.clone();
        changed.sources[0].contents.push_str("# changed\n");
        assert!(
            match_workspace_request(&store, "dev", &changed, &["bun".into(), "test".into()],)
                .is_err()
        );
    }

    #[test]
    fn wire_display_fields_reject_control_characters_and_limits() {
        assert!(validate_wire_string("safe", "field").is_ok());
        assert!(validate_wire_string("evil\u{1b}[2J", "field").is_err());
        assert!(validate_wire_string(&"x".repeat(4097), "field").is_err());
        assert!(validate_wire_strings(&vec!["x".into(); 129], "argv").is_err());
    }

    #[tokio::test]
    async fn policy_store_is_written_with_private_permissions() {
        let dir = crate::test_support::make_temp_dir("shine-broker-policy").await;
        let config = Config::new_for_test(&dir);
        let snapshot = snapshot();
        let allow = allow_from_snapshot(
            &snapshot,
            &["API_TOKEN".into()],
            &["bun".into(), "test".into()],
        )
        .unwrap();
        let store = PolicyStore {
            version: POLICY_VERSION,
            policies: vec![BrokerPolicy {
                name: "dev-api".into(),
                ssh_target: "dev".into(),
                project: "api".into(),
                workspace_sha256: sha256(snapshot.workspace_contents.as_bytes()),
                remote_workspace: None,
                allow: vec![allow],
            }],
        };
        save_store(&config, &store).await.unwrap();
        let loaded = load_store(&config).await.unwrap();
        assert_eq!(loaded, store);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = tokio::fs::metadata(policy_path(&config))
                .await
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn additional_local_policy_files_are_merged_and_validated() {
        let dir = crate::test_support::make_temp_dir("shine-broker-policy-merge").await;
        let config = Config::new_for_test(&dir);
        let snapshot = snapshot();
        let allow = allow_from_snapshot(
            &snapshot,
            &["API_TOKEN".into()],
            &["bun".into(), "test".into()],
        )
        .unwrap();
        let extra = PolicyStore {
            version: POLICY_VERSION,
            policies: vec![BrokerPolicy {
                name: "extra-api".into(),
                ssh_target: "dev".into(),
                project: "api".into(),
                workspace_sha256: sha256(snapshot.workspace_contents.as_bytes()),
                remote_workspace: None,
                allow: vec![allow],
            }],
        };
        let path = dir.join("extra.toml");
        tokio::fs::write(&path, toml::to_string(&extra).unwrap())
            .await
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .await
                .unwrap();
        }

        let merged = load_stores(&config, std::slice::from_ref(&path))
            .await
            .unwrap();
        assert_eq!(merged.policies.len(), 1);
        assert_eq!(merged.policies[0].name, "extra-api");
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn policy_store_rejects_symlink_and_broad_permissions() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = crate::test_support::make_temp_dir("shine-broker-policy-safety").await;
        let config = Config::new_for_test(&dir);
        let path = policy_path(&config);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        let target = dir.join("real-policy.toml");
        tokio::fs::write(&target, "version = 1\n").await.unwrap();
        symlink(&target, &path).unwrap();
        assert!(
            load_store(&config)
                .await
                .unwrap_err()
                .to_string()
                .contains("symlink")
        );
        tokio::fs::remove_file(&path).await.unwrap();
        tokio::fs::write(&path, "version = 1\n").await.unwrap();
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .await
            .unwrap();
        assert!(
            load_store(&config)
                .await
                .unwrap_err()
                .to_string()
                .contains("too broad")
        );
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}
