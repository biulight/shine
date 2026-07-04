use super::{SysDriverKind, SysItem};
use crate::apps::file_ops::{
    InstallOutcome, UninstallOutcome, install_bytes, install_bytes_admin, uninstall_entry,
    uninstall_entry_admin,
};
use crate::apps::{AppEntry, AppInstallStrategy, apply_transforms, hash_content};
use crate::config::{Config, full_expand_with_home};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

const RECEIPT_VERSION: u32 = 1;
const DNS_MARKER_PREFIX: &str = "Managed by shine: split-dns:";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "driver", rename_all = "kebab-case")]
pub(super) enum SystemReceipt {
    Script { version: u32 },
    SplitDns(SplitDnsReceipt),
    ManagedFile(ManagedFileReceipt),
}

impl SystemReceipt {
    pub(super) fn script() -> Self {
        Self::Script {
            version: RECEIPT_VERSION,
        }
    }

    pub(super) fn driver(&self) -> SysDriverKind {
        match self {
            Self::Script { .. } => SysDriverKind::Script,
            Self::SplitDns(_) => SysDriverKind::SplitDns,
            Self::ManagedFile(_) => SysDriverKind::ManagedFile,
        }
    }

    pub(super) fn requires_admin(&self) -> bool {
        match self {
            Self::Script { .. } => false,
            Self::SplitDns(_) => true,
            Self::ManagedFile(receipt) => receipt.privileged,
        }
    }

    fn ensure_supported(&self) -> Result<()> {
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
pub(super) struct SplitDnsReceipt {
    version: u32,
    os_id: String,
    item_id: String,
    domain: String,
    servers: Vec<String>,
    resource: String,
    content_hash: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct ManagedFileReceipt {
    version: u32,
    destination: PathBuf,
    backup: Option<PathBuf>,
    content_hash: u64,
    privileged: bool,
    restart_hint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ResourcePlan {
    pub description: String,
    pub requires_admin: bool,
    pub restart_hint: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct ResourceOutcome {
    pub changed: bool,
    pub detail: String,
    pub receipt: Option<SystemReceipt>,
    pub restart_hint: Option<String>,
}

pub(super) struct DriverContext<'a> {
    pub config: &'a Config,
    pub os_id: &'a str,
    pub item: &'a SysItem,
    pub preset_root: &'a Path,
    pub env: &'a BTreeMap<String, String>,
    pub dry_run: bool,
}

pub(super) trait SystemDriver {
    fn plan(&self, context: &DriverContext<'_>, removing: bool) -> Result<ResourcePlan>;
    async fn apply(
        &self,
        context: &DriverContext<'_>,
        previous: Option<&SystemReceipt>,
    ) -> Result<ResourceOutcome>;
    async fn remove(
        &self,
        context: Option<&DriverContext<'_>>,
        receipt: &SystemReceipt,
        dry_run: bool,
    ) -> Result<ResourceOutcome>;
}

pub(super) struct BuiltinDriver {
    kind: SysDriverKind,
}

impl BuiltinDriver {
    pub(super) fn new(kind: SysDriverKind) -> Self {
        Self { kind }
    }
}

impl SystemDriver for BuiltinDriver {
    fn plan(&self, context: &DriverContext<'_>, removing: bool) -> Result<ResourcePlan> {
        match self.kind {
            SysDriverKind::SplitDns => {
                let domain = config_env(context, "domain_env")?;
                let action = if removing { "remove" } else { "converge" };
                Ok(ResourcePlan {
                    description: format!("{action} split DNS for {domain}"),
                    requires_admin: context.item.requires_admin,
                    restart_hint: None,
                })
            }
            SysDriverKind::ManagedFile => {
                let target = config_string(&context.item.config, "target")?;
                Ok(ResourcePlan {
                    description: format!(
                        "{} managed file {target}",
                        if removing { "remove" } else { "converge" }
                    ),
                    requires_admin: context.item.requires_admin,
                    restart_hint: optional_config_string(&context.item.config, "restart_hint"),
                })
            }
            SysDriverKind::Script => bail!("script is not a built-in system resource driver"),
        }
    }

    async fn apply(
        &self,
        context: &DriverContext<'_>,
        previous: Option<&SystemReceipt>,
    ) -> Result<ResourceOutcome> {
        if let Some(previous) = previous {
            previous.ensure_supported()?;
        }
        match self.kind {
            SysDriverKind::SplitDns => apply_split_dns(context, previous).await,
            SysDriverKind::ManagedFile => apply_managed_file(context, previous).await,
            SysDriverKind::Script => bail!("script is not a built-in system resource driver"),
        }
    }

    async fn remove(
        &self,
        context: Option<&DriverContext<'_>>,
        receipt: &SystemReceipt,
        dry_run: bool,
    ) -> Result<ResourceOutcome> {
        receipt.ensure_supported()?;
        match (self.kind, receipt) {
            (SysDriverKind::SplitDns, SystemReceipt::SplitDns(receipt)) => {
                remove_split_dns(receipt, dry_run).await
            }
            (SysDriverKind::ManagedFile, SystemReceipt::ManagedFile(receipt)) => {
                remove_managed_file(receipt, dry_run).await
            }
            (kind, receipt) => bail!(
                "receipt driver mismatch: requested {kind:?}, found {:?}",
                receipt.driver()
            ),
        }
        .with_context(|| {
            context
                .map(|context| format!("processing sys item `{}`", context.item.id))
                .unwrap_or_else(|| "processing recorded system resource".to_string())
        })
    }
}

fn config_string(config: &toml::Table, key: &str) -> Result<String> {
    config
        .get(key)
        .and_then(toml::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .with_context(|| format!("driver config requires non-empty `{key}`"))
}

fn optional_config_string(config: &toml::Table, key: &str) -> Option<String> {
    config
        .get(key)
        .and_then(toml::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn config_env(context: &DriverContext<'_>, config_key: &str) -> Result<String> {
    let env_key = config_string(&context.item.config, config_key)?;
    context
        .env
        .get(&env_key)
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .with_context(|| format!("missing environment variable `{env_key}`"))
}

fn normalize_domain(domain: &str) -> Result<String> {
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    let valid = !domain.is_empty()
        && domain.len() <= 253
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .chars()
                    .next()
                    .is_some_and(|character| character.is_ascii_alphanumeric())
                && label
                    .chars()
                    .last()
                    .is_some_and(|character| character.is_ascii_alphanumeric())
                && label
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        });
    if !valid {
        bail!("invalid private DNS domain `{domain}`");
    }
    Ok(domain)
}

fn normalize_servers(servers: &str) -> Result<Vec<String>> {
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

fn split_dns_desired(context: &DriverContext<'_>) -> Result<SplitDnsReceipt> {
    let domain = normalize_domain(&config_env(context, "domain_env")?)?;
    let servers = normalize_servers(&config_env(context, "servers_env")?)?;
    let resource = match context.os_id {
        "macos" => format!("/etc/resolver/{domain}"),
        "ubuntu" => format!(
            "/etc/systemd/resolved.conf.d/shine-split-dns-{}.conf",
            context.item.id
        ),
        "windows" => format!(".{domain}"),
        other => bail!("split-dns is unsupported on `{other}`"),
    };
    Ok(SplitDnsReceipt {
        version: RECEIPT_VERSION,
        os_id: context.os_id.to_string(),
        item_id: context.item.id.clone(),
        domain,
        servers,
        resource,
        content_hash: None,
    })
}

fn split_dns_marker(item_id: &str) -> String {
    format!("{DNS_MARKER_PREFIX}{item_id}")
}

fn split_dns_file_content(receipt: &SplitDnsReceipt) -> Vec<u8> {
    let marker = split_dns_marker(&receipt.item_id);
    let content = if receipt.os_id == "macos" {
        let servers = receipt
            .servers
            .iter()
            .map(|server| format!("nameserver {server}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("# {marker}\n{servers}\n")
    } else {
        format!(
            "# {marker}\n[Resolve]\nDNS={}\nDomains=~{}\n",
            receipt.servers.join(" "),
            receipt.domain
        )
    };
    content.into_bytes()
}

async fn apply_split_dns(
    context: &DriverContext<'_>,
    previous: Option<&SystemReceipt>,
) -> Result<ResourceOutcome> {
    let mut desired = split_dns_desired(context)?;
    if context.dry_run {
        return Ok(ResourceOutcome {
            changed: true,
            detail: format!("{} -> {}", desired.domain, desired.servers.join(", ")),
            receipt: Some(SystemReceipt::SplitDns(desired)),
            restart_hint: None,
        });
    }

    let previous_to_remove = match previous {
        Some(SystemReceipt::SplitDns(previous))
            if previous.resource != desired.resource || previous.os_id != desired.os_id =>
        {
            Some(previous)
        }
        _ => None,
    };

    let changed = if desired.os_id == "windows" {
        apply_windows_split_dns(&desired).await?;
        true
    } else {
        let destination = PathBuf::from(&desired.resource);
        let content = split_dns_file_content(&desired);
        if destination.exists() {
            let current = tokio::fs::read(&destination)
                .await
                .with_context(|| format!("reading {}", destination.display()))?;
            let marker = format!("# {}", split_dns_marker(&desired.item_id));
            if !String::from_utf8_lossy(&current)
                .lines()
                .any(|line| line == marker)
            {
                bail!(
                    "split DNS destination {} exists but is not owned by shine",
                    destination.display()
                );
            }
            if current == content {
                if desired.os_id == "ubuntu"
                    && !matches!(previous, Some(SystemReceipt::SplitDns(_)))
                {
                    restart_systemd_resolved().await?;
                }
                if let Some(previous) = previous_to_remove {
                    remove_split_dns(previous, false).await?;
                }
                desired.content_hash = Some(hash_content(&content));
                return Ok(ResourceOutcome {
                    changed: false,
                    detail: desired.domain.clone(),
                    receipt: Some(SystemReceipt::SplitDns(desired)),
                    restart_hint: None,
                });
            }
        }
        install_bytes_admin(&content, &destination, destination.exists(), false, true).await?;
        desired.content_hash = Some(hash_content(&content));
        if desired.os_id == "ubuntu" {
            restart_systemd_resolved().await?;
        }
        if let Some(previous) = previous_to_remove {
            remove_split_dns(previous, false).await?;
        }
        true
    };

    Ok(ResourceOutcome {
        changed,
        detail: format!("{} -> {}", desired.domain, desired.servers.join(", ")),
        receipt: Some(SystemReceipt::SplitDns(desired)),
        restart_hint: None,
    })
}

async fn remove_split_dns(receipt: &SplitDnsReceipt, dry_run: bool) -> Result<ResourceOutcome> {
    if dry_run {
        return Ok(ResourceOutcome {
            changed: true,
            detail: format!("remove split DNS for {}", receipt.domain),
            receipt: None,
            restart_hint: None,
        });
    }
    let changed = if receipt.os_id == "windows" {
        remove_windows_split_dns(receipt).await?
    } else {
        let destination = PathBuf::from(&receipt.resource);
        if !destination.exists() {
            false
        } else {
            let current = tokio::fs::read(&destination)
                .await
                .with_context(|| format!("reading {}", destination.display()))?;
            let marker = format!("# {}", split_dns_marker(&receipt.item_id));
            if !String::from_utf8_lossy(&current)
                .lines()
                .any(|line| line == marker)
            {
                bail!(
                    "refusing to remove non-shine split DNS resource {}",
                    destination.display()
                );
            }
            let entry = AppEntry {
                source: format!("sys/{}/split-dns", receipt.os_id),
                destination: destination.clone(),
                backup: None,
                content_hash: hash_content(&current),
                install_strategy: AppInstallStrategy::Copy,
                uses_env: false,
            };
            let outcome = uninstall_entry_admin(&entry, false, false).await?;
            if receipt.os_id == "ubuntu" {
                restart_systemd_resolved().await?;
            }
            !matches!(outcome, UninstallOutcome::NotFound)
        }
    };
    Ok(ResourceOutcome {
        changed,
        detail: format!("split DNS for {} removed", receipt.domain),
        receipt: None,
        restart_hint: None,
    })
}

async fn restart_systemd_resolved() -> Result<()> {
    let mut command = if std::env::var("USER").is_ok_and(|user| user == "root") {
        tokio::process::Command::new("systemctl")
    } else {
        let mut command = tokio::process::Command::new("sudo");
        command.args(["-n", "systemctl"]);
        command
    };
    let status = command
        .args(["restart", "systemd-resolved"])
        .status()
        .await
        .context("restarting systemd-resolved")?;
    if !status.success() {
        bail!("failed to restart systemd-resolved");
    }
    Ok(())
}

async fn apply_windows_split_dns(receipt: &SplitDnsReceipt) -> Result<()> {
    let comment = split_dns_marker(&receipt.item_id);
    let servers = receipt
        .servers
        .iter()
        .map(|server| format!("'{}'", ps_quote(server)))
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        "$rules=@(Get-DnsClientNrptRule | Where-Object {{$_.Comment -eq '{comment}'}}); \
         foreach($rule in $rules){{Remove-DnsClientNrptRule -Name $rule.Name -Force}}; \
         Add-DnsClientNrptRule -Namespace '{namespace}' -NameServers @({servers}) -Comment '{comment}' | Out-Null",
        comment = ps_quote(&comment),
        namespace = ps_quote(&receipt.resource),
    );
    run_elevated_powershell(&script).await
}

async fn remove_windows_split_dns(receipt: &SplitDnsReceipt) -> Result<bool> {
    let comment = split_dns_marker(&receipt.item_id);
    let script = format!(
        "$rules=@(Get-DnsClientNrptRule | Where-Object {{$_.Comment -eq '{comment}'}}); \
         foreach($rule in $rules){{Remove-DnsClientNrptRule -Name $rule.Name -Force}}",
        comment = ps_quote(&comment),
    );
    run_elevated_powershell(&script).await?;
    Ok(true)
}

fn ps_quote(value: &str) -> String {
    value.replace('\'', "''")
}

async fn run_elevated_powershell(script: &str) -> Result<()> {
    let id = uuid::Uuid::new_v4();
    let script_path = std::env::temp_dir().join(format!("shine-system-{id}.ps1"));
    let result_path = std::env::temp_dir().join(format!("shine-system-{id}.result"));
    let body = format!(
        "$ErrorActionPreference='Stop'\ntry {{\n{script}\nSet-Content -LiteralPath '{result}' -Value 'ok'\nexit 0\n}} catch {{\nSet-Content -LiteralPath '{result}' -Value $_.Exception.Message\nexit 1\n}}\n",
        result = ps_quote(&result_path.display().to_string())
    );
    tokio::fs::write(&script_path, body)
        .await
        .with_context(|| format!("writing {}", script_path.display()))?;
    let arguments = format!(
        "@('-NoProfile','-ExecutionPolicy','Bypass','-File','\"{}\"')",
        ps_quote(&script_path.display().to_string())
    );
    let wrapper = format!(
        "$p=Start-Process powershell.exe -Verb RunAs -Wait -PassThru -ArgumentList {arguments}; exit $p.ExitCode"
    );
    let status = tokio::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", &wrapper])
        .stdin(Stdio::inherit())
        .status()
        .await
        .context("running elevated PowerShell")?;
    let result = tokio::fs::read_to_string(&result_path)
        .await
        .unwrap_or_else(|_| "elevated process did not return a result".to_string());
    let _ = tokio::fs::remove_file(&script_path).await;
    let _ = tokio::fs::remove_file(&result_path).await;
    if !status.success() {
        bail!("elevated PowerShell operation failed: {}", result.trim());
    }
    Ok(())
}

fn managed_file_desired(context: &DriverContext<'_>) -> Result<(PathBuf, Vec<u8>, Option<String>)> {
    let source = config_string(&context.item.config, "source")?;
    let target = config_string(&context.item.config, "target")?;
    let source = PathBuf::from(source);
    if source.is_absolute()
        || source
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("managed-file source must stay within its preset directory");
    }
    let source = context.preset_root.join(source);
    let canonical_root = std::fs::canonicalize(context.preset_root)
        .with_context(|| format!("reading preset root {}", context.preset_root.display()))?;
    let canonical_source =
        std::fs::canonicalize(&source).with_context(|| format!("reading {}", source.display()))?;
    if !canonical_source.starts_with(&canonical_root) {
        bail!("managed-file source must stay within its preset directory");
    }
    let target = PathBuf::from(
        full_expand_with_home(&target, &context.config.home_dir)
            .with_context(|| format!("expanding managed-file target `{target}`"))?,
    );
    if !target.is_absolute() {
        bail!("managed-file target must resolve to an absolute path");
    }
    let raw = std::fs::read(&canonical_source)
        .with_context(|| format!("reading {}", canonical_source.display()))?;
    let transforms = context
        .item
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
    let content = if transforms.is_empty() {
        raw
    } else {
        apply_transforms(&transforms, &raw, context.env)?
    };
    Ok((
        target,
        content,
        optional_config_string(&context.item.config, "restart_hint"),
    ))
}

async fn apply_managed_file(
    context: &DriverContext<'_>,
    previous: Option<&SystemReceipt>,
) -> Result<ResourceOutcome> {
    let (destination, content, restart_hint) = managed_file_desired(context)?;
    let previous = match previous {
        Some(SystemReceipt::ManagedFile(receipt)) => Some(receipt),
        Some(other) => bail!("managed-file received {:?} receipt", other.driver()),
        None => None,
    };
    if context.dry_run {
        return Ok(ResourceOutcome {
            changed: true,
            detail: destination.display().to_string(),
            receipt: None,
            restart_hint,
        });
    }
    if let Some(previous) = previous {
        if previous.destination != destination {
            remove_managed_file(previous, false).await?;
        } else if destination.exists() {
            let current = tokio::fs::read(&destination).await?;
            if hash_content(&current) != previous.content_hash {
                bail!(
                    "managed file {} was modified; keeping user content",
                    destination.display()
                );
            }
            if current == content {
                return Ok(ResourceOutcome {
                    changed: false,
                    detail: destination.display().to_string(),
                    receipt: Some(SystemReceipt::ManagedFile(previous.clone())),
                    restart_hint: None,
                });
            }
        }
    }
    let is_managed = previous.is_some_and(|receipt| receipt.destination == destination);
    let outcome = if context.item.requires_admin {
        install_bytes_admin(&content, &destination, is_managed, false, true).await?
    } else {
        install_bytes(&content, &destination, is_managed, false, true).await?
    };
    let changed = !matches!(outcome, InstallOutcome::AlreadyManaged);
    let backup = match outcome {
        InstallOutcome::BackedUpAndInstalled { backup, .. } => Some(backup),
        _ => previous.and_then(|receipt| receipt.backup.clone()),
    };
    let receipt = ManagedFileReceipt {
        version: RECEIPT_VERSION,
        destination: destination.clone(),
        backup,
        content_hash: hash_content(&content),
        privileged: context.item.requires_admin,
        restart_hint: restart_hint.clone(),
    };
    Ok(ResourceOutcome {
        changed,
        detail: destination.display().to_string(),
        receipt: Some(SystemReceipt::ManagedFile(receipt)),
        restart_hint,
    })
}

async fn remove_managed_file(
    receipt: &ManagedFileReceipt,
    dry_run: bool,
) -> Result<ResourceOutcome> {
    let entry = AppEntry {
        source: "sys/managed-file".to_string(),
        destination: receipt.destination.clone(),
        backup: receipt.backup.clone(),
        content_hash: receipt.content_hash,
        install_strategy: AppInstallStrategy::Copy,
        uses_env: false,
    };
    let outcome = if receipt.privileged {
        uninstall_entry_admin(&entry, dry_run, false).await?
    } else {
        uninstall_entry(&entry, dry_run, false).await?
    };
    if matches!(outcome, UninstallOutcome::UserModified) {
        bail!(
            "managed file {} was modified; keeping user content",
            receipt.destination.display()
        );
    }
    Ok(ResourceOutcome {
        changed: !matches!(outcome, UninstallOutcome::NotFound),
        detail: receipt.destination.display().to_string(),
        receipt: None,
        restart_hint: receipt.restart_hint.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::{SysDriverKind, SysItem, SysItemMode};

    async fn make_temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shine-resource-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        dir
    }

    fn managed_file_item(source: &str, target: &Path) -> SysItem {
        let mut config = toml::Table::new();
        config.insert(
            "source".to_string(),
            toml::Value::String(source.to_string()),
        );
        config.insert(
            "target".to_string(),
            toml::Value::String(target.display().to_string()),
        );
        config.insert(
            "restart_hint".to_string(),
            toml::Value::String("Restart sample".to_string()),
        );
        SysItem {
            id: "sample-file".to_string(),
            label: "Sample file".to_string(),
            description: String::new(),
            default: false,
            mode: SysItemMode::Managed,
            requires_admin: false,
            required_env: Vec::new(),
            driver: SysDriverKind::ManagedFile,
            config,
        }
    }

    #[tokio::test]
    async fn managed_file_backs_up_converges_and_restores() {
        let dir = make_temp_dir().await;
        let source = dir.join("source.txt");
        let destination = dir.join("destination.txt");
        tokio::fs::write(&source, "desired").await.unwrap();
        tokio::fs::write(&destination, "original").await.unwrap();
        let config = Config::new_for_test(&dir);
        let item = managed_file_item("source.txt", &destination);
        let env = BTreeMap::new();
        let context = DriverContext {
            config: &config,
            os_id: "fakeos",
            item: &item,
            preset_root: &dir,
            env: &env,
            dry_run: false,
        };
        let driver = BuiltinDriver::new(SysDriverKind::ManagedFile);

        let installed = driver.apply(&context, None).await.unwrap();
        assert!(installed.changed);
        assert_eq!(
            tokio::fs::read_to_string(&destination).await.unwrap(),
            "desired"
        );
        assert_eq!(installed.restart_hint.as_deref(), Some("Restart sample"));
        let receipt = installed.receipt.unwrap();

        let unchanged = driver.apply(&context, Some(&receipt)).await.unwrap();
        assert!(!unchanged.changed);
        assert!(unchanged.restart_hint.is_none());

        tokio::fs::write(&destination, "user edit").await.unwrap();
        let error = driver.apply(&context, Some(&receipt)).await.unwrap_err();
        assert!(error.to_string().contains("modified"));

        tokio::fs::write(&destination, "desired").await.unwrap();
        let removed = driver
            .remove(Some(&context), &receipt, false)
            .await
            .unwrap();
        assert!(removed.changed);
        assert_eq!(
            tokio::fs::read_to_string(&destination).await.unwrap(),
            "original"
        );

        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[test]
    fn receipt_roundtrips_with_version_and_driver() {
        #[derive(Serialize, Deserialize)]
        struct Wrapper {
            receipt: SystemReceipt,
        }
        let receipt = SystemReceipt::ManagedFile(ManagedFileReceipt {
            version: RECEIPT_VERSION,
            destination: PathBuf::from("/tmp/example"),
            backup: None,
            content_hash: 42,
            privileged: false,
            restart_hint: None,
        });
        let encoded = toml::to_string(&Wrapper {
            receipt: receipt.clone(),
        })
        .unwrap();
        assert!(encoded.contains("driver = \"managed-file\""));
        assert!(encoded.contains("version = 1"));
        let decoded: Wrapper = toml::from_str(&encoded).unwrap();
        assert_eq!(decoded.receipt, receipt);
    }

    #[test]
    fn split_dns_plans_platform_owned_resources() {
        let dir = std::env::temp_dir();
        let config = Config::new_for_test(&dir);
        let mut driver_config = toml::Table::new();
        driver_config.insert(
            "domain_env".to_string(),
            toml::Value::String("DOMAIN".to_string()),
        );
        driver_config.insert(
            "servers_env".to_string(),
            toml::Value::String("SERVERS".to_string()),
        );
        let item = SysItem {
            id: "private-dns".to_string(),
            label: "Private DNS".to_string(),
            description: String::new(),
            default: false,
            mode: SysItemMode::Managed,
            requires_admin: true,
            required_env: vec!["DOMAIN".to_string(), "SERVERS".to_string()],
            driver: SysDriverKind::SplitDns,
            config: driver_config,
        };
        let env = BTreeMap::from([
            ("DOMAIN".to_string(), "Home.Example.COM.".to_string()),
            ("SERVERS".to_string(), "10.0.0.2, 10.0.0.3".to_string()),
        ]);
        for (os_id, expected) in [
            ("macos", "/etc/resolver/home.example.com"),
            (
                "ubuntu",
                "/etc/systemd/resolved.conf.d/shine-split-dns-private-dns.conf",
            ),
            ("windows", ".home.example.com"),
        ] {
            let context = DriverContext {
                config: &config,
                os_id,
                item: &item,
                preset_root: &dir,
                env: &env,
                dry_run: true,
            };
            let receipt = split_dns_desired(&context).unwrap();
            assert_eq!(receipt.resource, expected);
            assert_eq!(receipt.servers, ["10.0.0.2", "10.0.0.3"]);
            if os_id != "windows" {
                let content = String::from_utf8(split_dns_file_content(&receipt)).unwrap();
                assert!(content.contains("Managed by shine: split-dns:private-dns"));
            }
        }
    }

    #[test]
    fn split_dns_rejects_invalid_domains_and_servers() {
        for domain in ["", ".example.com", "bad..example", "-bad.example"] {
            assert!(normalize_domain(domain).is_err(), "accepted {domain:?}");
        }
        assert!(normalize_servers("not-an-ip").is_err());
        assert!(normalize_servers(" ").is_err());
    }
}
