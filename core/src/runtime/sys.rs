mod inspect;
mod lifecycle;
mod transaction;
use super::sys_action_executor::SysReceiptTransitionV1;
use crate::action::{
    ActionIrV1, DeclarativeActionV1, ManagedFileCreationWithBackupSpecV1,
    ManagedFileRelocationBackupV1, ManagedFileRelocationSpecV1, ManagedFileRemoveSpecV1,
    ManagedFileRemoveWithBackupSpecV1, ManagedFileUpdateSpecV1, SysSplitDnsStateV1,
};
use crate::install::file_ops::{
    InstallOutcome, UninstallOutcome, install_bytes_with_host, uninstall_entry_with_host,
};
use crate::install::{AppEntry, AppInstallStrategy, hash_content};
use crate::lifecycle::LifecycleEffect;
use crate::lifecycle::{
    LifecycleOperation, LifecycleOutcomeV1, LifecycleResultV1, LifecycleStatus,
};
use crate::runtime::{
    CoreRuntime, FileSystemHost, PrivilegedFileSystemHost, RuntimeEvent, RuntimeInteraction,
    RuntimeObserver, SplitDnsHost, SplitDnsRequest, SplitDnsState, SysInstalledRow, SysItem,
    SysItemMode, SysItemOutcome, SysUpdateRow, SysUpgradeReport,
};
use anyhow::Context;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

pub const RECEIPT_VERSION: u32 = 1;
pub const SYS_MANIFEST_FILE: &str = "sys-manifest.toml";
pub const SYS_MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SysDriverKind {
    #[default]
    Script,
    SplitDns,
    ManagedFile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SysItemStatus {
    Installed,
    AlreadyInstalled,
    Skipped,
    Updated,
    NeedsAction,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "driver", rename_all = "kebab-case")]
pub enum SystemReceipt {
    Script { version: u32 },
    SplitDns(SplitDnsReceipt),
    ManagedFile(ManagedFileReceipt),
}

impl SystemReceipt {
    pub fn script() -> Self {
        Self::Script {
            version: RECEIPT_VERSION,
        }
    }

    pub fn driver(&self) -> SysDriverKind {
        match self {
            Self::Script { .. } => SysDriverKind::Script,
            Self::SplitDns(_) => SysDriverKind::SplitDns,
            Self::ManagedFile(_) => SysDriverKind::ManagedFile,
        }
    }

    pub fn requires_admin(&self) -> bool {
        match self {
            Self::Script { .. } => false,
            Self::SplitDns(_) => true,
            Self::ManagedFile(receipt) => receipt.privileged,
        }
    }

    pub fn ensure_supported(&self) -> Result<()> {
        let version = match self {
            Self::Script { version } => *version,
            Self::SplitDns(receipt) => receipt.version,
            Self::ManagedFile(receipt) => receipt.version,
        };
        if version != RECEIPT_VERSION {
            bail!("unsupported system resource receipt version {version}");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SplitDnsReceipt {
    pub version: u32,
    pub os_id: String,
    pub item_id: String,
    pub domain: String,
    pub servers: Vec<String>,
    pub resource: String,
    pub content_hash: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagedFileReceipt {
    pub version: u32,
    pub destination: PathBuf,
    pub backup: Option<PathBuf>,
    pub content_hash: u64,
    pub privileged: bool,
    pub restart_hint: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ResourceOutcome {
    pub changed: bool,
    pub effects: Vec<LifecycleEffect>,
    pub detail: String,
    pub receipt: Option<SystemReceipt>,
    pub restart_hint: Option<String>,
}

#[derive(Debug)]
pub struct ResourceConflict {
    message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SplitDnsDomainRequest {
    pub os_id: String,
    pub item_id: String,
    pub domain: String,
    pub servers: String,
    pub dry_run: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SysManagedAction {
    Apply,
    Remove,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SysManagedRequest {
    pub os_id: String,
    pub target: Option<String>,
    pub action: SysManagedAction,
    pub dry_run: bool,
    pub operation: LifecycleOperation,
}

#[derive(Clone, Debug)]
pub struct SysManagedReport {
    pub items: Vec<SysItemOutcome>,
    pub summary: SysUpgradeReport,
    pub lifecycle: LifecycleResultV1,
}

pub fn split_dns_receipt(request: &SplitDnsDomainRequest) -> Result<SplitDnsReceipt> {
    let domain = normalize_dns_domain(&request.domain)?;
    let servers = normalize_dns_servers(&request.servers)?;
    let resource = match request.os_id.as_str() {
        "macos" => format!("/etc/resolver/{domain}"),
        "ubuntu" => format!(
            "/etc/systemd/resolved.conf.d/shine-split-dns-{}.conf",
            request.item_id
        ),
        "windows" => format!(".{domain}"),
        other => bail!("split-dns is unsupported on `{other}`"),
    };
    Ok(SplitDnsReceipt {
        version: RECEIPT_VERSION,
        os_id: request.os_id.clone(),
        item_id: request.item_id.clone(),
        domain,
        servers,
        resource,
        content_hash: None,
    })
}

fn normalize_dns_domain(domain: &str) -> Result<String> {
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    let valid = !domain.is_empty()
        && domain.len() <= 253
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .chars()
                    .next()
                    .is_some_and(|value| value.is_ascii_alphanumeric())
                && label
                    .chars()
                    .last()
                    .is_some_and(|value| value.is_ascii_alphanumeric())
                && label
                    .chars()
                    .all(|value| value.is_ascii_alphanumeric() || value == '-')
        });
    if !valid {
        bail!("invalid private DNS domain `{domain}`");
    }
    Ok(domain)
}

fn normalize_dns_servers(servers: &str) -> Result<Vec<String>> {
    let servers = servers
        .split([',', ' ', '\t', '\n'])
        .map(str::trim)
        .filter(|server| !server.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if servers.is_empty() {
        bail!("PRIVATE_DNS_SERVERS must contain at least one server");
    }
    for server in &servers {
        server
            .parse::<std::net::IpAddr>()
            .with_context(|| format!("invalid DNS server address `{server}`"))?;
    }
    Ok(servers)
}

fn split_dns_marker(item_id: &str) -> String {
    format!("Managed by shine: split-dns:{item_id}")
}

fn split_dns_content(receipt: &SplitDnsReceipt) -> Vec<u8> {
    let marker = split_dns_marker(&receipt.item_id);
    if receipt.os_id == "windows" {
        return format!(
            "{marker}\n{}\n{}",
            receipt.resource,
            receipt.servers.join(",")
        )
        .into_bytes();
    }
    if receipt.os_id == "macos" {
        let servers = receipt
            .servers
            .iter()
            .map(|server| format!("nameserver {server}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("# {marker}\n{servers}\n").into_bytes()
    } else {
        format!(
            "# {marker}\n[Resolve]\nDNS={}\nDomains=~{}\n",
            receipt.servers.join(" "),
            receipt.domain
        )
        .into_bytes()
    }
}

fn split_dns_host_request(receipt: &SplitDnsReceipt) -> SplitDnsRequest {
    SplitDnsRequest {
        os_id: receipt.os_id.clone(),
        item_id: receipt.item_id.clone(),
        domain: receipt.domain.clone(),
        servers: receipt.servers.clone(),
        resource: PathBuf::from(&receipt.resource),
        content: split_dns_content(receipt),
    }
}

impl<H: SplitDnsHost + PrivilegedFileSystemHost> CoreRuntime<H> {
    pub(in crate::runtime) async fn split_dns_up_to_date(
        &self,
        request: &SplitDnsDomainRequest,
    ) -> Result<bool> {
        let desired = split_dns_receipt(request)?;
        let host_request = split_dns_host_request(&desired);
        let state = self.host.inspect_split_dns(&host_request).await?;
        Ok(split_dns_state_matches(&state, &host_request))
    }

    pub async fn apply_split_dns(
        &self,
        request: SplitDnsDomainRequest,
        previous: Option<&SystemReceipt>,
    ) -> Result<ResourceOutcome> {
        if let Some(previous) = previous {
            previous.ensure_supported()?;
        }
        let mut desired = split_dns_receipt(&request)?;
        if desired.os_id == "ubuntu" && !self.context.linux_split_dns_ready {
            bail!(
                "systemd-resolved's DNS stub listener (127.0.0.53) appears to be disabled on this host, likely because another service is bound to port 53 (e.g. a coredns/dnsmasq container). Split DNS routing via /etc/systemd/resolved.conf.d only takes effect when applications query that stub -- with it disabled, this change would be written but silently ineffective. Re-enable `DNSStubListener` in systemd-resolved, or point /etc/resolv.conf at whatever is actually resolving DNS on this host, before retrying."
            );
        }
        if request.dry_run {
            return Ok(ResourceOutcome {
                changed: true,
                effects: vec![LifecycleEffect::ResourceWritePreviewed],
                detail: format!("{} -> {}", desired.domain, desired.servers.join(", ")),
                receipt: Some(SystemReceipt::SplitDns(desired)),
                restart_hint: None,
            });
        }
        let _guard = self.host.acquire_privileged_operation().await?;
        let host_request = split_dns_host_request(&desired);
        let state = self.host.inspect_split_dns(&host_request).await?;
        if state.exists && !split_dns_owned(&state, &desired) {
            bail!(
                "split DNS destination {} exists but is not owned by shine",
                desired.resource
            );
        }
        let changed = !split_dns_state_matches(&state, &host_request);
        if changed {
            self.host.apply_split_dns(&host_request).await?;
        }
        if let Some(SystemReceipt::SplitDns(previous)) = previous
            && (previous.resource != desired.resource || previous.os_id != desired.os_id)
        {
            remove_split_dns_unlocked(&self.host, previous).await?;
        }
        desired.content_hash = Some(hash_content(&host_request.content));
        Ok(ResourceOutcome {
            changed,
            effects: changed
                .then_some(LifecycleEffect::ResourceWritten)
                .into_iter()
                .collect(),
            detail: format!("{} -> {}", desired.domain, desired.servers.join(", ")),
            receipt: Some(SystemReceipt::SplitDns(desired)),
            restart_hint: None,
        })
    }

    pub async fn remove_split_dns(
        &self,
        receipt: &SplitDnsReceipt,
        dry_run: bool,
    ) -> Result<ResourceOutcome> {
        remove_split_dns_with_host(&self.host, receipt, dry_run).await
    }
}

async fn remove_split_dns_with_host(
    host: &(impl SplitDnsHost + PrivilegedFileSystemHost),
    receipt: &SplitDnsReceipt,
    dry_run: bool,
) -> Result<ResourceOutcome> {
    receipt.ensure_version()?;
    if dry_run {
        return Ok(ResourceOutcome {
            changed: true,
            effects: vec![LifecycleEffect::ResourceRemovePreviewed],
            detail: format!("remove split DNS for {}", receipt.domain),
            receipt: None,
            restart_hint: None,
        });
    }
    let _guard = host.acquire_privileged_operation().await?;
    remove_split_dns_unlocked(host, receipt).await
}

async fn remove_split_dns_unlocked(
    host: &impl SplitDnsHost,
    receipt: &SplitDnsReceipt,
) -> Result<ResourceOutcome> {
    let request = split_dns_host_request(receipt);
    let state = host.inspect_split_dns(&request).await?;
    if state.exists && !split_dns_owned(&state, receipt) {
        bail!(
            "refusing to remove non-shine split DNS resource {}",
            receipt.resource
        );
    }
    if state.exists {
        host.remove_split_dns(&request).await?;
    }
    Ok(ResourceOutcome {
        changed: state.exists,
        effects: state
            .exists
            .then_some(LifecycleEffect::ResourceRemoved)
            .into_iter()
            .collect(),
        detail: format!("split DNS for {} removed", receipt.domain),
        receipt: None,
        restart_hint: None,
    })
}

impl SplitDnsReceipt {
    fn ensure_version(&self) -> Result<()> {
        if self.version != RECEIPT_VERSION {
            bail!(
                "unsupported system resource receipt version {}",
                self.version
            );
        }
        Ok(())
    }
}

fn split_dns_owned(state: &SplitDnsState, receipt: &SplitDnsReceipt) -> bool {
    String::from_utf8_lossy(&state.content).contains(&split_dns_marker(&receipt.item_id))
}

fn split_dns_state_matches(state: &SplitDnsState, request: &SplitDnsRequest) -> bool {
    state.exists && state.content == request.content
}

impl ResourceConflict {
    pub fn user_modified(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

struct ManagedFileDesired {
    destination: PathBuf,
    content: Vec<u8>,
    restart_hint: Option<String>,
}

struct PreparedSysTransaction {
    action_ir: ActionIrV1,
    receipt: SysReceiptTransitionV1,
    desired_content: Option<Vec<u8>>,
    outcome: ResourceOutcome,
}

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost,
{
    /// Core-owned managed Sys lifecycle. Selection and receipt assessment are
    /// performed once and reused for authorization, mutation and reporting.
    async fn sys_managed_up_to_date(
        &self,
        os_id: &str,
        item: &SysItem,
        previous: Option<&SystemReceipt>,
    ) -> Result<bool> {
        match item.driver {
            SysDriverKind::SplitDns => {
                let request = self.split_dns_item_request(os_id, item, false)?;
                self.split_dns_up_to_date(&request).await
            }
            SysDriverKind::ManagedFile => {
                let Some(SystemReceipt::ManagedFile(previous)) = previous else {
                    return Ok(false);
                };
                let desired = self.managed_file_desired(os_id, item)?;
                if previous.destination != desired.destination {
                    return Ok(false);
                }
                let current = match self.host.read(&desired.destination).await {
                    Ok(bytes) => bytes,
                    Err(error) if error.is_not_found() => return Ok(false),
                    Err(error) => return Err(error.into_anyhow("reading managed Sys file")),
                };
                Ok(hash_content(&current) == previous.content_hash && current == desired.content)
            }
            SysDriverKind::Script => Ok(false),
        }
    }

    async fn apply_system_item(
        &self,
        os_id: &str,
        item: &SysItem,
        previous: Option<&SystemReceipt>,
        dry_run: bool,
    ) -> Result<ResourceOutcome> {
        match item.driver {
            SysDriverKind::SplitDns => {
                self.apply_split_dns(self.split_dns_item_request(os_id, item, dry_run)?, previous)
                    .await
            }
            SysDriverKind::ManagedFile => {
                self.apply_managed_file_item(os_id, item, previous, dry_run)
                    .await
            }
            SysDriverKind::Script => bail!("script is not a built-in system resource driver"),
        }
    }

    async fn remove_system_receipt(
        &self,
        receipt: &SystemReceipt,
        dry_run: bool,
    ) -> Result<ResourceOutcome> {
        receipt.ensure_supported()?;
        match receipt {
            SystemReceipt::SplitDns(receipt) => self.remove_split_dns(receipt, dry_run).await,
            SystemReceipt::ManagedFile(receipt) => {
                self.remove_managed_file_receipt(receipt, dry_run).await
            }
            SystemReceipt::Script { .. } => {
                bail!("script receipt is not a managed system resource")
            }
        }
    }

    async fn apply_managed_file_item(
        &self,
        os_id: &str,
        item: &SysItem,
        previous: Option<&SystemReceipt>,
        dry_run: bool,
    ) -> Result<ResourceOutcome> {
        let desired = self.managed_file_desired(os_id, item)?;
        let previous = match previous {
            Some(SystemReceipt::ManagedFile(receipt)) => Some(receipt),
            Some(other) => bail!("managed-file received {:?} receipt", other.driver()),
            None => None,
        };
        if dry_run {
            return Ok(ResourceOutcome {
                changed: true,
                effects: vec![LifecycleEffect::ResourceWritePreviewed],
                detail: desired.destination.display().to_string(),
                receipt: None,
                restart_hint: desired.restart_hint,
            });
        }
        let mut effects = Vec::new();
        if let Some(previous) = previous {
            if previous.destination != desired.destination {
                effects.extend(
                    self.remove_managed_file_receipt(previous, false)
                        .await?
                        .effects,
                );
            } else if let Ok(current) = self.host.read(&desired.destination).await {
                if hash_content(&current) != previous.content_hash {
                    return Err(ResourceConflict::user_modified(format!(
                        "managed file {} was modified; keeping user content",
                        desired.destination.display()
                    ))
                    .into());
                }
                if current == desired.content {
                    return Ok(ResourceOutcome {
                        changed: false,
                        effects: Vec::new(),
                        detail: desired.destination.display().to_string(),
                        receipt: Some(SystemReceipt::ManagedFile(previous.clone())),
                        restart_hint: None,
                    });
                }
            }
        }
        let is_managed = previous.is_some_and(|receipt| receipt.destination == desired.destination);
        let outcome = install_sys_bytes(
            &self.host,
            &desired.content,
            &desired.destination,
            is_managed,
            item.requires_admin,
        )
        .await?;
        let backup = match outcome {
            InstallOutcome::BackedUpAndInstalled { ref backup, .. } => {
                effects.push(LifecycleEffect::BackupCreated);
                Some(backup.clone())
            }
            _ => previous.and_then(|receipt| receipt.backup.clone()),
        };
        let changed = !matches!(outcome, InstallOutcome::AlreadyManaged);
        if changed {
            effects.push(LifecycleEffect::ResourceWritten);
        }
        Ok(ResourceOutcome {
            changed,
            effects,
            detail: desired.destination.display().to_string(),
            receipt: Some(SystemReceipt::ManagedFile(ManagedFileReceipt {
                version: RECEIPT_VERSION,
                destination: desired.destination,
                backup,
                content_hash: hash_content(&desired.content),
                privileged: item.requires_admin,
                restart_hint: desired.restart_hint.clone(),
            })),
            restart_hint: desired.restart_hint,
        })
    }

    async fn remove_managed_file_receipt(
        &self,
        receipt: &ManagedFileReceipt,
        dry_run: bool,
    ) -> Result<ResourceOutcome> {
        if dry_run {
            return Ok(ResourceOutcome {
                changed: true,
                effects: vec![LifecycleEffect::ResourceRemovePreviewed],
                detail: receipt.destination.display().to_string(),
                receipt: None,
                restart_hint: receipt.restart_hint.clone(),
            });
        }
        let outcome = uninstall_sys_entry(
            &self.host,
            &app_entry_from_receipt(receipt),
            receipt.privileged,
        )
        .await?;
        if matches!(outcome, UninstallOutcome::UserModified) {
            return Err(ResourceConflict::user_modified(format!(
                "managed file {} was modified; keeping user content",
                receipt.destination.display()
            ))
            .into());
        }
        let changed = !matches!(outcome, UninstallOutcome::NotFound);
        let effects = match outcome {
            UninstallOutcome::Removed | UninstallOutcome::ForceRemoved => {
                vec![LifecycleEffect::ResourceRemoved]
            }
            UninstallOutcome::RestoredBackup { .. }
            | UninstallOutcome::ForceRestoredBackup { .. } => vec![LifecycleEffect::BackupRestored],
            _ => Vec::new(),
        };
        Ok(ResourceOutcome {
            changed,
            effects,
            detail: receipt.destination.display().to_string(),
            receipt: None,
            restart_hint: receipt.restart_hint.clone(),
        })
    }
}

impl<H> CoreRuntime<H> {
    fn split_dns_item_request(
        &self,
        os_id: &str,
        item: &SysItem,
        dry_run: bool,
    ) -> Result<SplitDnsDomainRequest> {
        let domain_key = sys_config_string(&item.config, "domain_env")?;
        let servers_key = sys_config_string(&item.config, "servers_env")?;
        Ok(SplitDnsDomainRequest {
            os_id: os_id.to_string(),
            item_id: item.id.clone(),
            domain: self
                .context
                .env
                .get(&domain_key)
                .cloned()
                .with_context(|| format!("missing environment variable `{domain_key}`"))?,
            servers: self
                .context
                .env
                .get(&servers_key)
                .cloned()
                .with_context(|| format!("missing environment variable `{servers_key}`"))?,
            dry_run,
        })
    }

    fn managed_file_desired(&self, os_id: &str, item: &SysItem) -> Result<ManagedFileDesired> {
        let source = safe_managed_source(&sys_config_string(&item.config, "source")?)?;
        let logical = format!(
            "sys/{os_id}/{}",
            source
                .components()
                .map(|part| part.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/")
        );
        let raw = self
            .presets
            .get(&logical)
            .with_context(|| format!("reading {logical}"))?;
        let destination = captured_sys_path(
            &sys_config_string(&item.config, "target")?,
            &self.context.home_dir,
        )?;
        let transforms = item
            .config
            .get("transforms")
            .and_then(toml::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::to_string)
                            .context("managed-file transforms must be strings")
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
        let content = crate::install::apply_transforms(&transforms, raw, &self.context.env)?;
        Ok(ManagedFileDesired {
            destination,
            content,
            restart_hint: item
                .config
                .get("restart_hint")
                .and_then(toml::Value::as_str)
                .map(str::to_string),
        })
    }
}

fn sys_config_string(config: &toml::Table, key: &str) -> Result<String> {
    config
        .get(key)
        .and_then(toml::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .with_context(|| format!("driver config requires non-empty `{key}`"))
}

fn safe_managed_source(value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        bail!("managed-file source must stay within its preset directory");
    }
    Ok(path.to_path_buf())
}

fn captured_sys_path(value: &str, home: &Path) -> Result<PathBuf> {
    let expanded = if value == "~" || value == "$HOME" {
        home.to_path_buf()
    } else if let Some(rest) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("$HOME/"))
    {
        home.join(rest)
    } else {
        PathBuf::from(value)
    };
    if !expanded.is_absolute() {
        bail!("managed-file target must resolve to an absolute path");
    }
    Ok(expanded)
}

async fn install_sys_bytes<H>(
    host: &H,
    content: &[u8],
    destination: &Path,
    is_managed: bool,
    privileged: bool,
) -> Result<InstallOutcome>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    if !privileged {
        return install_bytes_with_host(host, content, destination, is_managed, false, true).await;
    }
    let _guard = host.acquire_privileged_operation().await?;
    let hash = hash_content(content);
    let exists = match host.metadata(destination).await {
        Ok(_) => true,
        Err(error) if error.is_not_found() => false,
        Err(error) => return Err(error.into_anyhow("inspecting privileged managed Sys file")),
    };
    if exists && is_managed {
        let current = host
            .read(destination)
            .await
            .map_err(|error| error.into_anyhow("reading privileged managed Sys file"))?;
        if hash_content(&current) == hash {
            return Ok(InstallOutcome::AlreadyManaged);
        }
    }
    let backup = if exists && !is_managed {
        let backup = crate::install::file_ops::backup_path(destination);
        host.move_privileged(destination, &backup).await?;
        Some(backup)
    } else {
        None
    };
    if let Err(error) = host.write_privileged(destination, content).await {
        if let Some(backup) = &backup {
            let _ = host.move_privileged(backup, destination).await;
        }
        return Err(error);
    }
    Ok(match backup {
        Some(backup) => InstallOutcome::BackedUpAndInstalled { backup, hash },
        None => InstallOutcome::Installed { hash },
    })
}

async fn uninstall_sys_entry<H>(
    host: &H,
    entry: &AppEntry,
    privileged: bool,
) -> Result<UninstallOutcome>
where
    H: FileSystemHost + PrivilegedFileSystemHost,
{
    if !privileged {
        return uninstall_entry_with_host(host, entry, false, false).await;
    }
    let _guard = host.acquire_privileged_operation().await?;
    let current = match host.read(&entry.destination).await {
        Ok(bytes) => bytes,
        Err(error) if error.is_not_found() => return Ok(UninstallOutcome::NotFound),
        Err(error) => return Err(error.into_anyhow("reading privileged managed Sys file")),
    };
    if hash_content(&current) != entry.content_hash {
        return Ok(UninstallOutcome::UserModified);
    }
    host.remove_privileged(&entry.destination).await?;
    if let Some(backup) = &entry.backup
        && host.metadata(backup).await.is_ok()
    {
        host.move_privileged(backup, &entry.destination).await?;
        return Ok(UninstallOutcome::RestoredBackup {
            backup: backup.clone(),
        });
    }
    Ok(UninstallOutcome::Removed)
}

impl fmt::Display for ResourceConflict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ResourceConflict {}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SysRunManifest {
    #[serde(default = "legacy_manifest_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub entries: Vec<SysRunEntry>,
}

fn legacy_manifest_schema_version() -> u32 {
    0
}

impl Default for SysRunManifest {
    fn default() -> Self {
        Self {
            schema_version: SYS_MANIFEST_SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SysRunEntry {
    pub os_id: String,
    pub item_id: String,
    pub label: String,
    pub status: SysItemStatus,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    pub updated_at: String,
    #[serde(default)]
    pub managed: bool,
    #[serde(default = "default_profile_enabled")]
    pub profile_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<SystemReceipt>,
}

fn default_profile_enabled() -> bool {
    true
}

impl SysRunManifest {
    pub async fn load(
        host: &impl crate::runtime::FileSystemObservationHost,
        shine_dir: &Path,
    ) -> Result<Self> {
        load_manifest_with_host(host, shine_dir).await
    }

    pub async fn save(&self, host: &impl FileSystemHost, shine_dir: &Path) -> Result<()> {
        save_manifest_with_host(host, shine_dir, self).await
    }

    pub fn upsert(&mut self, entry: SysRunEntry) {
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|existing| existing.os_id == entry.os_id && existing.item_id == entry.item_id)
        {
            *existing = entry;
        } else {
            self.entries.push(entry);
        }
    }
}

fn app_entry_from_receipt(receipt: &ManagedFileReceipt) -> AppEntry {
    AppEntry {
        source: "sys/managed-file".into(),
        destination: receipt.destination.clone(),
        backup: receipt.backup.clone(),
        content_hash: receipt.content_hash,
        install_strategy: AppInstallStrategy::Copy,
        uses_env: false,
        requires_admin: receipt.privileged,
    }
}

fn split_dns_action_state(receipt: &SplitDnsReceipt) -> Result<SysSplitDnsStateV1> {
    let content_hash = receipt
        .content_hash
        .unwrap_or_else(|| hash_content(&split_dns_content(receipt)));
    Ok(SysSplitDnsStateV1 {
        os_id: receipt.os_id.clone(),
        item_id: receipt.item_id.clone(),
        domain: receipt.domain.clone(),
        servers: receipt.servers.clone(),
        resource: PathBuf::from(&receipt.resource),
        content_hash,
    })
}

async fn read_regular_sys_file(
    host: &impl crate::runtime::FileSystemObservationHost,
    path: &Path,
    context: &'static str,
) -> Result<Option<(Vec<u8>, Option<u32>)>> {
    let metadata = match host.metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.is_not_found() => return Ok(None),
        Err(error) => return Err(error.into_anyhow(context)),
    };
    if metadata.kind != crate::runtime::FileKind::File {
        bail!("managed Sys path {} is not a regular file", path.display());
    }
    let bytes = host
        .read(path)
        .await
        .map_err(|error| error.into_anyhow(context))?;
    Ok(Some((bytes, metadata.unix_mode)))
}

pub(crate) async fn load_manifest_with_host(
    host: &impl crate::runtime::FileSystemObservationHost,
    shine_dir: &Path,
) -> Result<SysRunManifest> {
    let path = shine_dir.join(SYS_MANIFEST_FILE);
    let mut manifest = match host.read(&path).await {
        Ok(bytes) => toml::from_slice(&bytes).context("failed to parse sys manifest")?,
        Err(error) if error.is_not_found() => SysRunManifest::default(),
        Err(error) => return Err(error.into_anyhow("failed to read sys manifest")),
    };
    match manifest.schema_version {
        0 => manifest.schema_version = SYS_MANIFEST_SCHEMA_VERSION,
        SYS_MANIFEST_SCHEMA_VERSION => {}
        version => bail!(
            "sys manifest schema version {version} is newer than this Shine supports ({SYS_MANIFEST_SCHEMA_VERSION})"
        ),
    }
    Ok(manifest)
}

pub(crate) async fn save_manifest_with_host(
    host: &impl FileSystemHost,
    shine_dir: &Path,
    manifest: &SysRunManifest,
) -> Result<()> {
    if manifest.schema_version != SYS_MANIFEST_SCHEMA_VERSION {
        bail!(
            "cannot write sys manifest schema version {}; expected {SYS_MANIFEST_SCHEMA_VERSION}",
            manifest.schema_version
        );
    }
    let content = toml::to_string_pretty(manifest).context("failed to serialize sys manifest")?;
    host.write_atomic(&shine_dir.join(SYS_MANIFEST_FILE), content.as_bytes())
        .await
        .map_err(|error| error.into_anyhow("failed to write sys manifest"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{
        InMemoryHost, PresetSnapshot, PresetSourceKind, RealHost, RuntimeContext, RuntimePlatform,
    };

    #[tokio::test]
    async fn manifest_and_receipt_versions_are_owned_by_core() {
        let root =
            std::env::temp_dir().join(format!("shine-sys-manifest-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let path = root.join(SYS_MANIFEST_FILE);
        tokio::fs::write(&path, "entries = []\n").await.unwrap();
        let legacy = SysRunManifest::load(&RealHost, &root).await.unwrap();
        assert_eq!(legacy.schema_version, SYS_MANIFEST_SCHEMA_VERSION);
        legacy.save(&RealHost, &root).await.unwrap();

        let receipt = SystemReceipt::ManagedFile(ManagedFileReceipt {
            version: RECEIPT_VERSION,
            destination: PathBuf::from("/managed"),
            backup: None,
            content_hash: 1,
            privileged: false,
            restart_hint: None,
        });
        receipt.ensure_supported().unwrap();
        assert_eq!(receipt.driver(), SysDriverKind::ManagedFile);
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn split_dns_entry_owns_exactly_one_privileged_operation_lock() {
        let runtime = CoreRuntime::new(
            InMemoryHost::new(),
            RuntimeContext::isolated(
                PathBuf::from("/home/test"),
                PathBuf::from("/home/test/.shine"),
                PathBuf::from("/presets"),
                PathBuf::from("/bin"),
                RuntimePlatform::Macos,
            ),
            PresetSnapshot::builder(PresetSourceKind::Embedded).build(),
        );

        runtime
            .apply_split_dns(
                SplitDnsDomainRequest {
                    os_id: "macos".to_string(),
                    item_id: "corp".to_string(),
                    domain: "corp.example".to_string(),
                    servers: "10.0.0.53".to_string(),
                    dry_run: false,
                },
                None,
            )
            .await
            .unwrap();

        assert_eq!(
            runtime
                .host()
                .operations()
                .iter()
                .filter(|operation| matches!(
                    operation,
                    crate::runtime::HostOperation::AcquirePrivilegedOperation
                ))
                .count(),
            1
        );
    }
}
