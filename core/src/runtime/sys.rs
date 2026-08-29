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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourcePlan {
    pub description: String,
    pub requires_admin: bool,
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

#[derive(Clone, Debug)]
pub struct ManagedFileRequest {
    pub os_id: String,
    pub item_id: String,
    pub label: String,
    pub destination: PathBuf,
    pub content: Vec<u8>,
    pub privileged: bool,
    pub restart_hint: Option<String>,
    pub dry_run: bool,
}

#[derive(Clone, Debug)]
pub struct ManagedFileRemoveRequest {
    pub os_id: String,
    pub item_id: String,
    pub dry_run: bool,
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

impl<H: SplitDnsHost> CoreRuntime<H> {
    pub async fn split_dns_up_to_date(&self, request: &SplitDnsDomainRequest) -> Result<bool> {
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
            self.remove_split_dns(previous, false).await?;
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

pub async fn remove_split_dns_with_host(
    host: &impl SplitDnsHost,
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

impl<H> CoreRuntime<H>
where
    H: FileSystemHost + PrivilegedFileSystemHost + SplitDnsHost,
{
    pub async fn installed_managed_sys(&self, os_id: &str) -> Result<Vec<SysInstalledRow>> {
        let manifest = load_manifest_with_host(&self.host, &self.context.shine_dir).await?;
        let mut rows = manifest
            .entries
            .into_iter()
            .filter(|entry| entry.os_id == os_id && entry.managed)
            .map(|entry| SysInstalledRow {
                item_id: entry.item_id,
                label: entry.label,
            })
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| {
            left.label
                .cmp(&right.label)
                .then_with(|| left.item_id.cmp(&right.item_id))
        });
        Ok(rows)
    }

    /// Inspect recorded managed resources against the same desired-state
    /// assessment used by mutation, without executing a host operation.
    pub async fn inspect_managed_sys(
        &self,
        os_id: &str,
    ) -> Result<(Vec<SysInstalledRow>, Vec<SysUpdateRow>, LifecycleResultV1)> {
        let manifest = load_manifest_with_host(&self.host, &self.context.shine_dir).await?;
        let recorded = manifest
            .entries
            .iter()
            .filter(|entry| entry.os_id == os_id && entry.managed)
            .collect::<Vec<_>>();
        let mut installed = recorded
            .iter()
            .map(|entry| SysInstalledRow {
                item_id: entry.item_id.clone(),
                label: entry.label.clone(),
            })
            .collect::<Vec<_>>();
        installed.sort_by(|left, right| {
            left.label
                .cmp(&right.label)
                .then_with(|| left.item_id.cmp(&right.item_id))
        });
        let mut lifecycle = LifecycleResultV1::new(LifecycleOperation::Update, false);
        if recorded.is_empty() {
            return Ok((installed, Vec::new(), lifecycle));
        }
        let loaded = self.load_sys_preset(os_id).await?;
        let mut updates = Vec::new();
        for entry in recorded {
            let Some(item) = loaded.manifest.items.iter().find(|item| {
                item.id == entry.item_id
                    && item.mode == SysItemMode::Managed
                    && item.driver != SysDriverKind::Script
            }) else {
                lifecycle.push(
                    LifecycleOutcomeV1::new(
                        format!("sys/{}", entry.item_id),
                        None::<String>,
                        LifecycleStatus::Skipped,
                        [],
                    )
                    .with_diagnostic_code("sys_managed_item_unavailable"),
                );
                continue;
            };
            let details = self.managed_update_details(os_id, item, entry.receipt.as_ref())?;
            if details.is_empty() {
                lifecycle.push(LifecycleOutcomeV1::new(
                    format!("sys/{}", item.id),
                    None::<String>,
                    LifecycleStatus::Unchanged,
                    [],
                ));
            } else {
                lifecycle.push(LifecycleOutcomeV1::new(
                    format!("sys/{}", item.id),
                    None::<String>,
                    LifecycleStatus::Pending,
                    [
                        LifecycleEffect::ResourceWritePreviewed,
                        LifecycleEffect::ReceiptWritePreviewed,
                    ],
                ));
                updates.push(SysUpdateRow {
                    item_id: item.id.clone(),
                    label: item.label.clone(),
                    details,
                });
            }
        }
        Ok((installed, updates, lifecycle))
    }

    fn managed_update_details(
        &self,
        os_id: &str,
        item: &SysItem,
        previous: Option<&SystemReceipt>,
    ) -> Result<Vec<String>> {
        match item.driver {
            SysDriverKind::SplitDns => {
                let desired = split_dns_receipt(&self.split_dns_item_request(os_id, item, true)?)?;
                let Some(SystemReceipt::SplitDns(previous)) = previous else {
                    return Ok(vec![
                        "Receipt: missing or incompatible -> desired split DNS state".to_string(),
                    ]);
                };
                let mut details = Vec::new();
                if previous.domain != desired.domain {
                    details.push(format!("Domain: {} -> {}", previous.domain, desired.domain));
                }
                if previous.servers != desired.servers {
                    details.push(format!(
                        "Servers: {} -> {}",
                        previous.servers.join(", "),
                        desired.servers.join(", ")
                    ));
                }
                if previous.resource != desired.resource || previous.os_id != desired.os_id {
                    details.push(format!(
                        "Resource: {} -> {}",
                        previous.resource, desired.resource
                    ));
                }
                if previous.item_id != desired.item_id {
                    details.push(format!("Item: {} -> {}", previous.item_id, desired.item_id));
                }
                Ok(details)
            }
            SysDriverKind::ManagedFile => {
                let desired = self.managed_file_desired(os_id, item)?;
                let Some(SystemReceipt::ManagedFile(previous)) = previous else {
                    return Ok(vec![
                        "Receipt: missing or incompatible -> desired managed file state"
                            .to_string(),
                    ]);
                };
                let mut details = Vec::new();
                if previous.destination != desired.destination {
                    details.push("Destination: changed".to_string());
                }
                if previous.content_hash != hash_content(&desired.content) {
                    details.push("Content: changed".to_string());
                }
                Ok(details)
            }
            SysDriverKind::Script => Ok(Vec::new()),
        }
    }

    /// Core-owned managed Sys lifecycle. Selection and receipt assessment are
    /// performed once and reused for authorization, mutation and reporting.
    pub async fn run_managed_sys(
        &self,
        request: SysManagedRequest,
        interaction: &mut impl RuntimeInteraction,
        observer: &mut impl RuntimeObserver,
    ) -> Result<SysManagedReport> {
        let mut manifest = load_manifest_with_host(&self.host, &self.context.shine_dir).await?;
        let loaded = self.load_sys_preset(&request.os_id).await;
        let available = loaded.as_ref().ok().map(|loaded| &loaded.manifest.items);
        let enabled = manifest
            .entries
            .iter()
            .filter(|entry| entry.os_id == request.os_id && entry.managed)
            .map(|entry| entry.item_id.clone())
            .collect::<BTreeSet<_>>();
        let selected = if let Some(target) = &request.target {
            if let Some(item) =
                available.and_then(|items| items.iter().find(|item| item.id == *target))
            {
                if item.mode != SysItemMode::Managed {
                    bail!("sys item `{target}` is not managed and cannot be reapplied");
                }
                vec![item.clone()]
            } else if request.action == SysManagedAction::Remove
                && manifest.entries.iter().any(|entry| {
                    entry.os_id == request.os_id
                        && entry.item_id == *target
                        && entry.receipt.is_some()
                })
            {
                Vec::new()
            } else {
                bail!("unknown sys item `{target}`");
            }
        } else {
            available
                .map(|items| {
                    items
                        .iter()
                        .filter(|item| {
                            item.mode == SysItemMode::Managed && enabled.contains(&item.id)
                        })
                        .cloned()
                        .collect()
                })
                .unwrap_or_default()
        };
        let lifecycle = LifecycleResultV1::new(request.operation, request.dry_run);
        let mut report = SysManagedReport {
            items: Vec::new(),
            summary: SysUpgradeReport::default(),
            lifecycle,
        };

        // Recorded built-in resources remain removable without their original preset.
        if selected.is_empty()
            && request.action == SysManagedAction::Remove
            && let Some(target) = &request.target
            && let Some(entry) = manifest
                .entries
                .iter()
                .find(|entry| entry.os_id == request.os_id && entry.item_id == *target)
                .cloned()
            && let Some(receipt) = entry.receipt.clone()
        {
            observer.emit(RuntimeEvent::Progress {
                code: "sys_managed_item",
                target: target.clone(),
            });
            if receipt.requires_admin()
                && !request.dry_run
                && !self.context.running_as_admin
                && !interaction.authorize_admin(1).await?
            {
                report.summary.failed = 1;
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        format!("sys/{target}"),
                        None::<String>,
                        LifecycleStatus::Failed,
                        [],
                    )
                    .with_diagnostic_code("sys_admin_not_authorized"),
                );
                return Ok(report);
            }
            let outcome = self.remove_system_receipt(&receipt, request.dry_run).await;
            self.record_managed_resource_result(
                &request,
                None,
                &entry.label,
                outcome,
                &mut manifest,
                &mut report,
            )
            .await?;
            return Ok(report);
        }
        if selected.is_empty() {
            return Ok(report);
        }

        let mut missing = BTreeMap::<String, Vec<String>>::new();
        for item in &selected {
            let keys = item
                .required_env
                .iter()
                .filter(|key| {
                    self.context
                        .env
                        .get(*key)
                        .is_none_or(|value| value.trim().is_empty())
                })
                .cloned()
                .collect::<Vec<_>>();
            if !keys.is_empty() {
                missing.insert(item.id.clone(), keys);
            }
        }
        let mut needs_admin = 0usize;
        if !request.dry_run {
            for item in &selected {
                if missing.contains_key(&item.id) {
                    continue;
                }
                let previous = manifest
                    .entries
                    .iter()
                    .find(|entry| entry.os_id == request.os_id && entry.item_id == item.id)
                    .and_then(|entry| entry.receipt.as_ref());
                let changes = match request.action {
                    SysManagedAction::Apply => !self
                        .sys_managed_up_to_date(&request.os_id, item, previous)
                        .await
                        .unwrap_or(false),
                    SysManagedAction::Remove => previous.is_some(),
                };
                if changes
                    && (item.requires_admin || previous.is_some_and(SystemReceipt::requires_admin))
                {
                    needs_admin += 1;
                }
            }
        }
        if needs_admin > 0 {
            for item in &selected {
                observer.emit(RuntimeEvent::Progress {
                    code: "sys_managed_item",
                    target: item.label.clone(),
                });
            }
        }
        if needs_admin > 0
            && !self.context.running_as_admin
            && !interaction.authorize_admin(needs_admin).await?
        {
            for item in &selected {
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        format!("sys/{}", item.id),
                        None::<String>,
                        LifecycleStatus::Failed,
                        [],
                    )
                    .with_diagnostic_code("sys_admin_not_authorized"),
                );
            }
            report.summary.failed = selected.len();
            return Ok(report);
        }

        for item in &selected {
            if needs_admin == 0 {
                observer.emit(RuntimeEvent::Progress {
                    code: "sys_managed_item",
                    target: item.label.clone(),
                });
            }
            if let Some(keys) = missing.get(&item.id) {
                let outcome = SysItemOutcome {
                    item_id: item.id.clone(),
                    label: item.label.clone(),
                    status: SysItemStatus::Failed,
                    detail: format!("missing environment variable(s): {}", keys.join(", ")),
                    logs: Vec::new(),
                };
                report.items.push(outcome);
                report.summary.failed += 1;
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        format!("sys/{}", item.id),
                        None::<String>,
                        LifecycleStatus::Failed,
                        [],
                    )
                    .with_diagnostic_code("sys_missing_required_env"),
                );
                continue;
            }
            let previous = manifest
                .entries
                .iter()
                .find(|entry| entry.os_id == request.os_id && entry.item_id == item.id)
                .and_then(|entry| entry.receipt.as_ref())
                .cloned();
            let resource = match request.action {
                SysManagedAction::Apply => {
                    self.apply_system_item(&request.os_id, item, previous.as_ref(), request.dry_run)
                        .await
                }
                SysManagedAction::Remove => match previous.as_ref() {
                    Some(receipt) => self.remove_system_receipt(receipt, request.dry_run).await,
                    None => Err(anyhow::anyhow!(
                        "managed item `{}` has no receipt to remove",
                        item.id
                    )),
                },
            };
            self.record_managed_resource_result(
                &request,
                Some(item),
                &item.label,
                resource,
                &mut manifest,
                &mut report,
            )
            .await?;
        }
        Ok(report)
    }

    async fn record_managed_resource_result(
        &self,
        request: &SysManagedRequest,
        item: Option<&SysItem>,
        label: &str,
        resource: Result<ResourceOutcome>,
        manifest: &mut SysRunManifest,
        report: &mut SysManagedReport,
    ) -> Result<()> {
        let item_id = item
            .map(|item| item.id.as_str())
            .or(request.target.as_deref())
            .context("managed Sys item id")?;
        match resource {
            Ok(resource) => {
                let mut effects = resource.effects.clone();
                let status = if request.dry_run {
                    LifecycleStatus::Previewed
                } else if resource.changed {
                    LifecycleStatus::Changed
                } else {
                    LifecycleStatus::Unchanged
                };
                if request.action == SysManagedAction::Remove {
                    effects.push(if request.dry_run {
                        LifecycleEffect::ReceiptRemovePreviewed
                    } else {
                        LifecycleEffect::ReceiptRemoved
                    });
                } else if resource.changed {
                    effects.push(if request.dry_run {
                        LifecycleEffect::ReceiptWritePreviewed
                    } else {
                        LifecycleEffect::ReceiptWritten
                    });
                }
                report.lifecycle.push(LifecycleOutcomeV1::new(
                    format!("sys/{item_id}"),
                    None::<String>,
                    status,
                    effects,
                ));
                let item_status = if resource.changed {
                    SysItemStatus::Updated
                } else {
                    SysItemStatus::AlreadyInstalled
                };
                report.items.push(SysItemOutcome {
                    item_id: item_id.to_string(),
                    label: label.to_string(),
                    status: item_status,
                    detail: resource.detail.clone(),
                    logs: Vec::new(),
                });
                if resource.changed && !request.dry_run {
                    report.summary.updated += 1;
                } else {
                    report.summary.skipped += 1;
                }
                if !request.dry_run {
                    if request.action == SysManagedAction::Remove {
                        manifest.entries.retain(|entry| {
                            !(entry.os_id == request.os_id && entry.item_id == item_id)
                        });
                    } else if resource.changed {
                        manifest.upsert(SysRunEntry {
                            os_id: request.os_id.clone(),
                            item_id: item_id.to_string(),
                            label: label.to_string(),
                            status: item_status,
                            detail: resource.detail,
                            updated_at: self.context.captured_unix_time.to_string(),
                            managed: true,
                            profile_enabled: false,
                            receipt: resource.receipt,
                        });
                    }
                    save_manifest_with_host(&self.host, &self.context.shine_dir, manifest).await?;
                }
            }
            Err(error) => {
                let user_modified = error.downcast_ref::<ResourceConflict>().is_some();
                report.items.push(SysItemOutcome {
                    item_id: item_id.to_string(),
                    label: label.to_string(),
                    status: SysItemStatus::Failed,
                    detail: format!("{error:#}"),
                    logs: Vec::new(),
                });
                report.summary.failed += 1;
                report.lifecycle.push(
                    LifecycleOutcomeV1::new(
                        format!("sys/{item_id}"),
                        None::<String>,
                        if user_modified {
                            LifecycleStatus::Preserved
                        } else {
                            LifecycleStatus::Failed
                        },
                        if user_modified {
                            vec![LifecycleEffect::UserResourcePreserved]
                        } else {
                            Vec::new()
                        },
                    )
                    .with_diagnostic_code(if user_modified {
                        "sys_resource_user_modified"
                    } else if request.action == SysManagedAction::Remove {
                        "sys_remove_failed"
                    } else {
                        "sys_apply_failed"
                    }),
                );
            }
        }
        Ok(())
    }

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
    pub async fn load(host: &impl FileSystemHost, shine_dir: &Path) -> Result<Self> {
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

impl<H: FileSystemHost> CoreRuntime<H> {
    pub async fn apply_managed_sys_file(
        &self,
        request: ManagedFileRequest,
    ) -> Result<LifecycleResultV1> {
        if request.privileged {
            bail!("managed Sys file requires a privileged host capability");
        }
        let mut manifest = load_manifest_with_host(&self.host, &self.context.shine_dir).await?;
        let previous = manifest
            .entries
            .iter()
            .find(|entry| entry.os_id == request.os_id && entry.item_id == request.item_id);
        let previous_receipt = previous.and_then(|entry| match &entry.receipt {
            Some(SystemReceipt::ManagedFile(receipt)) => Some(receipt),
            _ => None,
        });
        if let Some(receipt) = previous_receipt
            && receipt.destination != request.destination
        {
            let entry = app_entry_from_receipt(receipt);
            if matches!(
                uninstall_entry_with_host(&self.host, &entry, request.dry_run, false).await?,
                UninstallOutcome::UserModified
            ) {
                let mut result =
                    LifecycleResultV1::new(LifecycleOperation::Upgrade, request.dry_run);
                result.push(
                    LifecycleOutcomeV1::new(
                        format!("sys/{}", request.item_id),
                        None::<String>,
                        LifecycleStatus::Conflict,
                        [LifecycleEffect::UserResourcePreserved],
                    )
                    .with_diagnostic_code("sys_user_modified"),
                );
                return Ok(result);
            }
        }
        let is_managed =
            previous_receipt.is_some_and(|receipt| receipt.destination == request.destination);
        let outcome = install_bytes_with_host(
            &self.host,
            &request.content,
            &request.destination,
            is_managed,
            request.dry_run,
            true,
        )
        .await?;
        let (status, mut effects, backup) = match outcome {
            InstallOutcome::Installed { .. } => (
                LifecycleStatus::Changed,
                vec![LifecycleEffect::ResourceWritten],
                previous_receipt.and_then(|receipt| receipt.backup.clone()),
            ),
            InstallOutcome::BackedUpAndInstalled { backup, .. } => (
                LifecycleStatus::Changed,
                vec![
                    LifecycleEffect::BackupCreated,
                    LifecycleEffect::ResourceWritten,
                ],
                Some(backup),
            ),
            InstallOutcome::AlreadyManaged => (
                LifecycleStatus::Unchanged,
                Vec::new(),
                previous_receipt.and_then(|receipt| receipt.backup.clone()),
            ),
            InstallOutcome::DryRun => (
                LifecycleStatus::Previewed,
                vec![
                    LifecycleEffect::ResourceWritePreviewed,
                    LifecycleEffect::ReceiptWritePreviewed,
                ],
                None,
            ),
        };
        if !request.dry_run {
            manifest.upsert(SysRunEntry {
                os_id: request.os_id,
                item_id: request.item_id.clone(),
                label: request.label,
                status: if status == LifecycleStatus::Unchanged {
                    SysItemStatus::AlreadyInstalled
                } else {
                    SysItemStatus::Updated
                },
                detail: request.destination.display().to_string(),
                updated_at: String::new(),
                managed: true,
                profile_enabled: true,
                receipt: Some(SystemReceipt::ManagedFile(ManagedFileReceipt {
                    version: RECEIPT_VERSION,
                    destination: request.destination,
                    backup,
                    content_hash: hash_content(&request.content),
                    privileged: request.privileged,
                    restart_hint: request.restart_hint,
                })),
            });
            effects.push(LifecycleEffect::ReceiptWritten);
            save_manifest_with_host(&self.host, &self.context.shine_dir, &manifest).await?;
        }
        let mut result = LifecycleResultV1::new(LifecycleOperation::Upgrade, request.dry_run);
        result.push(LifecycleOutcomeV1::new(
            format!("sys/{}", request.item_id),
            None::<String>,
            status,
            effects,
        ));
        Ok(result)
    }

    pub async fn remove_managed_sys_file(
        &self,
        request: ManagedFileRemoveRequest,
    ) -> Result<LifecycleResultV1> {
        let mut manifest = load_manifest_with_host(&self.host, &self.context.shine_dir).await?;
        let position = manifest
            .entries
            .iter()
            .position(|entry| entry.os_id == request.os_id && entry.item_id == request.item_id);
        let mut result = LifecycleResultV1::new(LifecycleOperation::Uninstall, request.dry_run);
        let Some(position) = position else {
            return Ok(result);
        };
        let entry = manifest.entries[position].clone();
        let Some(SystemReceipt::ManagedFile(receipt)) = entry.receipt else {
            return Ok(result);
        };
        if receipt.privileged {
            bail!("managed Sys file requires a privileged host capability");
        }
        let outcome = uninstall_entry_with_host(
            &self.host,
            &app_entry_from_receipt(&receipt),
            request.dry_run,
            false,
        )
        .await?;
        let (status, effects, remove_receipt) = match outcome {
            UninstallOutcome::UserModified => (
                LifecycleStatus::Conflict,
                vec![LifecycleEffect::UserResourcePreserved],
                false,
            ),
            UninstallOutcome::DryRun => (
                LifecycleStatus::Previewed,
                vec![
                    LifecycleEffect::ResourceRemovePreviewed,
                    LifecycleEffect::ReceiptRemovePreviewed,
                ],
                false,
            ),
            UninstallOutcome::RestoredBackup { .. }
            | UninstallOutcome::ForceRestoredBackup { .. } => (
                LifecycleStatus::Changed,
                vec![
                    LifecycleEffect::BackupRestored,
                    LifecycleEffect::ReceiptRemoved,
                ],
                true,
            ),
            UninstallOutcome::Removed | UninstallOutcome::ForceRemoved => (
                LifecycleStatus::Changed,
                vec![
                    LifecycleEffect::ResourceRemoved,
                    LifecycleEffect::ReceiptRemoved,
                ],
                true,
            ),
            UninstallOutcome::NotFound => (
                LifecycleStatus::Changed,
                vec![LifecycleEffect::ReceiptRemoved],
                true,
            ),
        };
        if remove_receipt {
            manifest.entries.remove(position);
            save_manifest_with_host(&self.host, &self.context.shine_dir, &manifest).await?;
        }
        result.push(LifecycleOutcomeV1::new(
            format!("sys/{}", request.item_id),
            None::<String>,
            status,
            effects,
        ));
        Ok(result)
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

pub(crate) async fn load_manifest_with_host(
    host: &impl FileSystemHost,
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
    async fn managed_file_roundtrip_uses_virtual_host_and_receipt() {
        let runtime = CoreRuntime::new(
            InMemoryHost::new(),
            RuntimeContext::isolated(
                PathBuf::from("/home/test"),
                PathBuf::from("/home/test/.shine"),
                PathBuf::from("/presets"),
                PathBuf::from("/bin"),
                RuntimePlatform::Linux,
            ),
            PresetSnapshot::builder(PresetSourceKind::External).build(),
        );
        let applied = runtime
            .apply_managed_sys_file(ManagedFileRequest {
                os_id: "linux".into(),
                item_id: "managed".into(),
                label: "Managed".into(),
                destination: PathBuf::from("/etc/example"),
                content: b"desired".to_vec(),
                privileged: false,
                restart_hint: None,
                dry_run: false,
            })
            .await
            .unwrap();
        assert_eq!(applied.summary().changed, 1);
        let removed = runtime
            .remove_managed_sys_file(ManagedFileRemoveRequest {
                os_id: "linux".into(),
                item_id: "managed".into(),
                dry_run: false,
            })
            .await
            .unwrap();
        assert_eq!(removed.summary().changed, 1);
        assert!(
            runtime
                .host()
                .read(Path::new("/etc/example"))
                .await
                .is_err()
        );
    }
}
